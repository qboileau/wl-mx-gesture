//! HID++ features used to divert the gesture button and read its raw movement.

use wlmg_core::geometry::Vector;

use crate::Error;
use crate::channel::Channel;
use crate::report::{Length, Report};

pub(crate) const DIRECT_DEVICE_INDEX: u8 = 0xFF;
pub(crate) const RECEIVER_DEVICE_INDEXES: std::ops::RangeInclusive<u8> = 1..=6;

const ROOT_FEATURE_INDEX: u8 = 0x00;
const ROOT_GET_FEATURE: u8 = 0x0;
const FEATURE_NOT_FOUND: u8 = 0x00;

const REPROG_CONTROLS_V4: u16 = 0x1B04;
const GET_CONTROL_COUNT: u8 = 0x0;
const GET_CONTROL_INFO: u8 = 0x1;
const SET_CONTROL_REPORTING: u8 = 0x3;
pub(crate) const EVENT_DIVERTED_BUTTONS: u8 = 0x0;
pub(crate) const EVENT_DIVERTED_RAW_XY: u8 = 0x1;

pub(crate) const GESTURE_BUTTON: u16 = 0x00C3;
const DIVERT: u8 = 0x01;
const DIVERT_VALID: u8 = 0x02;
const RAW_XY: u8 = 0x10;
const RAW_XY_VALID: u8 = 0x20;

const CONTROL_FLAG_DIVERTABLE: u8 = 0x20;
const CONTROL_ADDITIONAL_FLAG_RAW_XY: u8 = 0x01;
const DIVERTED_BUTTON_SLOTS: usize = 4;

const SET_REGISTER: u8 = 0x80;
const REGISTER_NOTIFICATIONS: u8 = 0x00;
const NOTIFY_WIRELESS_AND_SOFTWARE_PRESENT: [u8; 3] = [0x00, 0x09, 0x00];
const DEVICE_CONNECTION: u8 = 0x41;
const LINK_NOT_ESTABLISHED: u8 = 0x40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Control {
    pub cid: u16,
    flags: u8,
    additional_flags: u8,
}

impl Control {
    pub fn divertable(&self) -> bool {
        self.flags & CONTROL_FLAG_DIVERTABLE != 0
    }

    pub fn raw_xy(&self) -> bool {
        self.additional_flags & CONTROL_ADDITIONAL_FLAG_RAW_XY != 0
    }
}

impl Channel {
    fn feature_index(&mut self, device_index: u8, feature: u16) -> Result<u8, Error> {
        let response = self.request(Report::feature_call(
            device_index,
            ROOT_FEATURE_INDEX,
            ROOT_GET_FEATURE,
            &feature.to_be_bytes(),
        ))?;
        match response.params[0] {
            FEATURE_NOT_FOUND => Err(Error::FeatureMissing(feature)),
            index => Ok(index),
        }
    }

    /// Returns the feature index the button and raw movement events will be reported on.
    pub fn divert_gesture_button(&mut self, device_index: u8) -> Result<u8, Error> {
        let feature = self.feature_index(device_index, REPROG_CONTROLS_V4)?;
        self.request(gesture_button_reporting(device_index, feature, DIVERT | DIVERT_VALID | RAW_XY | RAW_XY_VALID))?;
        Ok(feature)
    }

    pub fn controls(&mut self, device_index: u8) -> Result<Vec<Control>, Error> {
        let feature = self.feature_index(device_index, REPROG_CONTROLS_V4)?;
        let count = self.request(Report::feature_call(device_index, feature, GET_CONTROL_COUNT, &[]))?.params[0];
        (0..count)
            .map(|index| {
                let info =
                    self.request(Report::feature_call(device_index, feature, GET_CONTROL_INFO, &[index]))?.params;
                Ok(Control { cid: u16::from_be_bytes([info[0], info[1]]), flags: info[4], additional_flags: info[8] })
            })
            .collect()
    }

    /// Without this a receiver does not announce devices waking up, and their diversion would be lost.
    pub fn enable_receiver_notifications(&mut self) -> Result<(), Error> {
        let mut params = [0; 16];
        params[..3].copy_from_slice(&NOTIFY_WIRELESS_AND_SOFTWARE_PRESENT);
        self.request(Report {
            length: Length::Short,
            device_index: DIRECT_DEVICE_INDEX,
            feature_index: SET_REGISTER,
            address: REGISTER_NOTIFICATIONS,
            params,
        })
        .map(drop)
    }
}

/// Hands the gesture button back to the system, so it acts as a regular button again.
pub(crate) fn undivert_gesture_button(device_index: u8, feature_index: u8) -> Report {
    gesture_button_reporting(device_index, feature_index, DIVERT_VALID | RAW_XY_VALID)
}

fn gesture_button_reporting(device_index: u8, feature_index: u8, flags: u8) -> Report {
    let [cid_high, cid_low] = GESTURE_BUTTON.to_be_bytes();
    Report::feature_call(device_index, feature_index, SET_CONTROL_REPORTING, &[cid_high, cid_low, flags])
}

/// `Some(linked)` when the report is a receiver's device (dis)connection notification.
pub(crate) fn connection_change(report: &Report) -> Option<bool> {
    (report.length == Length::Short
        && report.feature_index == DEVICE_CONNECTION
        && RECEIVER_DEVICE_INDEXES.contains(&report.device_index))
    .then_some(report.params[0] & LINK_NOT_ESTABLISHED == 0)
}

pub(crate) fn gesture_button_held(params: &[u8]) -> bool {
    params.as_chunks::<2>().0.iter().take(DIVERTED_BUTTON_SLOTS).any(|&cid| u16::from_be_bytes(cid) == GESTURE_BUTTON)
}

pub(crate) fn raw_movement(params: &[u8]) -> Vector {
    let axis = |offset: usize| f32::from(i16::from_be_bytes([params[offset], params[offset + 1]]));
    Vector::new(axis(0), axis(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gesture_button_found_in_any_slot() {
        assert!(gesture_button_held(&[0x00, 0x52, 0x00, 0xC3, 0, 0, 0, 0]));
        assert!(!gesture_button_held(&[0x00, 0x52, 0, 0, 0, 0, 0, 0]));
        assert!(!gesture_button_held(&[0; 8]));
    }

    #[test]
    fn raw_movement_is_signed_big_endian() {
        assert_eq!(raw_movement(&[0xFF, 0xFE, 0x00, 0x05]), Vector::new(-2.0, 5.0));
    }

    #[test]
    fn connection_notification_reports_link_state() {
        let linked = Report::parse(&[0x10, 0x02, 0x41, 0x10, 0x02, 0x34, 0xB0]).unwrap();
        let unlinked = Report::parse(&[0x10, 0x02, 0x41, 0x10, 0x42, 0x34, 0xB0]).unwrap();
        assert_eq!(connection_change(&linked), Some(true));
        assert_eq!(connection_change(&unlinked), Some(false));
        assert_eq!(connection_change(&Report::feature_call(0x02, 0x41, 0, &[])), None);
    }
}
