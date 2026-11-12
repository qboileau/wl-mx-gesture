//! Input adapter: turns the MX Master gesture button into [`Event`]s by diverting it
//! over HID++ on every Logitech hidraw node, through a Bolt/Unifying receiver or Bluetooth.

mod channel;
mod device;
mod hidraw;
mod report;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::{self, Write};
use std::iter;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use log::{debug, info, warn};
use wlmg_core::gesture::Input;

use channel::Channel;
pub use device::Control;
use device::{DIRECT_DEVICE_INDEX, EVENT_DIVERTED_BUTTONS, EVENT_DIVERTED_RAW_XY, RECEIVER_DEVICE_INDEXES};
use hidraw::{Bus, Node};
use report::Report;

/// Bluetooth reconnections create a new hidraw node; polling sysfs avoids a udev dependency.
const RESCAN_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "cannot open {path}: {source}. Install the udev rule packaging/70-wl-mx-gesture.rules \
         (NixOS: programs.wl-mx-gesture.enable = true) then replug the receiver or reconnect the mouse"
    )]
    Open { path: PathBuf, source: io::Error },
    #[error("hidraw i/o: {0}")]
    Io(#[from] io::Error),
    #[error("no answer from device")]
    Timeout,
    #[error("device disconnected")]
    Disconnected,
    #[error("device answered with HID++ error {0:#04x}")]
    Protocol(u8),
    #[error("device lacks HID++ feature {0:#06x}")]
    FeatureMissing(u16),
}

#[derive(Debug)]
pub struct ProbedDevice {
    pub path: PathBuf,
    pub device_index: u8,
    pub controls: Vec<Control>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId {
    pub path: PathBuf,
    /// HID++ device index: 1..=6 behind a receiver, 0xFF when directly connected.
    pub index: u8,
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} device {:#04x}", self.path.display(), self.index)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Input(Input),
    /// The gesture button is diverted: its events now reach us instead of the system.
    Ready(DeviceId),
    Lost(DeviceId),
}

/// Gesture buttons currently diverted, to hand them back to the system before exiting:
/// a diversion outlives the process and would leave the button dead until the mouse reconnects.
#[derive(Clone, Default)]
pub struct Diversions(Arc<Mutex<HashMap<DeviceId, Restore>>>);

struct Restore {
    writer: File,
    feature_index: u8,
}

impl Diversions {
    pub fn release(&self) {
        for (device, mut restore) in self.0.lock().unwrap_or_else(PoisonError::into_inner).drain() {
            match restore
                .writer
                .write_all(&device::undivert_gesture_button(device.index, restore.feature_index).encode())
            {
                Ok(()) => info!("{device}: gesture button released"),
                Err(error) => warn!("{device}: cannot release gesture button: {error}"),
            }
        }
    }

    fn add(&self, device: DeviceId, restore: Restore) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).insert(device, restore);
    }

    fn remove(&self, device: &DeviceId) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).remove(device);
    }
}

/// Watches for Logitech devices forever, sending their gesture button events to `events`.
pub fn watch(events: Sender<Event>) -> Diversions {
    let diversions = Diversions::default();
    let watched = diversions.clone();
    thread::spawn(move || {
        let mut known = HashSet::new();
        loop {
            let nodes = hidraw::logitech_nodes();
            for node in nodes.iter().filter(|node| !known.contains(&node.id)).cloned() {
                let (events, diversions) = (events.clone(), watched.clone());
                thread::spawn(move || {
                    let path = node.path.clone();
                    match serve(node, events, diversions) {
                        Error::Disconnected => debug!("{}: gone", path.display()),
                        error => warn!("{}: {error}", path.display()),
                    }
                });
            }
            known = nodes.into_iter().map(|node| node.id).collect();
            thread::sleep(RESCAN_INTERVAL);
        }
    });
    diversions
}

pub fn probe() -> Vec<ProbedDevice> {
    hidraw::logitech_nodes()
        .into_iter()
        .filter_map(|node| {
            Channel::open(&node.path).inspect_err(|error| warn!("{error}")).ok().map(|channel| (node, channel))
        })
        .flat_map(|(node, mut channel)| {
            device_indexes(node.bus)
                .filter_map(|device_index| {
                    channel
                        .controls(device_index)
                        .inspect_err(|error| debug!("{} device {device_index:#04x}: {error}", node.path.display()))
                        .ok()
                        .map(|controls| ProbedDevice { path: node.path.clone(), device_index, controls })
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn device_indexes(bus: Bus) -> impl Iterator<Item = u8> {
    let receiver_slots = (bus == Bus::Usb).then_some(RECEIVER_DEVICE_INDEXES).into_iter().flatten();
    iter::once(DIRECT_DEVICE_INDEX).chain(receiver_slots)
}

struct Diverted {
    feature_index: u8,
    held: bool,
}

/// Diverts the gesture button of every device reachable through one hidraw node and relays its events.
struct Session {
    node: Node,
    channel: Channel,
    events: Sender<Event>,
    diverted: HashMap<u8, Diverted>,
    diversions: Diversions,
}

fn serve(node: Node, events: Sender<Event>, diversions: Diversions) -> Error {
    let channel = match Channel::open(&node.path) {
        Ok(channel) => channel,
        Err(error) => return error,
    };
    let mut session = Session { node, channel, events, diverted: HashMap::new(), diversions };
    session.start();
    let error = session.relay();
    let lost: Vec<u8> = session.diverted.keys().copied().collect();
    lost.into_iter().for_each(|index| session.lose(index));
    error
}

impl Session {
    fn device(&self, index: u8) -> DeviceId {
        DeviceId { path: self.node.path.clone(), index }
    }

    fn emit(&self, event: Event) {
        // The listener only goes away when the process is exiting.
        let _ = self.events.send(event);
    }

    fn start(&mut self) {
        if self.node.bus == Bus::Usb
            && let Err(error) = self.channel.enable_receiver_notifications()
        {
            debug!("{}: no receiver notifications: {error}", self.node.path.display());
        }
        device_indexes(self.node.bus).for_each(|index| self.divert(index));
    }

    fn divert(&mut self, index: u8) {
        match self.channel.divert_gesture_button(index) {
            Ok(feature_index) => {
                self.diverted.insert(index, Diverted { feature_index, held: false });
                match self.channel.writer() {
                    Ok(writer) => self.diversions.add(self.device(index), Restore { writer, feature_index }),
                    Err(error) => warn!("{}: button will stay diverted after exit: {error}", self.device(index)),
                }
                self.emit(Event::Ready(self.device(index)));
            }
            Err(error) => debug!("{}: {error}", self.device(index)),
        }
    }

    fn lose(&mut self, index: u8) {
        if self.diverted.remove(&index).is_some() {
            self.diversions.remove(&self.device(index));
            self.emit(Event::Lost(self.device(index)));
        }
    }

    fn relay(&mut self) -> Error {
        loop {
            let report = match self.channel.next_report() {
                Ok(report) => report,
                Err(error) => return error,
            };
            match device::connection_change(&report) {
                Some(true) => self.divert(report.device_index),
                Some(false) => self.lose(report.device_index),
                None => {
                    if let Some(input) = self.input(&report) {
                        self.emit(Event::Input(input));
                    }
                }
            }
        }
    }

    fn input(&mut self, report: &Report) -> Option<Input> {
        let device = self
            .diverted
            .get_mut(&report.device_index)
            .filter(|device| device.feature_index == report.feature_index)?;
        match report.event()? {
            EVENT_DIVERTED_BUTTONS => {
                let held = device::gesture_button_held(&report.params);
                if held == device.held {
                    return None;
                }
                device.held = held;
                Some(if held { Input::Pressed } else { Input::Released })
            }
            EVENT_DIVERTED_RAW_XY => Some(Input::Moved(device::raw_movement(&report.params))),
            _ => None,
        }
    }
}
