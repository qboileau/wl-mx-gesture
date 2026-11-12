//! Output adapter: draws the action wheel on a wlr-layer-shell overlay surface, from its own thread.

mod icons;
mod layout;
mod render;
mod text;
mod wayland;

use std::io;
use std::sync::mpsc;
use std::thread;

use log::warn;
use smithay_client_toolkit::reexports::calloop;
use smithay_client_toolkit::reexports::calloop::channel;
use smithay_client_toolkit::reexports::client::ConnectError;
use smithay_client_toolkit::reexports::client::globals::{BindError, GlobalError};
use wlmg_core::profile::Wheel;
use wlmg_core::style::Style;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot connect to the Wayland display: {0}")]
    Connect(#[from] ConnectError),
    #[error("cannot list Wayland globals: {0}")]
    Globals(#[from] GlobalError),
    #[error("the compositor lacks {name}, required to show the wheel: {source}")]
    MissingProtocol { name: &'static str, source: BindError },
    #[error("event loop: {0}")]
    EventLoop(#[from] calloop::Error),
    #[error("cannot start the overlay thread: {0}")]
    Thread(#[from] io::Error),
    #[error("the overlay thread stopped while starting")]
    Stopped,
    #[error("font `{0}` from `style.font` is not installed")]
    FontNotFound(String),
    #[error("no usable font found on the system")]
    NoFont,
}

pub(crate) enum Command {
    /// Loads the fonts and icons a wheel needs, so that opening it is instant.
    Prepare(Wheel),
    Open(Wheel),
    Hover(Option<usize>),
    Close,
}

pub struct Overlay {
    commands: channel::Sender<Command>,
}

impl Overlay {
    pub fn spawn(style: Style) -> Result<Self, Error> {
        let (commands, receiver) = channel::channel();
        let (ready, started) = mpsc::sync_channel(1);
        thread::Builder::new().name("overlay".into()).spawn(move || wayland::run(style, receiver, ready))?;
        started.recv().map_err(|_| Error::Stopped)??;
        Ok(Self { commands })
    }

    pub fn prepare(&self, wheel: &Wheel) {
        self.send(Command::Prepare(wheel.clone()));
    }

    pub fn open(&self, wheel: &Wheel) {
        self.send(Command::Open(wheel.clone()));
    }

    pub fn hover(&self, slot: Option<usize>) {
        self.send(Command::Hover(slot));
    }

    pub fn close(&self) {
        self.send(Command::Close);
    }

    fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            warn!("the wheel overlay has stopped");
        }
    }
}
