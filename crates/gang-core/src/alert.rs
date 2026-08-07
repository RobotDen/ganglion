//! Minimal alerting primitive: metric → threshold → webhook.
//!
//! This is deliberately the useful 20% of alerting, not an incident-management
//! platform. A rule names a metric, a comparator, and a threshold; when a
//! sampled value breaches it, the rule fires a single structured webhook
//! (a Slack-incoming-webhook–compatible JSON payload) and then stays quiet for
//! a cooldown so one flapping metric can't storm the channel. Everything here
//! is pure and unit-tested; delivery (an HTTPS POST) lives in the CLI so the
//! core stays dependency-free.
//!
//! Richer routing — incident-management integrations, escalation, SLAs — is out
//! of scope by design and belongs to the commercial layer, not the open core.

use serde::{Deserialize, Serialize};

/// How a sampled value is compared against a rule's threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparator {
    /// value > threshold
    Gt,
    /// value >= threshold
    Ge,
    /// value < threshold
    Lt,
    /// value <= threshold
    Le,
}

impl Comparator {
    /// Evaluate `value <cmp> threshold`.
    pub fn compare(self, value: f64, threshold: f64) -> bool {
        match self {
            Comparator::Gt => value > threshold,
            Comparator::Ge => value >= threshold,
            Comparator::Lt => value < threshold,
            Comparator::Le => value <= threshold,
        }
    }

    /// The mathematical symbol, for human-readable messages.
    pub fn symbol(self) -> &'static str {
        match self {
            Comparator::Gt => ">",
            Comparator::Ge => ">=",
            Comparator::Lt => "<",
            Comparator::Le => "<=",
        }
    }
}

/// A single alerting rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertRule {
    /// Human name for the rule (shown in the alert).
    pub name: String,
    /// Metric name this rule watches.
    pub metric: String,
    /// Comparator applied as `value <cmp> threshold`.
    pub comparator: Comparator,
    /// Threshold value.
    pub threshold: f64,
    /// Minimum seconds between fires for this rule (anti-flap).
    #[serde(default)]
    pub cooldown_secs: u64,
}

impl AlertRule {
    /// Whether `value` breaches this rule.
    pub fn breached(&self, value: f64) -> bool {
        self.comparator.compare(value, self.threshold)
    }

    /// Whether the rule may fire now, given the last fire time (as a monotonic
    /// seconds counter the caller supplies) and the current time. A rule with
    /// no prior fire (`last_fired_secs = None`) may always fire.
    pub fn may_fire(&self, last_fired_secs: Option<u64>, now_secs: u64) -> bool {
        match last_fired_secs {
            None => true,
            Some(last) => now_secs.saturating_sub(last) >= self.cooldown_secs,
        }
    }

    /// A one-line human summary of a breach.
    pub fn summary(&self, value: f64) -> String {
        format!(
            "{}: {} = {} ({} {})",
            self.name,
            self.metric,
            value,
            self.comparator.symbol(),
            self.threshold
        )
    }
}

/// Build a Slack-incoming-webhook–compatible JSON payload for a breach.
///
/// Slack renders the top-level `text`; the `attachments` carry structured
/// fields that other generic webhook receivers can also read. The shape is a
/// plain JSON object, so any endpoint that accepts JSON works.
pub fn webhook_payload(rule: &AlertRule, value: f64) -> String {
    let text = format!(":rotating_light: Ganglion alert — {}", rule.summary(value));
    let payload = serde_json::json!({
        "text": text,
        "attachments": [{
            "color": "danger",
            "fields": [
                { "title": "rule", "value": rule.name, "short": true },
                { "title": "metric", "value": rule.metric, "short": true },
                { "title": "value", "value": value.to_string(), "short": true },
                {
                    "title": "condition",
                    "value": format!("{} {}", rule.comparator.symbol(), rule.threshold),
                    "short": true
                },
            ]
        }]
    });
    payload.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(cmp: Comparator, threshold: f64, cooldown: u64) -> AlertRule {
        AlertRule {
            name: "hot".into(),
            metric: "cpu_temp_c".into(),
            comparator: cmp,
            threshold,
            cooldown_secs: cooldown,
        }
    }

    #[test]
    fn comparators_evaluate() {
        assert!(Comparator::Gt.compare(90.0, 80.0));
        assert!(!Comparator::Gt.compare(80.0, 80.0));
        assert!(Comparator::Ge.compare(80.0, 80.0));
        assert!(Comparator::Lt.compare(1.0, 5.0));
        assert!(Comparator::Le.compare(5.0, 5.0));
    }

    #[test]
    fn breach_detection() {
        let r = rule(Comparator::Gt, 80.0, 0);
        assert!(r.breached(85.0));
        assert!(!r.breached(75.0));
    }

    #[test]
    fn cooldown_gates_firing() {
        let r = rule(Comparator::Gt, 80.0, 60);
        assert!(r.may_fire(None, 1000)); // first fire always allowed
        assert!(!r.may_fire(Some(1000), 1030)); // 30s < 60s cooldown
        assert!(r.may_fire(Some(1000), 1060)); // exactly at cooldown
        assert!(r.may_fire(Some(1000), 5000)); // well past
    }

    #[test]
    fn zero_cooldown_always_fires() {
        let r = rule(Comparator::Gt, 80.0, 0);
        assert!(r.may_fire(Some(1000), 1000));
    }

    #[test]
    fn payload_is_slack_compatible_json() {
        let r = rule(Comparator::Gt, 80.0, 0);
        let payload = webhook_payload(&r, 91.5);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(v["text"].as_str().unwrap().contains("cpu_temp_c"));
        assert_eq!(v["attachments"][0]["color"], "danger");
        assert_eq!(v["attachments"][0]["fields"][0]["title"], "rule");
        assert_eq!(v["attachments"][0]["fields"][2]["value"], "91.5");
    }

    #[test]
    fn summary_reads_clearly() {
        let r = rule(Comparator::Ge, 80.0, 0);
        assert_eq!(r.summary(80.0), "hot: cpu_temp_c = 80 (>= 80)");
    }
}
