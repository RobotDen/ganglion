//! `gang doctor --profile-out` — turn the customer's link into a CI test case.
//!
//! When a customer says "it's slow at their site", the link conditions
//! evaporate the moment the support call ends. This module measures the
//! *actual* link (TCP connect RTT and failure rate against the configured
//! relay, sampled over N probes) and emits a **deterministic** degraded-link
//! profile in the `test-harness/degraded-link` fixture format — so the
//! customer network becomes a replayable `run-matrix.sh` case instead of an
//! anecdote.
//!
//! Determinism contract (same as the gate profiles): the emitted shape uses
//! only fixed netem delay (measured RTT split evenly across both directions,
//! matching the `high-latency` fixture convention) and iptables
//! statistic-nth loss (nearest every-Nth to the measured failure rate).
//! Throughput is *not* measurable from a handshake probe, so rate caps are
//! only emitted when the operator supplies them (`--uplink-kbit` /
//! `--downlink-kbit`, e.g. from a site speed test) — the profile header says
//! which numbers were measured and which were supplied.
//!
//! Measurement is a thin blocking wrapper over `TcpStream::connect_timeout`;
//! everything downstream of the raw samples (statistics, synthesis,
//! rendering, name sanitization) is pure and unit-tested.

use std::io::Write as _;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::{Duration, Instant};

/// Gap between probe connects: long enough to avoid SYN-burst throttling
/// skewing the tail samples, short enough that 20 samples finish in ~2s.
const PROBE_GAP: Duration = Duration::from_millis(75);

/// Raw link measurement: connect RTT samples + failure count against one
/// TCP target.
#[derive(Debug, Clone)]
pub struct LinkMeasurement {
    /// `host:port` actually probed.
    pub target: String,
    /// Total probes attempted.
    pub samples: usize,
    /// Probes that failed to connect within the timeout.
    pub failures: usize,
    /// Connect RTTs of the successful probes, milliseconds.
    pub rtts_ms: Vec<f64>,
}

/// A successful connect that took at least this much longer than the median
/// almost certainly lost its first SYN: Linux's initial SYN retransmission
/// timeout is 1s, so a retransmitted handshake lands ~1000ms above the true
/// path RTT. 800ms leaves margin for RTT variance while staying far above
/// any plausible jitter. (#38)
const SYN_RETRANSMIT_THRESHOLD_MS: f64 = 800.0;

impl LinkMeasurement {
    /// Median connect RTT, or `None` when every probe failed. The median is
    /// robust to retransmit-inflated outliers as long as fewer than half the
    /// samples retransmitted, so the delay estimate stays clean on lossy
    /// links.
    pub fn median_rtt_ms(&self) -> Option<f64> {
        percentile(&self.rtts_ms, 50.0)
    }

    /// Successful connects that almost certainly lost their first SYN:
    /// landed ≥ [`SYN_RETRANSMIT_THRESHOLD_MS`] above the median. This is
    /// how light (1–5%) loss shows up in a connect probe — the kernel
    /// retries and the connect *succeeds*, just ~1s late — so counting only
    /// hard failures misses it entirely. (#38)
    pub fn syn_retransmits(&self) -> usize {
        match self.median_rtt_ms() {
            Some(median) => self
                .rtts_ms
                .iter()
                .filter(|&&rtt| rtt >= median + SYN_RETRANSMIT_THRESHOLD_MS)
                .count(),
            None => 0,
        }
    }

    /// Total loss events: hard connect failures plus retransmit-timing
    /// detections.
    pub fn loss_events(&self) -> usize {
        self.failures + self.syn_retransmits()
    }

    /// Spread: p90 − p10 of the successful samples (0 for < 2 samples).
    pub fn spread_ms(&self) -> f64 {
        match (
            percentile(&self.rtts_ms, 90.0),
            percentile(&self.rtts_ms, 10.0),
        ) {
            (Some(hi), Some(lo)) => (hi - lo).max(0.0),
            _ => 0.0,
        }
    }

    /// Measured loss rate mapped to the deterministic `statistic --mode
    /// nth` form: drop every Nth packet. Loss events are hard failures plus
    /// SYN-retransmit detections (#38). `None` when nothing was lost (no
    /// loss rule) or when *every* probe hard-failed (that is an unreachable
    /// target, not a lossy link — synthesizing "drop every packet" would be
    /// nonsense).
    pub fn loss_every_nth(&self) -> Option<u32> {
        let events = self.loss_events();
        if events == 0 || self.failures >= self.samples {
            return None;
        }
        // events/samples ≈ 1/N  →  N = round(samples/events), floor 2
        // (N=1 means "every packet": excluded above), cap at samples.
        let n = (self.samples as f64 / events as f64).round() as u32;
        Some(n.clamp(2, self.samples as u32))
    }
}

/// Linear-interpolated percentile of an unsorted sample set.
fn percentile(samples: &[f64], p: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut v = samples.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).expect("no NaN rtts"));
    let rank = (p / 100.0) * (v.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let frac = rank - lo as f64;
    Some(v[lo] + (v[hi] - v[lo]) * frac)
}

/// Probe `host:port` with `samples` sequential TCP connects. Blocking; call
/// via `spawn_blocking`.
pub fn measure_tcp(host: &str, port: u16, samples: usize, timeout: Duration) -> LinkMeasurement {
    let target = format!("{host}:{port}");
    let mut rtts_ms = Vec::with_capacity(samples);
    let mut failures = 0usize;
    for i in 0..samples {
        if i > 0 {
            std::thread::sleep(PROBE_GAP);
        }
        // Re-resolve each round so DNS failures count as link failures too.
        let addr = (host, port)
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next());
        let started = Instant::now();
        match addr {
            Some(addr) => match TcpStream::connect_timeout(&addr, timeout) {
                Ok(_) => rtts_ms.push(started.elapsed().as_secs_f64() * 1000.0),
                Err(_) => failures += 1,
            },
            None => failures += 1,
        }
    }
    LinkMeasurement {
        target,
        samples,
        failures,
        rtts_ms,
    }
}

/// Synthesized deterministic profile parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileParams {
    /// Sanitized profile name (`[a-z0-9-]`).
    pub name: String,
    /// Fixed one-way delay per side, ms (measured RTT split evenly, floor 1
    /// when a measurement exists).
    pub delay_each_way_ms: Option<u32>,
    /// Deterministic loss: drop every Nth packet, both directions.
    pub loss_every_nth: Option<u32>,
    /// Operator-supplied robot uplink cap (tbf on robot egress).
    pub uplink_kbit: Option<u32>,
    /// Operator-supplied robot downlink cap (tbf on operator egress).
    pub downlink_kbit: Option<u32>,
}

/// Lowercase, map anything outside `[a-z0-9]` to `-`, squeeze repeats, trim.
/// Falls back to `"site"` for names that sanitize to nothing.
pub fn sanitize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_dash = true; // suppress leading dash
    for c in raw.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "site".to_string()
    } else {
        out
    }
}

/// Derive deterministic profile parameters from a measurement + operator
/// supplied rate caps.
pub fn synthesize(
    m: &LinkMeasurement,
    name: &str,
    uplink_kbit: Option<u32>,
    downlink_kbit: Option<u32>,
) -> ProfileParams {
    ProfileParams {
        name: sanitize_name(name),
        delay_each_way_ms: m
            .median_rtt_ms()
            .map(|rtt| ((rtt / 2.0).round() as u32).max(1)),
        loss_every_nth: m.loss_every_nth(),
        uplink_kbit,
        downlink_kbit,
    }
}

/// Build the robot-side shape command string (netem delay + nth-loss + tbf
/// uplink cap). Empty string when nothing applies.
fn robot_shape(p: &ProfileParams) -> String {
    let mut cmds: Vec<String> = Vec::new();
    match (p.delay_each_way_ms, p.uplink_kbit) {
        (Some(d), Some(rate)) => {
            // tbf root with nested netem — the `asymmetric` fixture pattern.
            cmds.push(format!(
                "tc qdisc add dev eth0 root handle 1: tbf rate {rate}kbit burst 16kb \
                 latency 400ms && tc qdisc add dev eth0 parent 1:1 handle 10: netem \
                 delay {d}ms"
            ));
        }
        (Some(d), None) => cmds.push(format!("tc qdisc add dev eth0 root netem delay {d}ms")),
        (None, Some(rate)) => cmds.push(format!(
            "tc qdisc add dev eth0 root tbf rate {rate}kbit burst 16kb latency 400ms"
        )),
        (None, None) => {}
    }
    if let Some(n) = p.loss_every_nth {
        cmds.push(format!(
            "iptables -A OUTPUT -m statistic --mode nth --every {n} --packet 0 -j DROP && \
             iptables -A INPUT -m statistic --mode nth --every {n} --packet 0 -j DROP"
        ));
    }
    cmds.join(" && ")
}

/// Build the operator-side shape (matching delay half + downlink cap).
fn operator_shape(p: &ProfileParams) -> String {
    match (p.delay_each_way_ms, p.downlink_kbit) {
        (Some(d), Some(rate)) => format!(
            "tc qdisc add dev eth0 root handle 1: tbf rate {rate}kbit burst 16kb \
             latency 400ms && tc qdisc add dev eth0 parent 1:1 handle 10: netem delay {d}ms"
        ),
        (Some(d), None) => format!("tc qdisc add dev eth0 root netem delay {d}ms"),
        (None, Some(rate)) => {
            format!("tc qdisc add dev eth0 root tbf rate {rate}kbit burst 16kb latency 400ms")
        }
        (None, None) => String::new(),
    }
}

/// Render the harness-format `.profile` file with measurement provenance.
pub fn render_profile(p: &ProfileParams, m: &LinkMeasurement, generated_at: &str) -> String {
    let mut desc_bits: Vec<String> = Vec::new();
    if let Some(d) = p.delay_each_way_ms {
        desc_bits.push(format!("{}ms RTT ({d}ms each way)", d * 2));
    }
    if let Some(n) = p.loss_every_nth {
        desc_bits.push(format!("~{:.1}% loss (every {n}th pkt)", 100.0 / n as f64));
    }
    if let Some(u) = p.uplink_kbit {
        desc_bits.push(format!("uplink {u}kbit"));
    }
    if let Some(dl) = p.downlink_kbit {
        desc_bits.push(format!("downlink {dl}kbit"));
    }
    if desc_bits.is_empty() {
        desc_bits.push("measured clean link, no impairment".to_string());
    }
    let desc = desc_bits.join(", ");

    let median = m
        .median_rtt_ms()
        .map(|v| format!("{v:.1}ms"))
        .unwrap_or_else(|| "n/a".to_string());
    let rates_line = match (p.uplink_kbit, p.downlink_kbit) {
        (None, None) => "# rates: not measurable from a handshake probe; none supplied".to_string(),
        _ => "# rates: OPERATOR-SUPPLIED (--uplink-kbit/--downlink-kbit), not measured".to_string(),
    };
    // Honest about the two structural limits of a connect-probe measurement:
    // resolution of the loss estimate, and which leg of the path was seen.
    let loss_line = format!(
        "# loss: {fails} hard failure(s) + {retx} SYN-retransmit detection(s) \
         (connect ≥800ms over median)\n\
         # over {total} samples — resolves nothing finer than ~{res:.1}%; still a \
         proxy, not a packet count",
        fails = m.failures,
        retx = m.syn_retransmits(),
        total = m.samples,
        res = 100.0 / m.samples as f64,
    );
    let leg_line = "# path: ONE leg (this host -> target). The full operator<->robot path \
                    crosses two\n\
                    # relay legs; run doctor on the robot's network to capture the \
                    impaired side.";

    format!(
        "# Site profile generated by `gang doctor --profile-out` — {generated_at}\n\
         # measured against: {target} ({ok}/{total} probes ok)\n\
         # rtt: median {median}, p90-p10 spread {spread:.1}ms (spread is NOT \
         reproduced: fixed delay only,\n\
         # per the gate determinism contract; jittered latency belongs to the \
         chaos run)\n\
         {loss_line}\n\
         {rates_line}\n\
         {leg_line}\n\
         # replay: ./run-matrix.sh --profile-file <this file>\n\
         PROFILE_NAME=\"{name}\"\n\
         PROFILE_DESC=\"{desc}\"\n\
         PROFILE_CLASS=\"site\"\n\
         ROBOT_SHAPE='{robot}'\n\
         OPERATOR_SHAPE='{operator}'\n",
        target = m.target,
        ok = m.samples - m.failures,
        total = m.samples,
        spread = m.spread_ms(),
        name = p.name,
        robot = robot_shape(p),
        operator = operator_shape(p),
    )
}

/// Measure, synthesize, render, and write the profile file. Returns the
/// rendered description line for the CLI to echo. Blocking measurement runs
/// on the caller's thread — callers wrap in `spawn_blocking`.
#[allow(clippy::too_many_arguments)] // one call site, mirrors the CLI flags 1:1
pub fn write_profile(
    host: &str,
    port: u16,
    samples: usize,
    timeout: Duration,
    name: &str,
    uplink_kbit: Option<u32>,
    downlink_kbit: Option<u32>,
    out_path: &Path,
) -> anyhow::Result<(LinkMeasurement, ProfileParams)> {
    let m = measure_tcp(host, port, samples, timeout);
    if m.failures >= m.samples {
        anyhow::bail!(
            "all {} probes to {} failed — cannot profile an unreachable link \
             (fix reachability first; see the doctor report above)",
            m.samples,
            m.target
        );
    }
    let p = synthesize(&m, name, uplink_kbit, downlink_kbit);
    let generated_at = chrono_free_timestamp();
    let rendered = render_profile(&p, &m, &generated_at);
    let mut f = std::fs::File::create(out_path)?;
    f.write_all(rendered.as_bytes())?;
    Ok((m, p))
}

/// RFC3339-ish UTC timestamp without pulling in chrono.
fn chrono_free_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days-to-civil conversion (Howard Hinnant's algorithm).
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meas(rtts: &[f64], failures: usize) -> LinkMeasurement {
        LinkMeasurement {
            target: "relay.example:443".into(),
            samples: rtts.len() + failures,
            failures,
            rtts_ms: rtts.to_vec(),
        }
    }

    #[test]
    fn median_and_spread() {
        let m = meas(&[10.0, 20.0, 30.0, 40.0, 50.0], 0);
        assert_eq!(m.median_rtt_ms(), Some(30.0));
        // p90 = 46, p10 = 14 with linear interpolation.
        assert!((m.spread_ms() - 32.0).abs() < 1e-9);
        assert_eq!(meas(&[], 5).median_rtt_ms(), None);
    }

    #[test]
    fn loss_maps_to_every_nth() {
        // 1 failure in 20 → every 20th.
        assert_eq!(meas(&[10.0; 19], 1).loss_every_nth(), Some(20));
        // 7 in 20 → round(20/7) = 3.
        assert_eq!(meas(&[10.0; 13], 7).loss_every_nth(), Some(3));
        // No failures → no loss rule.
        assert_eq!(meas(&[10.0; 20], 0).loss_every_nth(), None);
        // All failed → unreachable, not lossy.
        assert_eq!(meas(&[], 20).loss_every_nth(), None);
        // Floor at every-2nd even for absurd rates.
        assert_eq!(meas(&[10.0], 9).loss_every_nth(), Some(2));
    }

    #[test]
    fn syn_retransmits_detected_from_timing() {
        // 38 fast connects at ~30ms, 2 that lost their first SYN and landed
        // ~1s late: 0 hard failures but 2 loss events → every-20th (5%).
        let mut rtts = vec![30.0; 38];
        rtts.push(1032.0);
        rtts.push(1029.5);
        let m = meas(&rtts, 0);
        assert_eq!(m.syn_retransmits(), 2);
        assert_eq!(m.loss_events(), 2);
        assert_eq!(m.loss_every_nth(), Some(20));
        // The delay estimate is NOT skewed by the retransmit outliers.
        assert_eq!(m.median_rtt_ms(), Some(30.0));
    }

    #[test]
    fn retransmits_and_failures_combine() {
        // 1 hard failure + 1 retransmit over 40 samples → 2 events → every-20th.
        let mut rtts = vec![25.0; 38];
        rtts.push(1050.0);
        let m = meas(&rtts, 1);
        assert_eq!(m.loss_events(), 2);
        assert_eq!(m.loss_every_nth(), Some(20));
    }

    #[test]
    fn high_latency_link_is_not_misread_as_loss() {
        // Satellite-ish 600ms RTT with ±40ms jitter: nothing lands 800ms
        // over the median, so no retransmit detections.
        let rtts: Vec<f64> = (0..40).map(|i| 600.0 + (i % 5) as f64 * 10.0).collect();
        let m = meas(&rtts, 0);
        assert_eq!(m.syn_retransmits(), 0);
        assert_eq!(m.loss_every_nth(), None);
    }

    #[test]
    fn bench_synthesis_pipeline_stays_fast() {
        // Perf tripwire: statistics + synthesis + render for a 40-sample
        // measurement is pure CPU and must stay trivially cheap next to the
        // ~3s of wire time the probes themselves take.
        let rtts: Vec<f64> = (0..40).map(|i| 30.0 + (i % 7) as f64).collect();
        let m = meas(&rtts, 1);
        const ITERS: u32 = 10_000;
        let start = Instant::now();
        for _ in 0..ITERS {
            let p = synthesize(&m, "Acme East", Some(384), None);
            let _ = render_profile(&p, &m, "2026-08-14T00:00:00Z");
        }
        let per_op = start.elapsed() / ITERS;
        eprintln!("link_profile synth+render: {per_op:?}/op ({ITERS} iters)");
        assert!(
            per_op < std::time::Duration::from_micros(100),
            "synthesis pipeline regressed to {per_op:?}/op (ceiling 100µs)"
        );
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_name("Acme Plant #3 (east)"), "acme-plant-3-east");
        assert_eq!(sanitize_name("--weird__stuff--"), "weird-stuff");
        assert_eq!(sanitize_name("???"), "site");
    }

    #[test]
    fn synthesis_splits_rtt_and_floors_delay() {
        let p = synthesize(&meas(&[84.0, 86.0, 85.0], 0), "site", None, None);
        assert_eq!(p.delay_each_way_ms, Some(43)); // round(85/2)
        let p = synthesize(&meas(&[1.0, 1.2], 0), "site", None, None);
        assert_eq!(p.delay_each_way_ms, Some(1)); // floor 1
    }

    #[test]
    fn shapes_follow_fixture_patterns() {
        // Delay only: plain netem both sides, like high-latency.
        let p = ProfileParams {
            name: "s".into(),
            delay_each_way_ms: Some(40),
            loss_every_nth: None,
            uplink_kbit: None,
            downlink_kbit: None,
        };
        assert_eq!(
            robot_shape(&p),
            "tc qdisc add dev eth0 root netem delay 40ms"
        );
        assert_eq!(
            operator_shape(&p),
            "tc qdisc add dev eth0 root netem delay 40ms"
        );

        // Uplink cap + delay: tbf root with nested netem, like asymmetric.
        let p = ProfileParams {
            delay_each_way_ms: Some(30),
            uplink_kbit: Some(192),
            ..p
        };
        assert!(robot_shape(&p).contains("tbf rate 192kbit"));
        assert!(robot_shape(&p).contains("parent 1:1"));
        assert!(robot_shape(&p).contains("netem delay 30ms"));

        // Loss rides iptables statistic-nth in both directions.
        let p = ProfileParams {
            loss_every_nth: Some(33),
            ..p
        };
        assert!(robot_shape(&p).contains("--every 33"));
        assert!(robot_shape(&p).contains("-A OUTPUT"));
        assert!(robot_shape(&p).contains("-A INPUT"));
    }

    #[test]
    fn rendered_profile_is_sourceable_fixture() {
        let m = meas(&[80.0, 90.0, 100.0], 1);
        let p = synthesize(&m, "Acme East", None, None);
        let text = render_profile(&p, &m, "2026-08-14T00:00:00Z");
        assert!(text.contains("PROFILE_NAME=\"acme-east\""));
        assert!(text.contains("PROFILE_CLASS=\"site\""));
        assert!(text.contains("ROBOT_SHAPE='tc qdisc"));
        assert!(text.contains("measured against: relay.example:443 (3/4 probes ok)"));
        assert!(text.contains("not measurable from a handshake probe"));
        // The two structural-limit caveats are always present.
        assert!(text.contains("SYN-retransmit detection"));
        assert!(text.contains("still a proxy"));
        assert!(text.contains("ONE leg"));
        // No single quotes inside the shape strings (they are single-quoted).
        for line in text.lines().filter(|l| l.contains("_SHAPE='")) {
            let inner = line.split_once("='").unwrap().1;
            assert!(!inner.trim_end_matches('\'').contains('\''));
        }
    }

    #[test]
    fn clean_link_renders_empty_shapes_and_says_so() {
        let m = meas(&[1.0, 1.1, 0.9], 0);
        let mut p = synthesize(&m, "lab", None, None);
        // A 1ms floor delay still emits; force the truly-clean case.
        p.delay_each_way_ms = None;
        let text = render_profile(&p, &m, "2026-08-14T00:00:00Z");
        assert!(text.contains("ROBOT_SHAPE=''"));
        assert!(text.contains("measured clean link, no impairment"));
    }

    #[test]
    fn timestamp_shape() {
        let ts = chrono_free_timestamp();
        assert_eq!(ts.len(), 20);
        assert!(ts.ends_with('Z'));
        assert!(ts.starts_with("20"));
    }
}
