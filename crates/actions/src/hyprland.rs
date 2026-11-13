//! Hyprland IPC over `$XDG_RUNTIME_DIR/hypr/<instance>/.socket.sock`, for both the legacy dispatcher
//! syntax and the Lua one (`dispatch` evaluates `hl.dispatch(<expr>)`) introduced around 0.55.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use log::{debug, info};
use serde::Deserialize;
use wlmg_core::action::CompositorAction;
use wlmg_core::workspace::{self, Layout, Monitor, Navigation, Side, Workspace};

use crate::Error;

const TIMEOUT: Duration = Duration::from_secs(1);
const OK: &str = "ok";
const SOCKET: &str = ".socket.sock";
const SPECIAL_PREFIX: &str = "special:";
/// Answers `ok` only where requests are evaluated as Lua; legacy versions reply `unknown request`.
const LUA_PROBE: &str = "eval assert(hl and hl.dispatch)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Syntax {
    Legacy,
    Lua,
}

/// A Hyprland instance, its dispatch syntax detected once on first use.
pub(crate) struct Hyprland {
    socket: PathBuf,
    syntax: Syntax,
}

impl Hyprland {
    pub fn connect() -> Result<Self, Error> {
        let socket = socket_path()?;
        let syntax = if request(&socket, LUA_PROBE)? == OK { Syntax::Lua } else { Syntax::Legacy };
        info!("Hyprland {} uses {syntax:?} dispatch syntax", socket.display());
        Ok(Self { socket, syntax })
    }

    /// False once Hyprland restarted, since a new instance gets a new socket.
    pub fn is_current(&self) -> bool {
        socket_path().is_ok_and(|socket| socket == self.socket)
    }

    pub fn perform(&self, action: &CompositorAction, navigation: Navigation) -> Result<(), Error> {
        let adjacent = |side| Ok(workspace::adjacent(&self.layout()?, navigation, side).map(reference));
        match dispatch_args(action, self.syntax, adjacent)? {
            Some(args) => self.dispatch(&args),
            None => {
                debug!("{action:?}: no other workspace on that side");
                Ok(())
            }
        }
    }

    pub fn dispatch(&self, args: &str) -> Result<(), Error> {
        match request(&self.socket, &format!("dispatch {args}"))?.as_str() {
            OK => Ok(()),
            reply => Err(Error::HyprlandRejected { dispatch: args.to_owned(), reply: reply.to_owned() }),
        }
    }

    fn layout(&self) -> Result<Layout, Error> {
        layout(self.query("j/monitors")?, self.query("j/workspaces")?)
    }

    fn query<T: serde::de::DeserializeOwned>(&self, request_name: &'static str) -> Result<T, Error> {
        serde_json::from_str(&request(&self.socket, request_name)?)
            .map_err(|source| Error::HyprlandReply { request: request_name, source })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MonitorReply {
    name: String,
    x: i32,
    y: i32,
    focused: bool,
    active_workspace: WorkspaceRef,
}

#[derive(Deserialize)]
struct WorkspaceRef {
    id: i64,
    name: String,
}

#[derive(Deserialize)]
struct WorkspaceReply {
    id: i64,
    name: String,
    monitor: String,
}

fn layout(monitors: Vec<MonitorReply>, workspaces: Vec<WorkspaceReply>) -> Result<Layout, Error> {
    let mut by_monitor: HashMap<String, Vec<Workspace>> = HashMap::new();
    for WorkspaceReply { id, name, monitor } in workspaces.into_iter().filter(|ws| !ws.name.starts_with(SPECIAL_PREFIX))
    {
        by_monitor.entry(monitor).or_default().push(Workspace { id, name });
    }
    let (focused, others): (Vec<_>, Vec<_>) = monitors
        .into_iter()
        .map(|MonitorReply { name, x, y, focused, active_workspace: WorkspaceRef { id, name: active } }| {
            let workspaces = by_monitor.remove(&name).unwrap_or_default();
            (focused, Monitor { x, y, active: Workspace { id, name: active }, workspaces })
        })
        .partition(|(focused, _)| *focused);
    let focused = focused.into_iter().next().ok_or(Error::NoFocusedMonitor)?.1;
    Ok(Layout { focused, others: others.into_iter().map(|(_, monitor)| monitor).collect() })
}

/// Named workspaces have non-positive ids and are only reachable by name.
fn reference(workspace: &Workspace) -> String {
    if workspace.id > 0 { workspace.id.to_string() } else { format!("name:{}", workspace.name) }
}

/// `None` when an adjacent workspace was asked for and there is none.
fn dispatch_args(
    action: &CompositorAction,
    syntax: Syntax,
    adjacent: impl FnOnce(Side) -> Result<Option<String>, Error>,
) -> Result<Option<String>, Error> {
    let args = match (syntax, action) {
        (_, CompositorAction::AdjacentWorkspace(side)) => {
            return Ok(adjacent(*side)?.map(|workspace| focus_workspace(&workspace, syntax)));
        }
        (_, CompositorAction::MoveToAdjacentWorkspace(side)) => {
            return Ok(adjacent(*side)?.map(|workspace| move_to_workspace(&workspace, syntax)));
        }
        (_, CompositorAction::Workspace(workspace)) => focus_workspace(workspace, syntax),
        (_, CompositorAction::MoveToWorkspace(workspace)) => move_to_workspace(workspace, syntax),
        (Syntax::Legacy, CompositorAction::ToggleSpecial(name)) => format!("togglespecialworkspace {name}"),
        (Syntax::Legacy, CompositorAction::CloseWindow) => "killactive".into(),
        (Syntax::Legacy, CompositorAction::ToggleFloating) => "togglefloating".into(),
        (Syntax::Legacy, CompositorAction::Fullscreen) => "fullscreen 0".into(),
        (Syntax::Legacy, CompositorAction::Maximize) => "fullscreen 1".into(),
        (Syntax::Lua, CompositorAction::ToggleSpecial(name)) => {
            format!("hl.dsp.workspace.toggle_special({})", lua_string(name))
        }
        (Syntax::Lua, CompositorAction::CloseWindow) => "hl.dsp.window.close()".into(),
        (Syntax::Lua, CompositorAction::ToggleFloating) => r#"hl.dsp.window.float({ action = "toggle" })"#.into(),
        (Syntax::Lua, CompositorAction::Fullscreen) => {
            r#"hl.dsp.window.fullscreen({ action = "toggle", mode = "fullscreen" })"#.into()
        }
        (Syntax::Lua, CompositorAction::Maximize) => {
            r#"hl.dsp.window.fullscreen({ action = "toggle", mode = "maximized" })"#.into()
        }
    };
    Ok(Some(args))
}

fn focus_workspace(workspace: &str, syntax: Syntax) -> String {
    match syntax {
        Syntax::Legacy => format!("workspace {workspace}"),
        Syntax::Lua => format!("hl.dsp.focus({{ workspace = {} }})", lua_string(workspace)),
    }
}

fn move_to_workspace(workspace: &str, syntax: Syntax) -> String {
    match syntax {
        Syntax::Legacy => format!("movetoworkspace {workspace}"),
        Syntax::Lua => format!("hl.dsp.window.move({{ workspace = {}, follow = true }})", lua_string(workspace)),
    }
}

fn lua_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

fn request(socket: &Path, message: &str) -> Result<String, Error> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.write_all(message.as_bytes())?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply.trim().to_owned())
}

fn socket_path() -> Result<PathBuf, Error> {
    let hypr_dir = PathBuf::from(env::var_os("XDG_RUNTIME_DIR").ok_or(Error::NoRuntimeDir)?).join("hypr");
    let instance = match env::var_os("HYPRLAND_INSTANCE_SIGNATURE") {
        Some(signature) => hypr_dir.join(signature),
        None => newest_instance(&hypr_dir)?,
    };
    Ok(instance.join(SOCKET))
}

/// Systemd user services often lack HYPRLAND_INSTANCE_SIGNATURE, so pick the latest instance.
fn newest_instance(hypr_dir: &Path) -> Result<PathBuf, Error> {
    let no_instance = || Error::NoHyprland(hypr_dir.to_owned());
    fs::read_dir(hypr_dir)
        .map_err(|_| no_instance())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|instance| instance.join(SOCKET).exists())
        .max_by_key(|instance| fs::metadata(instance).and_then(|meta| meta.modified()).ok())
        .ok_or_else(no_instance)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_with(syntax: Syntax, adjacent: Option<&str>) -> impl Fn(CompositorAction) -> Option<String> {
        move |action| dispatch_args(&action, syntax, |_| Ok(adjacent.map(str::to_owned))).unwrap()
    }

    #[test]
    fn legacy_syntax() {
        let args = |action| args_with(Syntax::Legacy, None)(action).unwrap();
        assert_eq!(args(CompositorAction::Workspace("e+1".into())), "workspace e+1");
        assert_eq!(args(CompositorAction::MoveToWorkspace("3".into())), "movetoworkspace 3");
        assert_eq!(args(CompositorAction::ToggleSpecial("magic".into())), "togglespecialworkspace magic");
        assert_eq!(args(CompositorAction::CloseWindow), "killactive");
        assert_eq!(args(CompositorAction::Maximize), "fullscreen 1");
    }

    #[test]
    fn lua_syntax() {
        let args = |action| args_with(Syntax::Lua, None)(action).unwrap();
        assert_eq!(args(CompositorAction::Workspace("e+1".into())), r#"hl.dsp.focus({ workspace = "e+1" })"#);
        assert_eq!(
            args(CompositorAction::MoveToWorkspace("special:magic".into())),
            r#"hl.dsp.window.move({ workspace = "special:magic", follow = true })"#
        );
        assert_eq!(
            args(CompositorAction::ToggleSpecial("magic".into())),
            r#"hl.dsp.workspace.toggle_special("magic")"#
        );
        assert_eq!(args(CompositorAction::CloseWindow), "hl.dsp.window.close()");
    }

    #[test]
    fn adjacent_workspaces_dispatch_the_resolved_target() {
        let legacy = args_with(Syntax::Legacy, Some("3"));
        assert_eq!(legacy(CompositorAction::AdjacentWorkspace(Side::Right)).as_deref(), Some("workspace 3"));
        assert_eq!(legacy(CompositorAction::MoveToAdjacentWorkspace(Side::Left)).as_deref(), Some("movetoworkspace 3"));
        assert_eq!(
            args_with(Syntax::Lua, Some("3"))(CompositorAction::AdjacentWorkspace(Side::Left)).as_deref(),
            Some(r#"hl.dsp.focus({ workspace = "3" })"#)
        );
        assert_eq!(args_with(Syntax::Legacy, None)(CompositorAction::AdjacentWorkspace(Side::Left)), None);
    }

    #[test]
    fn layout_groups_regular_workspaces_by_monitor() {
        let monitors = serde_json::from_str(
            r#"[{"name":"DP-1","x":0,"y":0,"focused":true,"activeWorkspace":{"id":4,"name":"4"},"width":2560},
                {"name":"DP-2","x":2560,"y":0,"focused":false,"activeWorkspace":{"id":3,"name":"3"}}]"#,
        )
        .unwrap();
        let workspaces = serde_json::from_str(
            r#"[{"id":1,"name":"1","monitor":"DP-1"},{"id":4,"name":"4","monitor":"DP-1"},
                {"id":-98,"name":"special:magic","monitor":"DP-1"},{"id":3,"name":"3","monitor":"DP-2"}]"#,
        )
        .unwrap();
        let layout = layout(monitors, workspaces).unwrap();
        assert_eq!(layout.focused.active.id, 4);
        assert_eq!(layout.focused.workspaces.iter().map(|ws| ws.id).collect::<Vec<_>>(), [1, 4]);
        assert_eq!(layout.others[0].x, 2560);
        assert_eq!(workspace::adjacent(&layout, Navigation::Spatial, Side::Right).map(reference).as_deref(), Some("3"));
    }

    #[test]
    fn named_workspaces_are_referenced_by_name() {
        assert_eq!(reference(&Workspace { id: -1337, name: "web".into() }), "name:web");
    }

    #[test]
    fn lua_strings_cannot_break_out() {
        assert_eq!(lua_string(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(lua_string("a\nb"), r#""a\nb""#);
    }
}
