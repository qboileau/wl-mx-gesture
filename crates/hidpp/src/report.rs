//! HID++ wire format. Spec: https://lekensteyn.nl/files/logitech/

const SHORT_REPORT_ID: u8 = 0x10;
const SHORT_LEN: usize = 7;
pub(crate) const LONG_REPORT_ID: u8 = 0x11;
const LONG_LEN: usize = 20;
const HEADER_LEN: usize = 4;
const PARAMS_LEN: usize = LONG_LEN - HEADER_LEN;

const HIDPP10_ERROR: u8 = 0x8F;
const HIDPP20_ERROR: u8 = 0xFF;
pub(crate) const SOFTWARE_ID: u8 = 0x07;
const SOFTWARE_ID_MASK: u8 = 0x0F;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Length {
    Short,
    Long,
}

/// For HID++ 2.0, `address` is `function << 4 | software id`;
/// for HID++ 1.0, `feature_index` is the sub id and `address` the register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Report {
    pub length: Length,
    pub device_index: u8,
    pub feature_index: u8,
    pub address: u8,
    pub params: [u8; PARAMS_LEN],
}

impl Report {
    pub fn feature_call(device_index: u8, feature_index: u8, function: u8, args: &[u8]) -> Self {
        let mut params = [0; PARAMS_LEN];
        params[..args.len()].copy_from_slice(args);
        Self { length: Length::Long, device_index, feature_index, address: function << 4 | SOFTWARE_ID, params }
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let (length, size) = match *bytes.first()? {
            SHORT_REPORT_ID => (Length::Short, SHORT_LEN),
            LONG_REPORT_ID => (Length::Long, LONG_LEN),
            _ => return None,
        };
        let bytes = bytes.get(..size)?;
        let mut params = [0; PARAMS_LEN];
        params[..size - HEADER_LEN].copy_from_slice(&bytes[HEADER_LEN..]);
        Some(Self { length, device_index: bytes[1], feature_index: bytes[2], address: bytes[3], params })
    }

    pub fn encode(&self) -> Vec<u8> {
        let (report_id, size) = match self.length {
            Length::Short => (SHORT_REPORT_ID, SHORT_LEN),
            Length::Long => (LONG_REPORT_ID, LONG_LEN),
        };
        [&[report_id, self.device_index, self.feature_index, self.address], &self.params[..size - HEADER_LEN]].concat()
    }

    pub fn answers(&self, request: &Report) -> bool {
        (self.device_index, self.feature_index, self.address)
            == (request.device_index, request.feature_index, request.address)
    }

    /// HID++ 1.0 and 2.0 errors share the layout `[error id, request feature, request address, code]`.
    pub fn error_code_for(&self, request: &Report) -> Option<u8> {
        (matches!(self.feature_index, HIDPP10_ERROR | HIDPP20_ERROR)
            && self.device_index == request.device_index
            && self.address == request.feature_index
            && self.params[0] == request.address)
            .then_some(self.params[1])
    }

    /// Function number of a device-initiated HID++ 2.0 event, which carries software id 0.
    pub fn event(&self) -> Option<u8> {
        (self.address & SOFTWARE_ID_MASK == 0).then_some(self.address >> 4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_report_round_trips() {
        let request = Report::feature_call(0x01, 0x00, 0x0, &[0x1B, 0x04]);
        let bytes = request.encode();
        assert_eq!(bytes.len(), LONG_LEN);
        assert_eq!(&bytes[..6], &[0x11, 0x01, 0x00, 0x07, 0x1B, 0x04]);
        assert_eq!(Report::parse(&bytes), Some(request));
    }

    #[test]
    fn short_report_is_zero_padded() {
        let report = Report::parse(&[0x10, 0x02, 0x41, 0x04, 0x40, 0x34, 0xB0]).unwrap();
        assert_eq!((report.length, report.feature_index, report.params[0]), (Length::Short, 0x41, 0x40));
        assert_eq!(report.params[3..], [0; PARAMS_LEN - 3]);
    }

    #[test]
    fn truncated_or_foreign_reports_are_ignored() {
        assert_eq!(Report::parse(&[0x11, 0x01, 0x00]), None);
        assert_eq!(Report::parse(&[0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]), None);
    }

    #[test]
    fn hidpp20_error_matches_its_request() {
        let request = Report::feature_call(0x01, 0x05, 0x3, &[]);
        let error =
            Report::parse(&[0x11, 0x01, 0xFF, 0x05, 0x37, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(error.error_code_for(&request), Some(0x02));
        assert!(!error.answers(&request));
    }

    #[test]
    fn hidpp10_error_matches_its_request() {
        let request = Report::feature_call(0x03, 0x00, 0x0, &[]);
        let error = Report::parse(&[0x10, 0x03, 0x8F, 0x00, 0x07, 0x09, 0x00]).unwrap();
        assert_eq!(error.error_code_for(&request), Some(0x09));
    }

    #[test]
    fn events_have_no_software_id() {
        let event = Report::parse(&[0x11, 0x01, 0x08, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(event.event(), Some(1));
        assert_eq!(Report::feature_call(0x01, 0x08, 0x1, &[]).event(), None);
    }
}
