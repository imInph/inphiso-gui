//! Everything that differs between Linux, macOS and Windows lives here,
//! behind one set of types so the rest of inphiso doesn't care.

use serde::{Deserialize, Serialize};

/// A whole physical disk that could be flashed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drive {
    /// OS identifier: `/dev/sdb`, `disk4`, `\\.\PhysicalDrive2`.
    pub id: String,
    /// Human name, e.g. "SanDisk Ultra".
    pub name: String,
    pub size: u64,
    /// "USB 3.0", "USB-C", "SD", ... when the OS tells us.
    pub bus: Option<String>,
    /// True for USB / SD media. Non-removable drives only show with "Show all drives".
    pub removable: bool,
    /// Mounted volume names currently on the disk, shown in the erase confirmation.
    pub volumes: Vec<String>,
}
