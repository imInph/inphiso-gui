use std::collections::VecDeque;
use std::time::{Duration, Instant};

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
#[serde(rename_all = "camelCase")]
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

/// Speed is averaged over this window so one slow chunk doesn't make the number jump.
const WINDOW: Duration = Duration::from_secs(3);
/// Don't flood the UI: at most this many events per second.
const EMIT_EVERY: Duration = Duration::from_millis(200);

/// Turns a stream of "N more bytes done" into throttled [`Progress`] snapshots.
pub struct Tracker {
    phase: Phase,
    total: Option<u64>,
    bytes: u64,
    peak: f64,
    samples: VecDeque<(Instant, u64)>,
    last_emit: Option<Instant>,
}

impl Tracker {
    pub fn new(phase: Phase, total: Option<u64>) -> Self {
        let mut samples = VecDeque::new();
        samples.push_back((Instant::now(), 0));
        Self {
            phase,
            total,
            bytes: 0,
            peak: 0.0,
            samples,
            last_emit: None,
        }
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Records progress; returns a snapshot when it's time to tell the UI.
    pub fn advance(&mut self, n: u64) -> Option<Progress> {
        self.advance_at(n, Instant::now())
    }

    fn advance_at(&mut self, n: u64, now: Instant) -> Option<Progress> {
        self.bytes += n;
        self.samples.push_back((now, self.bytes));
        while self.samples.len() > 2 && now.duration_since(self.samples[0].0) > WINDOW {
            self.samples.pop_front();
        }
        if self.last_emit.is_some_and(|t| now.duration_since(t) < EMIT_EVERY) {
            return None;
        }
        self.last_emit = Some(now);
        Some(self.snapshot_at(now))
    }

    /// A snapshot regardless of throttling, e.g. for the final 100% event.
    pub fn snapshot(&mut self) -> Progress {
        self.snapshot_at(Instant::now())
    }

    fn snapshot_at(&mut self, now: Instant) -> Progress {
        let (t0, b0) = self.samples[0];
        let secs = now.duration_since(t0).as_secs_f64();
        let speed = if secs > 0.05 {
            (self.bytes - b0) as f64 / secs
        } else {
            0.0
        };
        self.peak = self.peak.max(speed);
        Progress {
            phase: self.phase,
            bytes: self.bytes,
            total: self.total,
            speed,
            peak: self.peak,
            eta_secs: eta(self.bytes, self.total, speed),
        }
    }
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

    #[test]
    fn tracker_throttles_and_measures() {
        let mut t = Tracker::new(Phase::Write, Some(10_000_000));
        let start = t.samples[0].0;
        let at = |ms| start + Duration::from_millis(ms);

        let first = t.advance_at(1_000_000, at(500)).expect("first event goes out");
        assert!((first.speed - 2_000_000.0).abs() < 1.0);
        // 100 ms later: throttled.
        assert!(t.advance_at(1_000_000, at(600)).is_none());
        let p = t.advance_at(1_000_000, at(1000)).expect("after 200 ms");
        assert_eq!(p.bytes, 3_000_000);
        assert!((p.speed - 3_000_000.0).abs() < 1.0);
        assert_eq!(p.eta_secs, Some(3));
        assert!(p.peak >= p.speed);
    }

    #[test]
    fn serializes_camel_case() {
        let p = Progress {
            phase: Phase::Verify,
            bytes: 1,
            total: None,
            speed: 0.0,
            peak: 0.0,
            eta_secs: None,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"etaSecs\":null"), "{json}");
        assert!(json.contains("\"phase\":\"verify\""), "{json}");
    }
}
