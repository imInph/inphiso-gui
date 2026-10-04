//! Platform-independent logic for inphiso: image inspection, decompression,
//! checksums and the write pipelines. Nothing in here needs elevated rights;
//! the privileged helper drives these pipelines against a raw device.

pub mod blockcache;
pub mod blockio;
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
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
