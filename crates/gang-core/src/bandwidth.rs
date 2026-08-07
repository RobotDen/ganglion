//! Named bandwidth profiles — degraded-link streaming presets.
//!
//! Field links are rarely the clean gigabit of a lab bench: cellular modems,
//! saturated warehouse Wi-Fi, and relay hops all mean the operator has to trade
//! fidelity for reachability. Rather than make every engineer hand-tune
//! decimation and payload caps per topic, Ganglion ships a small set of named
//! presets — `full`, `lidar-low`, `vision-low`, `logs-only` — that any
//! streaming surface (the `topic-echo` capability, the Foxglove projection
//! bridge) can apply with a single `--profile <name>`.
//!
//! A profile is intentionally a *transport-shaping* concept, not a policy
//! concept: it never grants access to a topic (that is the default-deny policy
//! engine's job), it only decides how much of an already-permitted stream to
//! forward. The three knobs compose:
//!
//! - `decimation` — forward every Nth message (1 = every message).
//! - `max_bytes_per_message` — skip individual messages larger than this cap
//!   (drops the occasional huge frame without starving the whole stream).
//! - `max_rate_hz` — an optional ceiling on forwarded messages per second per
//!   topic, enforced by the consumer as a minimum inter-message interval.
//!
//! Profiles are data, so operators can define their own in config and pass the
//! name through unchanged; `BandwidthProfile::resolve` checks the built-ins
//! first and then a caller-supplied set.

use serde::{Deserialize, Serialize};

/// A named degraded-link streaming preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BandwidthProfile {
    /// Short, stable name used on the command line (`--profile lidar-low`).
    pub name: String,
    /// One-line human description shown in listings.
    pub description: String,
    /// Forward every Nth message. Values below 1 are treated as 1.
    pub decimation: u32,
    /// Skip any single message larger than this many bytes. `None` = no cap.
    pub max_bytes_per_message: Option<u64>,
    /// Ceiling on forwarded messages per second per topic. `None` = unlimited.
    pub max_rate_hz: Option<f64>,
}

impl BandwidthProfile {
    /// The names of every built-in profile, in listing order.
    pub const BUILTIN_NAMES: [&'static str; 4] = ["full", "lidar-low", "vision-low", "logs-only"];

    /// The unshaped default: forward everything.
    pub fn full() -> Self {
        Self {
            name: "full".into(),
            description: "No shaping — forward every message at full fidelity.".into(),
            decimation: 1,
            max_bytes_per_message: None,
            max_rate_hz: None,
        }
    }

    /// Heavy decimation for high-rate point clouds on a thin link.
    fn lidar_low() -> Self {
        Self {
            name: "lidar-low".into(),
            description: "Point clouds on a thin link: 1-in-10 messages, ~2 Hz ceiling.".into(),
            decimation: 10,
            max_bytes_per_message: None,
            max_rate_hz: Some(2.0),
        }
    }

    /// Image/vision topics capped in both rate and per-frame size.
    fn vision_low() -> Self {
        Self {
            name: "vision-low".into(),
            description: "Camera/vision topics: 1-in-5 messages, ~1 Hz, 256 KiB/frame cap.".into(),
            decimation: 5,
            max_bytes_per_message: Some(256 * 1024),
            max_rate_hz: Some(1.0),
        }
    }

    /// Text-scale telemetry only — the last-resort link.
    fn logs_only() -> Self {
        Self {
            name: "logs-only".into(),
            description: "Last-resort link: every message but only small (<=16 KiB) payloads."
                .into(),
            decimation: 1,
            max_bytes_per_message: Some(16 * 1024),
            max_rate_hz: None,
        }
    }

    /// All built-in profiles, in [`Self::BUILTIN_NAMES`] order.
    pub fn builtins() -> Vec<Self> {
        vec![
            Self::full(),
            Self::lidar_low(),
            Self::vision_low(),
            Self::logs_only(),
        ]
    }

    /// Look up a built-in profile by name (case-insensitive).
    pub fn builtin(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        Self::builtins().into_iter().find(|p| p.name == name)
    }

    /// Resolve a profile name against the built-ins first, then a caller's own
    /// custom profiles (e.g. loaded from operator config). Returns `None` when
    /// the name matches neither, so the caller can surface the valid choices.
    pub fn resolve(name: &str, custom: &[BandwidthProfile]) -> Option<Self> {
        Self::builtin(name).or_else(|| {
            let name = name.trim().to_ascii_lowercase();
            custom
                .iter()
                .find(|p| p.name.to_ascii_lowercase() == name)
                .cloned()
        })
    }

    /// Effective decimation factor (never zero).
    pub fn effective_decimation(&self) -> u32 {
        self.decimation.max(1)
    }

    /// Whether a message of `bytes` length should be forwarded under this
    /// profile's per-message size cap. Messages exactly at the cap pass.
    pub fn allows_message_size(&self, bytes: u64) -> bool {
        match self.max_bytes_per_message {
            Some(cap) => bytes <= cap,
            None => true,
        }
    }

    /// The minimum interval between forwarded messages implied by `max_rate_hz`,
    /// in milliseconds. `None` when the profile sets no rate ceiling.
    pub fn min_interval_ms(&self) -> Option<u64> {
        self.max_rate_hz.and_then(|hz| {
            if hz > 0.0 {
                Some((1000.0 / hz).round() as u64)
            } else {
                None
            }
        })
    }
}

impl Default for BandwidthProfile {
    fn default() -> Self {
        Self::full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_cover_declared_names() {
        let names: Vec<String> = BandwidthProfile::builtins()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, BandwidthProfile::BUILTIN_NAMES);
    }

    #[test]
    fn lookup_is_case_insensitive() {
        assert_eq!(
            BandwidthProfile::builtin("LIDAR-LOW").unwrap().decimation,
            10
        );
        assert_eq!(BandwidthProfile::builtin("  full ").unwrap().name, "full");
        assert!(BandwidthProfile::builtin("nope").is_none());
    }

    #[test]
    fn resolve_prefers_builtin_then_custom() {
        let custom = vec![BandwidthProfile {
            name: "my-link".into(),
            description: "custom".into(),
            decimation: 3,
            max_bytes_per_message: None,
            max_rate_hz: None,
        }];
        assert_eq!(
            BandwidthProfile::resolve("my-link", &custom)
                .unwrap()
                .decimation,
            3
        );
        assert_eq!(
            BandwidthProfile::resolve("full", &custom).unwrap().name,
            "full"
        );
        assert!(BandwidthProfile::resolve("missing", &custom).is_none());
    }

    #[test]
    fn size_cap_boundary_passes() {
        let p = BandwidthProfile::builtin("logs-only").unwrap();
        assert!(p.allows_message_size(16 * 1024));
        assert!(!p.allows_message_size(16 * 1024 + 1));
        assert!(BandwidthProfile::full().allows_message_size(u64::MAX));
    }

    #[test]
    fn rate_ceiling_maps_to_interval() {
        assert_eq!(
            BandwidthProfile::builtin("lidar-low")
                .unwrap()
                .min_interval_ms(),
            Some(500)
        );
        assert_eq!(BandwidthProfile::full().min_interval_ms(), None);
    }

    #[test]
    fn decimation_never_zero() {
        let p = BandwidthProfile {
            name: "z".into(),
            description: String::new(),
            decimation: 0,
            max_bytes_per_message: None,
            max_rate_hz: None,
        };
        assert_eq!(p.effective_decimation(), 1);
    }
}
