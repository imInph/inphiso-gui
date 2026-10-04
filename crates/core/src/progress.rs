use serde::{Deserialize, Serialize};

/// Which stage of a flash a progress event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Write,
    Verify,
}

/// A progress snapshot sent from the helper to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub phase: Phase,
    /// Bytes processed so far in this phase.
    pub bytes: u64,
    /// Total bytes for this phase, if known (gzip/bzip2/zstd streams don't say).
    pub total: Option<u64>,
    /// Current throughput in bytes per second.
    pub speed: f64,
    /// Highest throughput seen in this phase.
    pub peak: f64,
    /// Estimated seconds remaining, if `total` is known and speed is non-zero.
    pub eta_secs: Option<u64>,
}

impl Progress {
    pub fn fraction(&self) -> Option<f64> {
        self.total
            .filter(|&t| t > 0)
            .map(|t| (self.bytes as f64 / t as f64).min(1.0))
    }
}

/// Estimate remaining seconds from bytes left and current speed.
pub fn eta(bytes: u64, total: Option<u64>, speed: f64) -> Option<u64> {
    let total = total?;
    if speed <= 0.0 || bytes >= total {
        return (bytes >= total).then_some(0);
    }
    Some(((total - bytes) as f64 / speed).ceil() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eta_math() {
        assert_eq!(eta(0, Some(100), 10.0), Some(10));
        assert_eq!(eta(95, Some(100), 10.0), Some(1));
        assert_eq!(eta(100, Some(100), 10.0), Some(0));
        assert_eq!(eta(10, None, 10.0), None);
        assert_eq!(eta(10, Some(100), 0.0), None);
    }

    #[test]
    fn fraction_clamps() {
        let p = Progress {
            phase: Phase::Write,
            bytes: 150,
            total: Some(100),
            speed: 1.0,
            peak: 1.0,
            eta_secs: None,
        };
        assert_eq!(p.fraction(), Some(1.0));
    }
}
