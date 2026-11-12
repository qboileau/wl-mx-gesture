use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use log::debug;

use crate::Error;
use crate::report::Report;

/// A sleeping mouse behind a receiver needs about a second to wake up and answer.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const READ_BUFFER_LEN: usize = 64;

/// Request/response over one hidraw node; a reader thread feeds every incoming report.
pub(crate) struct Channel {
    writer: File,
    reports: Receiver<Report>,
}

impl Channel {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| Error::Open { path: path.to_owned(), source })?;
        let mut reader = file.try_clone()?;
        let (sender, reports) = mpsc::channel();
        let name = path.display().to_string();
        thread::spawn(move || {
            let mut buffer = [0; READ_BUFFER_LEN];
            loop {
                match reader.read(&mut buffer) {
                    Ok(len) => {
                        if let Some(report) = Report::parse(&buffer[..len])
                            && sender.send(report).is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => return debug!("{name}: reader stopped: {error}"),
                }
            }
        });
        Ok(Self { writer: file, reports })
    }

    /// Reports unrelated to the request that arrive meanwhile are dropped.
    pub fn request(&mut self, request: Report) -> Result<Report, Error> {
        self.writer.write_all(&request.encode())?;
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            let report =
                self.reports.recv_timeout(deadline.saturating_duration_since(Instant::now())).map_err(|error| {
                    match error {
                        RecvTimeoutError::Timeout => Error::Timeout,
                        RecvTimeoutError::Disconnected => Error::Disconnected,
                    }
                })?;
            if report.answers(&request) {
                return Ok(report);
            }
            if let Some(code) = report.error_code_for(&request) {
                return Err(Error::Protocol(code));
            }
        }
    }

    pub fn next_report(&self) -> Result<Report, Error> {
        self.reports.recv().map_err(|_| Error::Disconnected)
    }
}
