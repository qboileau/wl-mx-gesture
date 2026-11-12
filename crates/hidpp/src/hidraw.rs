//! Discovery of Logitech HID++ capable `/dev/hidraw*` nodes through sysfs.

use std::fs;
use std::path::{Path, PathBuf};

use log::{debug, warn};

use crate::report::LONG_REPORT_ID;

const SYSFS_HIDRAW: &str = "/sys/class/hidraw";
const LOGITECH_VENDOR: &str = "046D";
const REPORT_ID_ITEM: u8 = 0x85;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bus {
    /// A receiver (Bolt/Unifying) or a mouse plugged by cable.
    Usb,
    Bluetooth,
}

#[derive(Debug, Clone)]
pub(crate) struct Node {
    pub path: PathBuf,
    /// Kernel HID device name, e.g. `0005:046D:B034.0015`; the suffix changes on every reconnection.
    pub id: String,
    pub bus: Bus,
}

pub(crate) fn logitech_nodes() -> Vec<Node> {
    let entries = match fs::read_dir(SYSFS_HIDRAW) {
        Ok(entries) => entries,
        Err(error) => {
            warn!("cannot list {SYSFS_HIDRAW}: {error}");
            return Vec::new();
        }
    };
    entries.filter_map(Result::ok).filter_map(|entry| node(&entry.path())).collect()
}

fn node(sysfs_entry: &Path) -> Option<Node> {
    let device = sysfs_entry.join("device");
    let id = fs::canonicalize(&device).ok()?.file_name()?.to_str()?.to_owned();
    let bus = logitech_bus(&id)?;
    let descriptor = fs::read(device.join("report_descriptor"))
        .inspect_err(|error| debug!("{id}: cannot read report descriptor: {error}"))
        .ok()?;
    let path = Path::new("/dev").join(sysfs_entry.file_name()?);
    supports_hidpp(&descriptor).then_some(Node { path, id, bus })
}

fn logitech_bus(id: &str) -> Option<Bus> {
    let mut fields = id.split(':');
    let bus = match fields.next()? {
        "0003" => Bus::Usb,
        "0005" => Bus::Bluetooth,
        _ => return None,
    };
    (fields.next()? == LOGITECH_VENDOR).then_some(bus)
}

fn supports_hidpp(descriptor: &[u8]) -> bool {
    descriptor.windows(2).any(|item| item == [REPORT_ID_ITEM, LONG_REPORT_ID])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_logitech_buses() {
        assert_eq!(logitech_bus("0003:046D:C548.0009"), Some(Bus::Usb));
        assert_eq!(logitech_bus("0005:046D:B034.0015"), Some(Bus::Bluetooth));
        assert_eq!(logitech_bus("0003:00003434:0E20.0001"), None);
        assert_eq!(logitech_bus("0018:046D:0001.0001"), None);
    }

    #[test]
    fn detects_hidpp_long_report_in_bolt_descriptor() {
        let bolt_hidpp_interface = [
            0x06, 0x00, 0xFF, 0x09, 0x01, 0xA1, 0x01, 0x85, 0x10, 0x95, 0x06, 0x75, 0x08, 0x15, 0x00, 0x26, 0xFF, 0x00,
            0x09, 0x01, 0x81, 0x00, 0x09, 0x01, 0x91, 0x00, 0xC0, 0x06, 0x00, 0xFF, 0x09, 0x02, 0xA1, 0x01, 0x85, 0x11,
            0x95, 0x13,
        ];
        let bolt_mouse_interface = [0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x85, 0x02, 0x09, 0x01];
        assert!(supports_hidpp(&bolt_hidpp_interface));
        assert!(!supports_hidpp(&bolt_mouse_interface));
    }
}
