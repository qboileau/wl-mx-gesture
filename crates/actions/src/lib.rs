mod hyprland;

use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

use log::warn;
use wlmg_core::action::Action;

use hyprland::Hyprland;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot spawn `{command}`: {source}")]
    Spawn { command: String, source: io::Error },
    #[error("XDG_RUNTIME_DIR is not set")]
    NoRuntimeDir,
    #[error("no running Hyprland instance found in {0}")]
    NoHyprland(PathBuf),
    #[error("Hyprland IPC: {0}")]
    HyprlandIo(#[from] io::Error),
    #[error("Hyprland rejected `dispatch {dispatch}`: {reply}")]
    HyprlandRejected { dispatch: String, reply: String },
}

#[derive(Default)]
pub struct Runner {
    hyprland: Option<Hyprland>,
}

impl Runner {
    pub fn run(&mut self, action: &Action) -> Result<(), Error> {
        match action {
            Action::Exec(command) => exec(command),
            Action::Hyprland(args) => self.hyprland()?.dispatch(args),
            Action::Compositor(action) => self.hyprland()?.perform(action),
        }
    }

    fn hyprland(&mut self) -> Result<&Hyprland, Error> {
        let hyprland = match self.hyprland.take() {
            Some(hyprland) if hyprland.is_current() => hyprland,
            _ => Hyprland::connect()?,
        };
        Ok(self.hyprland.insert(hyprland))
    }
}

fn exec(command: &str) -> Result<(), Error> {
    let mut child = Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::null())
        .spawn()
        .map_err(|source| Error::Spawn { command: command.to_owned(), source })?;
    let command = command.to_owned();
    // Waiting reaps the child so it does not linger as a zombie.
    thread::spawn(move || match child.wait() {
        Ok(status) if !status.success() => warn!("`{command}` exited with {status}"),
        Ok(_) => {}
        Err(error) => warn!("`{command}`: {error}"),
    });
    Ok(())
}
