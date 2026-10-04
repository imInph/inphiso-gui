//! Platform-independent logic for inphiso: image inspection, decompression,
//! checksums and the write pipelines. Nothing in here needs elevated rights;
//! the privileged helper drives these pipelines against a raw device.

pub mod progress;
