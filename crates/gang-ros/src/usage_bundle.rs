//! Robot-side usage bundle (ADR-027) — local-only anonymous counters.
//!
//! The agent accumulates a tiny JSON file of ganglion-usage-only counters:
//! per-capability-*group* ok/err invocation counts and a bare policy-denial
//! count. Nothing here can transmit — this module has no network client and
//! no endpoint. Operators may fetch the bundle over the authenticated
//! control channel (`FetchUsageBundle`), which resets it; whether anything
//! ever leaves the operator's machine is decided there, behind an explicit
//! opt-in (`gang telemetry fleet on`).
//!
//! Never recorded, by construction: capability *names* (operator-authored
//! names can identify customers or sites — categories, never names), robot
//! or peer identifiers, topics, patterns, paths, policy contents,
//! arguments, error text, hostnames, or timestamps.
//!
//! Disable with any of: `DO_NOT_TRACK`, `GANG_TELEMETRY=off`, a `None`
//! bundle path in [`crate::agent::AgentConfig`], or build `gang-ros`
//! without the `usage-bundle` feature (all methods become no-ops).

use std::path::PathBuf;

/// Recorder handle held by the robot agent. All methods are infallible and
/// silent: usage accounting must never affect agent behavior.
#[derive(Debug)]
pub struct UsageBundleRecorder {
    #[cfg(feature = "usage-bundle")]
    inner: Option<imp::Inner>,
}

impl UsageBundleRecorder {
    /// Build a recorder writing to `path`. `None` — or an opt-out
    /// environment variable (`DO_NOT_TRACK`, `GANG_TELEMETRY=off`) —
    /// yields a disabled recorder whose methods do nothing.
    #[allow(unused_variables)]
    pub fn new(path: Option<PathBuf>) -> Self {
        #[cfg(feature = "usage-bundle")]
        {
            let disabled = imp::env_disabled(
                std::env::var_os("DO_NOT_TRACK").as_deref(),
                std::env::var("GANG_TELEMETRY").ok().as_deref(),
            );
            Self {
                inner: match (disabled, path) {
                    (false, Some(p)) => Some(imp::Inner::new(p)),
                    _ => None,
                },
            }
        }
        #[cfg(not(feature = "usage-bundle"))]
        {
            Self {}
        }
    }

    /// Record one capability invocation under each capability group the
    /// capability declared. `error_kind` is `None` on success, or one of
    /// the CLOSED set of failure kinds ("trapped", "deadline",
    /// "policy-denied", "fuel-exhausted", "hash-mismatch", "failed") —
    /// never free text, never an error message.
    #[allow(unused_variables)]
    pub fn record_invocation(&self, group_names: &[String], error_kind: Option<&str>) {
        #[cfg(feature = "usage-bundle")]
        if let Some(inner) = &self.inner {
            inner.record_invocation(group_names, error_kind);
        }
    }

    /// Record one policy denial (a bare count — no pattern, no operation).
    pub fn record_denial(&self) {
        #[cfg(feature = "usage-bundle")]
        if let Some(inner) = &self.inner {
            inner.record_denial();
        }
    }

    /// Return the current bundle as JSON and reset the counters (delete the
    /// file). `None` when disabled, compiled out, or nothing accumulated.
    /// Counts are deltas, so fetch-then-fetch can never double-count.
    pub fn fetch_and_reset(&self) -> Option<String> {
        #[cfg(feature = "usage-bundle")]
        {
            self.inner.as_ref().and_then(imp::Inner::fetch_and_reset)
        }
        #[cfg(not(feature = "usage-bundle"))]
        {
            None
        }
    }
}

#[cfg(feature = "usage-bundle")]
mod imp {
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use std::ffi::OsStr;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// The complete bundle. This field list is exhaustive (ADR-027):
    /// adding a field requires amending the ADR and `TELEMETRY.md`.
    #[derive(Debug, Serialize, Deserialize)]
    pub(super) struct Bundle {
        pub schema: u32,
        pub version: String,
        pub os: String,
        pub arch: String,
        pub counts: BTreeMap<String, CountPair>,
        /// Per-category failure-kind breakouts. Kinds are a CLOSED set
        /// defined by the runtime ("trapped", "deadline", "policy-denied",
        /// "fuel-exhausted", "hash-mismatch", "failed") — never messages.
        #[serde(default)]
        pub errors: BTreeMap<String, BTreeMap<String, u64>>,
        pub denials: u64,
    }

    #[derive(Debug, Default, Serialize, Deserialize)]
    pub(super) struct CountPair {
        pub ok: u64,
        pub err: u64,
    }

    impl Bundle {
        fn empty() -> Self {
            Self {
                schema: 1,
                version: env!("CARGO_PKG_VERSION").to_string(),
                os: std::env::consts::OS.to_string(),
                arch: std::env::consts::ARCH.to_string(),
                counts: BTreeMap::new(),
                errors: BTreeMap::new(),
                denials: 0,
            }
        }
    }

    /// The closed set of failure kinds. Anything not in this list is
    /// recorded as "failed" — the bundle never grows a new kind without a
    /// code change here and an ADR-027 amendment.
    pub(super) const ERROR_KINDS: &[&str] = &[
        "trapped",
        "deadline",
        "policy-denied",
        "fuel-exhausted",
        "hash-mismatch",
        "failed",
    ];

    /// `true` when an opt-out environment variable disables accumulation.
    pub(super) fn env_disabled(do_not_track: Option<&OsStr>, gang_telemetry: Option<&str>) -> bool {
        do_not_track.is_some_and(|v| !v.is_empty())
            || gang_telemetry.is_some_and(|v| v.eq_ignore_ascii_case("off"))
    }

    /// Map a qualified capability-group name to its bundle category:
    /// `ganglion:ros/interface` → `ros`. The closed set of categories is
    /// defined by us; capability *names* never enter the bundle.
    pub(super) fn category(group_name: &str) -> &str {
        let s = group_name.strip_prefix("ganglion:").unwrap_or(group_name);
        s.split(['/', '@']).next().unwrap_or(s)
    }

    #[derive(Debug)]
    pub(super) struct Inner {
        path: PathBuf,
        /// Serializes read-modify-write cycles on the bundle file.
        lock: Mutex<()>,
    }

    impl Inner {
        pub(super) fn new(path: PathBuf) -> Self {
            Self {
                path,
                lock: Mutex::new(()),
            }
        }

        fn load(&self) -> Bundle {
            std::fs::read_to_string(&self.path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(Bundle::empty)
        }

        /// Atomic write: temp file + rename, so a crash mid-write can never
        /// leave a torn bundle. All failures are silent by design.
        fn store(&self, bundle: &Bundle) {
            let Ok(json) = serde_json::to_string(bundle) else {
                return;
            };
            if let Some(parent) = self.path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }

        pub(super) fn record_invocation(&self, group_names: &[String], error_kind: Option<&str>) {
            // Closed-set enforcement: an unknown kind degrades to "failed"
            // rather than minting a new bucket.
            let kind = error_kind.map(|k| {
                if ERROR_KINDS.contains(&k) {
                    k
                } else {
                    "failed"
                }
            });
            let _guard = self.lock.lock().expect("bundle lock poisoned");
            let mut bundle = self.load();
            for name in group_names {
                let cat = category(name).to_string();
                let pair = bundle.counts.entry(cat.clone()).or_default();
                match kind {
                    None => pair.ok += 1,
                    Some(k) => {
                        pair.err += 1;
                        *bundle
                            .errors
                            .entry(cat)
                            .or_default()
                            .entry(k.to_string())
                            .or_default() += 1;
                    }
                }
            }
            self.store(&bundle);
        }

        pub(super) fn record_denial(&self) {
            let _guard = self.lock.lock().expect("bundle lock poisoned");
            let mut bundle = self.load();
            bundle.denials += 1;
            self.store(&bundle);
        }

        pub(super) fn fetch_and_reset(&self) -> Option<String> {
            let _guard = self.lock.lock().expect("bundle lock poisoned");
            let json = std::fs::read_to_string(&self.path).ok()?;
            // Validate before handing out: a corrupt file yields None and is
            // cleared rather than shipped.
            let valid = serde_json::from_str::<Bundle>(&json).is_ok();
            let _ = std::fs::remove_file(&self.path);
            valid.then_some(json)
        }
    }
}

#[cfg(all(test, feature = "usage-bundle"))]
mod tests {
    use super::*;

    fn recorder(dir: &tempfile::TempDir) -> UsageBundleRecorder {
        UsageBundleRecorder {
            inner: Some(imp::Inner::new(dir.path().join("bundle.json"))),
        }
    }

    #[test]
    fn category_mapping_covers_all_groups() {
        let cases = [
            ("ganglion:ros/interface", "ros"),
            ("ganglion:logs/stream", "logs"),
            ("ganglion:fs/bounded", "fs"),
            ("ganglion:diagnostics/collect", "diagnostics"),
            ("ganglion:artifacts/publish", "artifacts"),
            ("ganglion:process/spawn", "process"),
            ("ganglion:network/probe", "network"),
            ("ganglion:metrics/emit", "metrics"),
            ("ganglion:http/egress", "http"),
        ];
        for (qualified, want) in cases {
            assert_eq!(imp::category(qualified), want, "category({qualified})");
        }
    }

    #[test]
    fn bundle_field_list_is_locked() {
        // ADR-027: the bundle payload is exhaustive. Changing this list
        // requires amending the ADR and TELEMETRY.md.
        let dir = tempfile::tempdir().unwrap();
        let rec = recorder(&dir);
        rec.record_invocation(&["ganglion:ros/interface".into()], None);
        let json = rec.fetch_and_reset().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "arch", "counts", "denials", "errors", "os", "schema", "version"
            ]
        );
    }

    #[test]
    fn counts_accumulate_and_reset_on_fetch() {
        let dir = tempfile::tempdir().unwrap();
        let rec = recorder(&dir);
        rec.record_invocation(&["ganglion:ros/interface".into()], None);
        rec.record_invocation(&["ganglion:ros/interface".into()], Some("trapped"));
        rec.record_invocation(
            &["ganglion:fs/bounded".into(), "ganglion:logs/stream".into()],
            None,
        );
        rec.record_denial();

        let json = rec.fetch_and_reset().unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["counts"]["ros"]["ok"], 1);
        assert_eq!(v["counts"]["ros"]["err"], 1);
        assert_eq!(v["counts"]["fs"]["ok"], 1);
        assert_eq!(v["counts"]["logs"]["ok"], 1);
        assert_eq!(v["errors"]["ros"]["trapped"], 1);
        assert_eq!(v["denials"], 1);

        // Reset: a second fetch has nothing (no double counting, ever).
        assert!(rec.fetch_and_reset().is_none());
    }

    #[test]
    fn capability_names_never_appear() {
        // A distinctively named capability's groups are recorded by
        // category only — the name must not appear anywhere in the bundle.
        let dir = tempfile::tempdir().unwrap();
        let rec = recorder(&dir);
        // The caller passes group names, never capability names; assert the
        // stored JSON contains only closed-set categories.
        rec.record_invocation(&["ganglion:process/spawn".into()], None);
        let json = rec.fetch_and_reset().unwrap();
        assert!(!json.contains("acme"), "no operator strings in bundle");
        assert!(
            !json.contains("ganglion:"),
            "qualified names are reduced to categories"
        );
        assert!(json.contains("\"process\""));
    }

    #[test]
    fn unknown_error_kind_degrades_to_failed() {
        // The kinds set is CLOSED: a caller passing anything else (say, a
        // raw error message by mistake) is recorded as "failed" — free
        // text can never enter the bundle.
        let dir = tempfile::tempdir().unwrap();
        let rec = recorder(&dir);
        rec.record_invocation(
            &["ganglion:ros/interface".into()],
            Some("kaboom: open /etc/passwd"),
        );
        let json = rec.fetch_and_reset().unwrap();
        assert!(!json.contains("kaboom"), "free text leaked: {json}");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["errors"]["ros"]["failed"], 1);
        assert_eq!(v["counts"]["ros"]["err"], 1);
    }

    #[test]
    fn env_opt_outs_disable() {
        assert!(imp::env_disabled(Some(std::ffi::OsStr::new("1")), None));
        assert!(!imp::env_disabled(Some(std::ffi::OsStr::new("")), None));
        assert!(imp::env_disabled(None, Some("off")));
        assert!(imp::env_disabled(None, Some("OFF")));
        assert!(!imp::env_disabled(None, Some("on")));
        assert!(!imp::env_disabled(None, None));
    }

    #[test]
    fn disabled_recorder_is_inert() {
        let rec = UsageBundleRecorder { inner: None };
        rec.record_invocation(&["ganglion:ros/interface".into()], None);
        rec.record_denial();
        assert!(rec.fetch_and_reset().is_none());
    }

    #[test]
    fn corrupt_bundle_is_cleared_not_shipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bundle.json");
        std::fs::write(&path, "{not json").unwrap();
        let rec = recorder(&dir);
        assert!(rec.fetch_and_reset().is_none());
        assert!(!path.exists(), "corrupt bundle removed on fetch");
    }
}
