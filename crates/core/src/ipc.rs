//! Messages between the app and the privileged helper.
//!
//! The app listens on a local socket and launches the helper with the socket
//! name and a one-time token. The helper connects, sends [`HelperMsg::Hello`]
//! with the token, and only then receives the job. Messages are one JSON
//! object per line in both directions.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::progress::Progress;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PartitionScheme {
    Mbr,
    Gpt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Mode {
    /// Copy the (possibly compressed) image byte for byte.
    Raw,
    /// Partition, format FAT32 and copy the files of a Windows ISO.
    Windows { scheme: PartitionScheme },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub image: PathBuf,
    /// Drive id as listed: `disk4`, `/dev/sdb`, `\\.\PhysicalDrive2`.
    pub device: String,
    /// Size the UI showed. The helper refuses the job if the drive no longer matches.
    pub device_size: u64,
    pub mode: Mode,
    pub verify: bool,
}

/// App → helper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AppMsg {
    Start { job: Job },
    Cancel,
}

/// Helper → app. Everything after `Hello` is forwarded to the UI unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum HelperMsg {
    Hello {
        token: String,
        version: String,
    },
    Progress {
        progress: Progress,
    },
    #[serde(rename_all = "camelCase")]
    Done {
        elapsed_ms: u64,
        verified: bool,
    },
    Error {
        message: String,
    },
    Cancelled,
}

pub fn send<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    let mut line = serde_json::to_vec(msg).map_err(io::Error::other)?;
    line.push(b'\n');
    w.write_all(&line)?;
    w.flush()
}

/// Reads the next message; `Ok(None)` when the other side closed the connection.
pub fn recv<T: DeserializeOwned>(r: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    serde_json::from_str(&line)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::Phase;

    #[test]
    fn round_trips_over_a_byte_stream() {
        let job = Job {
            image: "/tmp/ubuntu.iso".into(),
            device: "disk4".into(),
            device_size: 32_000_000_000,
            mode: Mode::Windows { scheme: PartitionScheme::Gpt },
            verify: true,
        };
        let mut wire = Vec::new();
        send(&mut wire, &AppMsg::Start { job: job.clone() }).unwrap();
        send(&mut wire, &AppMsg::Cancel).unwrap();

        let mut r = io::Cursor::new(wire);
        assert_eq!(recv::<AppMsg>(&mut r).unwrap(), Some(AppMsg::Start { job }));
        assert_eq!(recv::<AppMsg>(&mut r).unwrap(), Some(AppMsg::Cancel));
        assert_eq!(recv::<AppMsg>(&mut r).unwrap(), None);
    }

    #[test]
    fn helper_messages_match_the_frontend_shape() {
        let p = HelperMsg::Progress {
            progress: Progress {
                phase: Phase::Write,
                bytes: 10,
                total: Some(20),
                speed: 1.5,
                peak: 2.0,
                eta_secs: Some(7),
            },
        };
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["type"], "progress");
        assert_eq!(v["progress"]["etaSecs"], 7);

        let d = serde_json::to_value(HelperMsg::Done { elapsed_ms: 5, verified: true }).unwrap();
        assert_eq!(d, serde_json::json!({"type": "done", "elapsedMs": 5, "verified": true}));
        let c = serde_json::to_value(HelperMsg::Cancelled).unwrap();
        assert_eq!(c, serde_json::json!({"type": "cancelled"}));
    }
}
