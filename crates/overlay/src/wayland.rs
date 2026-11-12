use std::sync::mpsc::SyncSender;
use std::time::Duration;

use log::{error, warn};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::channel::{self, Channel};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{EventLoop, LoopHandle, RegistrationToken};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_dispatch2, delegate_registry, registry_handlers};
use wlmg_core::geometry::Vector;
use wlmg_core::profile::Wheel;
use wlmg_core::style::Style;

use crate::layout::{blit, wheel_centre};
use crate::render::Renderer;
use crate::{Command, Error};

const NAMESPACE: &str = "wl-mx-gesture";
const BYTES_PER_PIXEL: i32 = 4;
/// The pool grows on demand to the output size.
const INITIAL_POOL_BYTES: usize = 4096;
/// The cursor position comes from `wl_pointer.enter` once the overlay maps under it; compositors that do not
/// send it for a still cursor get the wheel centred on the output instead.
const CURSOR_WAIT: Duration = Duration::from_millis(80);

pub(crate) fn run(style: Style, commands: Channel<Command>, ready: SyncSender<Result<(), Error>>) {
    let (mut event_loop, mut state) = match setup(style, commands) {
        Ok(started) => started,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    if let Err(error) = event_loop.run(None, &mut state, |_| {}) {
        error!("wheel overlay stopped: {error}");
    }
}

fn setup(style: Style, commands: Channel<Command>) -> Result<(EventLoop<'static, State>, State), Error> {
    let renderer = Renderer::new(style)?;
    let connection = Connection::connect_to_env()?;
    let (globals, queue) = registry_queue_init(&connection)?;
    let qh = queue.handle();
    let event_loop = EventLoop::try_new()?;
    let handle = event_loop.handle();
    WaylandSource::new(connection, queue).insert(handle.clone()).map_err(|error| error.error)?;
    handle
        .insert_source(commands, |event, _, state: &mut State| match event {
            channel::Event::Msg(command) => state.command(command),
            channel::Event::Closed => state.close(),
        })
        .map_err(|error| error.error)?;
    let missing = |name| move |source| Error::MissingProtocol { name, source };
    let state = State {
        compositor: CompositorState::bind(&globals, &qh).map_err(missing("wl_compositor"))?,
        layer_shell: LayerShell::bind(&globals, &qh).map_err(missing("zwlr_layer_shell_v1"))?,
        shm: Shm::bind(&globals, &qh).map_err(missing("wl_shm"))?,
        registry: RegistryState::new(&globals),
        seat: SeatState::new(&globals, &qh),
        output: OutputState::new(&globals, &qh),
        pointer: None,
        renderer,
        session: None,
        handle,
        qh,
    };
    Ok((event_loop, state))
}

struct State {
    registry: RegistryState,
    seat: SeatState,
    output: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pointer: Option<wl_pointer::WlPointer>,
    renderer: Renderer,
    session: Option<Session>,
    handle: LoopHandle<'static, State>,
    qh: QueueHandle<State>,
}

/// One wheel on screen. Buffers stay transparent outside the wheel without being cleared because the pool
/// starts zeroed and every frame draws the wheel at the same place and scale; a scale change gets a new pool.
struct Session {
    wheel: Wheel,
    hovered: Option<usize>,
    layer: LayerSurface,
    pool: SlotPool,
    size: Option<(u32, u32)>,
    scale: i32,
    centre: Option<Vector>,
    cursor_wait: Option<RegistrationToken>,
}

impl State {
    fn command(&mut self, command: Command) {
        match command {
            Command::Prepare(wheel) => {
                self.renderer.render(&wheel, None, 1.0);
            }
            Command::Open(wheel) => self.open(wheel),
            Command::Hover(slot) => {
                if let Some(session) = &mut self.session {
                    session.hovered = slot;
                    self.draw();
                }
            }
            Command::Close => self.close(),
        }
    }

    fn open(&mut self, wheel: Wheel) {
        self.close();
        let pool = match SlotPool::new(INITIAL_POOL_BYTES, &self.shm) {
            Ok(pool) => pool,
            Err(error) => return warn!("cannot allocate the wheel buffer: {error}"),
        };
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Overlay, Some(NAMESPACE), None);
        layer.set_anchor(Anchor::all());
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        self.session =
            Some(Session { wheel, hovered: None, layer, pool, size: None, scale: 1, centre: None, cursor_wait: None });
    }

    fn close(&mut self) {
        if let Some(token) = self.session.take().and_then(|session| session.cursor_wait) {
            self.handle.remove(token);
        }
    }

    fn session_of(&mut self, surface: &wl_surface::WlSurface) -> Option<&mut Session> {
        self.session.as_mut().filter(|session| session.layer.wl_surface() == surface)
    }

    fn place(&mut self, cursor: Option<Vector>) {
        let radius = self.renderer.radius();
        let Some(session) = &mut self.session else { return };
        let (Some((width, height)), None) = (session.size, session.centre) else { return };
        let output = Vector::new(width as f32, height as f32);
        session.centre = Some(wheel_centre(cursor.unwrap_or(output * 0.5), output, radius));
        if let Some(token) = session.cursor_wait.take() {
            self.handle.remove(token);
        }
        self.draw();
    }

    fn draw(&mut self) {
        let Self { session: Some(session), renderer, .. } = self else { return };
        let Some((width, height)) = session.size else { return };
        let (width, height) = (width as i32 * session.scale, height as i32 * session.scale);
        let (buffer, canvas) =
            match session.pool.create_buffer(width, height, width * BYTES_PER_PIXEL, wl_shm::Format::Argb8888) {
                Ok(buffer) => buffer,
                Err(error) => return warn!("cannot allocate the wheel buffer: {error}"),
            };
        let surface = session.layer.wl_surface();
        let scale = session.scale as f32;
        if let Some(centre) = session.centre
            && let Some(pixmap) = renderer.render(&session.wheel, session.hovered, scale)
        {
            let half = pixmap.width() as i32 / 2;
            let origin = ((centre.x * scale) as i32 - half, (centre.y * scale) as i32 - half);
            if let Some(region) = blit(canvas, width, &pixmap, origin) {
                surface.damage_buffer(region.x, region.y, region.width, region.height);
            }
        }
        if let Err(error) = buffer.attach_to(surface) {
            return warn!("cannot attach the wheel buffer: {error}");
        }
        session.layer.commit();
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.session_of(layer.wl_surface()).is_some() {
            self.close();
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(session) = self.session_of(layer.wl_surface()) else { return };
        let first = session.size.replace(configure.new_size).is_none();
        if first {
            let wait = Timer::from_duration(CURSOR_WAIT);
            let token = self.handle.insert_source(wait, |_, _, state| {
                if let Some(session) = &mut state.session {
                    session.cursor_wait = None;
                }
                state.place(None);
                TimeoutAction::Drop
            });
            match (token, &mut self.session) {
                (Ok(token), Some(session)) => session.cursor_wait = Some(token),
                (Err(error), _) => warn!("cannot wait for the cursor position: {}", error.error),
                (Ok(_), None) => {}
            }
        }
        self.draw();
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        let Self { session: Some(session), shm, .. } = self else { return };
        if session.layer.wl_surface() != surface || session.scale == factor {
            return;
        }
        match SlotPool::new(INITIAL_POOL_BYTES, shm) {
            Ok(pool) => session.pool = pool,
            Err(error) => return warn!("cannot reallocate the wheel buffer: {error}"),
        }
        session.scale = factor;
        surface.set_buffer_scale(factor);
        self.draw();
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        let entered = events.iter().rev().find(|event| {
            matches!(event.kind, PointerEventKind::Enter { .. })
                && self.session.as_ref().is_some_and(|session| session.layer.wl_surface() == &event.surface)
        });
        if let Some(event) = entered {
            self.place(Some(Vector::new(event.position.0 as f32, event.position.1 as f32)));
        }
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            match self.seat.get_pointer(qh, &seat) {
                Ok(pointer) => self.pointer = Some(pointer),
                Err(error) => warn!("no pointer, the wheel will be centred on the output: {error}"),
            }
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer
            && let Some(pointer) = self.pointer.take()
        {
            pointer.release();
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState, SeatState];
}

delegate_registry!(State);
delegate_dispatch2!(State);
