//! Platform-independent logic for inphiso: image inspection, decompression,
//! checksums and the write pipelines. Nothing in here needs elevated rights;
//! the privileged helper drives these pipelines against a raw device.

pub mod blockcache;
pub mod blockio;
pub mod bootcode;
pub mod checksum;
pub mod image;
pub mod ipc;
pub mod partition;
pub mod progress;
pub mod udf;

#[cfg(test)]
pub(crate) mod testutil;
pub mod write_raw;
pub mod write_windows;

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cancelled")]
    Cancelled,
    #[error("the image ({image} bytes) is bigger than the drive ({drive} bytes)")]
    TooLarge { image: u64, drive: u64 },
    #[error("verification failed: the drive returned different data than was written")]
    VerifyMismatch,
    #[error("the drive ended early while reading it back")]
    ShortRead,
    #[error("{0}")]
    Unsupported(String),
    /// Shown as "<what>: <cause>" by error chains (`{:#}`).
    #[error("{what}")]
    Step {
        what: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Labels an I/O error with the step it happened in, so failures say where.
pub(crate) trait StepExt<T> {
    fn step(self, what: &'static str) -> Result<T>;
}

impl<T> StepExt<T> for std::io::Result<T> {
    fn step(self, what: &'static str) -> Result<T> {
        self.map_err(|source| Error::Step { what, source })
    }
}

pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
