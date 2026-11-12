use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use log::{debug, info, warn};
use wlmg_actions::Runner;
use wlmg_core::config;
use wlmg_core::geometry::Vector;
use wlmg_core::gesture::{Input, Output, Recognizer};
use wlmg_core::profile::Profile;
use wlmg_hidpp::Event;
use wlmg_overlay::Overlay;

const DEFAULT_CONFIG: &str = include_str!("../../../config/default.toml");
const CONFIG_FILE: &str = "wl-mx-gesture/config.toml";
const PREVIEW_STEP: Duration = Duration::from_millis(700);
const USAGE: &str = "usage: wl-mx-gesture [--probe | --test | --preview]
  (none)     run the daemon
  --probe    list Logitech devices and their reprogrammable controls
  --test     print mouse status and recognised gestures without running actions
  --preview  show the wheel with its style, cycling the highlighted slot";

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    match env::args().nth(1).as_deref() {
        None => run(),
        Some("--probe") => {
            probe();
            Ok(())
        }
        Some("--test") => test(),
        Some("--preview") => preview(),
        Some(other) => bail!("unknown argument `{other}`\n{USAGE}"),
    }
}

enum Seen<'a> {
    Event(&'a Event),
    Output(Output),
}

fn listen(profile: &Profile, mut see: impl FnMut(Seen)) -> Result<()> {
    let (sender, events) = mpsc::channel();
    let diversions = wlmg_hidpp::watch(sender);
    ctrlc::set_handler(move || {
        diversions.release();
        process::exit(0);
    })
    .context("cannot install the SIGINT/SIGTERM handler")?;
    let mut recognizer = Recognizer::new(profile.recognizer_settings());
    loop {
        let event = match recognizer.deadline() {
            None => Some(events.recv().context("device watcher stopped")?),
            Some(deadline) => match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(event) => Some(event),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => bail!("device watcher stopped"),
            },
        };
        let input = match &event {
            None => Input::Tick,
            Some(event) => {
                see(Seen::Event(event));
                match event {
                    Event::Input(input) => *input,
                    Event::Ready(_) | Event::Lost(_) => continue,
                }
            }
        };
        if let Some(output) = recognizer.handle(input, Instant::now()) {
            see(Seen::Output(output));
        }
    }
}

fn run() -> Result<()> {
    let profile = load_profile()?;
    let overlay = start_overlay(&profile);
    let mut runner = Runner::default();
    listen(&profile, |seen| match seen {
        Seen::Event(Event::Ready(device)) => info!("{device}: gesture button diverted"),
        Seen::Event(Event::Lost(device)) => info!("{device}: disconnected"),
        Seen::Event(Event::Input(_)) => {}
        Seen::Output(output) => {
            show(overlay.as_ref(), &profile, output);
            react(&profile, &mut runner, output);
        }
    })
}

fn start_overlay(profile: &Profile) -> Option<Overlay> {
    let overlay = Overlay::spawn(profile.style.clone())
        .inspect_err(|error| warn!("the wheel will not be shown: {error}"))
        .ok()?;
    profile.wheels().for_each(|wheel| overlay.prepare(wheel));
    Some(overlay)
}

fn show(overlay: Option<&Overlay>, profile: &Profile, output: Output) {
    let Some(overlay) = overlay else { return };
    match output {
        Output::WheelOpened(trigger) => {
            if let Some(wheel) = profile.wheel(trigger) {
                overlay.open(wheel);
            }
        }
        Output::WheelHovered(slot) => overlay.hover(slot),
        Output::WheelSelected(..) | Output::WheelCancelled => overlay.close(),
        Output::Gesture(_) => {}
    }
}

fn react(profile: &Profile, runner: &mut Runner, output: Output) {
    debug!("{output:?}");
    if let Some(action) = profile.action_for(output)
        && let Err(error) = runner.run(action)
    {
        warn!("{action:?} failed: {error}");
    }
}

struct Press {
    since: Instant,
    travel: Vector,
    swiped: bool,
}

fn test() -> Result<()> {
    let profile = load_profile()?;
    let overlay = start_overlay(&profile);
    println!("Listening to the gesture button; actions are printed, not run. Ctrl+C to stop.");
    let mut press: Option<Press> = None;
    listen(&profile, |seen| match seen {
        Seen::Event(Event::Ready(device)) => println!("{device}: connected, gesture button diverted"),
        Seen::Event(Event::Lost(device)) => println!("{device}: disconnected"),
        Seen::Event(Event::Input(Input::Pressed)) => {
            press = Some(Press { since: Instant::now(), travel: Vector::ZERO, swiped: false });
            println!("button pressed");
        }
        Seen::Event(Event::Input(Input::Moved(delta))) => {
            if let Some(press) = &mut press {
                press.travel = press.travel + *delta;
                if !press.swiped && press.travel.length() >= profile.gesture.swipe_threshold {
                    press.swiped = true;
                    println!(
                        "  raw movement reached the swipe threshold after {} ms",
                        press.since.elapsed().as_millis()
                    );
                }
            }
        }
        Seen::Event(Event::Input(Input::Released)) => {
            if let Some(Press { since, travel, .. }) = press.take() {
                println!(
                    "button released after {} ms, travel ({:+.0}, {:+.0}), distance {:.0}",
                    since.elapsed().as_millis(),
                    travel.x,
                    travel.y,
                    travel.length()
                );
            }
        }
        Seen::Event(Event::Input(Input::Tick)) => {}
        Seen::Output(output) => {
            show(overlay.as_ref(), &profile, output);
            match profile.action_for(output) {
                Some(action) => println!("  {output:?} → would run {action:?}"),
                None => println!("  {output:?}"),
            }
        }
    })
}

fn preview() -> Result<()> {
    let profile = load_profile()?;
    let wheel = profile.wheels().next().context("no binding opens a wheel, there is nothing to preview")?;
    let overlay = Overlay::spawn(profile.style.clone())?;
    println!("Showing the wheel at the cursor. Ctrl+C to stop.");
    overlay.open(wheel);
    for slot in (0..wheel.slot_count().get()).map(Some).chain([None]).cycle() {
        thread::sleep(PREVIEW_STEP);
        overlay.hover(slot);
    }
    Ok(())
}

fn load_profile() -> Result<Profile> {
    let path = config_path()?;
    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            info!("{} not found, using built-in defaults", path.display());
            DEFAULT_CONFIG.to_owned()
        }
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    config::parse(&source).with_context(|| format!("invalid configuration {}", path.display()))
}

fn config_path() -> Result<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .context("neither XDG_CONFIG_HOME nor HOME is set")?;
    Ok(base.join(CONFIG_FILE))
}

fn probe() {
    let devices = wlmg_hidpp::probe();
    if devices.is_empty() {
        println!("no HID++ device with reprogrammable controls found (run with RUST_LOG=debug for details)");
    }
    for device in devices {
        println!("{} device {:#04x}", device.path.display(), device.device_index);
        for control in device.controls {
            println!("  control {:#06x} divertable={} raw_xy={}", control.cid, control.divertable(), control.raw_xy());
        }
    }
}
