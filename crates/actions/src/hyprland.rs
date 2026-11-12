//! Hyprland IPC over `$XDG_RUNTIME_DIR/hypr/<instance>/.socket.sock`, for both the legacy dispatcher
//! syntax and the Lua one (`dispatch` evaluates `hl.dispatch(<expr>)`) introduced around 0.55.

use std::env;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use log::info;
use wlmg_core::action::CompositorAction;

use crate::Error;

const TIMEOUT: Duration = Duration::from_secs(1);
const OK: &str = "ok";
const SOCKET: &str = ".socket.sock";
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

    pub fn perform(&self, action: &CompositorAction) -> Result<(), Error> {
        self.dispatch(&dispatch_args(action, self.syntax))
    }

    pub fn dispatch(&self, args: &str) -> Result<(), Error> {
        match request(&self.socket, &format!("dispatch {args}"))?.as_str() {
            OK => Ok(()),
            reply => Err(Error::HyprlandRejected { dispatch: args.to_owned(), reply: reply.to_owned() }),
        }
    }
}

fn dispatch_args(action: &CompositorAction, syntax: Syntax) -> String {
    match (syntax, action) {
        (Syntax::Legacy, CompositorAction::Workspace(workspace)) => format!("workspace {workspace}"),
        (Syntax::Legacy, CompositorAction::MoveToWorkspace(workspace)) => format!("movetoworkspace {workspace}"),
        (Syntax::Legacy, CompositorAction::ToggleSpecial(name)) => format!("togglespecialworkspace {name}"),
        (Syntax::Legacy, CompositorAction::CloseWindow) => "killactive".into(),
        (Syntax::Legacy, CompositorAction::ToggleFloating) => "togglefloating".into(),
        (Syntax::Legacy, CompositorAction::Fullscreen) => "fullscreen 0".into(),
        (Syntax::Legacy, CompositorAction::Maximize) => "fullscreen 1".into(),
        (Syntax::Lua, CompositorAction::Workspace(workspace)) => {
            format!("hl.dsp.focus({{ workspace = {} }})", lua_string(workspace))
        }
        (Syntax::Lua, CompositorAction::MoveToWorkspace(workspace)) => {
            format!("hl.dsp.window.move({{ workspace = {}, follow = true }})", lua_string(workspace))
        }
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

    #[test]
    fn legacy_syntax() {
        let args = |action| dispatch_args(&action, Syntax::Legacy);
        assert_eq!(args(CompositorAction::Workspace("e+1".into())), "workspace e+1");
        assert_eq!(args(CompositorAction::MoveToWorkspace("3".into())), "movetoworkspace 3");
        assert_eq!(args(CompositorAction::ToggleSpecial("magic".into())), "togglespecialworkspace magic");
        assert_eq!(args(CompositorAction::CloseWindow), "killactive");
        assert_eq!(args(CompositorAction::Maximize), "fullscreen 1");
    }

    #[test]
    fn lua_syntax() {
        let args = |action| dispatch_args(&action, Syntax::Lua);
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
    fn lua_strings_cannot_break_out() {
        assert_eq!(lua_string(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(lua_string("a\nb"), r#""a\nb""#);
    }
}
