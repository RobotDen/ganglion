use serde::{Deserialize, Serialize};

use crate::capability::{CapabilityGroup, RosAccess};
use crate::error::PolicyError;
use crate::identity::PeerId;

/// The policy engine evaluates capability declarations against active policy
/// at load time. Default-deny: anything not explicitly permitted is rejected.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Policy {
    /// Rules per capability group. If a group has no entry, it is denied.
    #[serde(default)]
    pub capability_rules: Vec<CapabilityRule>,

    /// Per-peer rules: which operators can deploy capabilities.
    #[serde(default)]
    pub peer_rules: Vec<PeerRule>,
}

/// A rule permitting a capability group and constraining its patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRule {
    /// Capability group name (e.g., "ganglion:ros/interface").
    pub group: String,
    /// Allowed patterns within this group.
    pub allowed_patterns: Vec<String>,
    /// Maximum access level (for ROS interface).
    #[serde(default)]
    pub max_access: Option<String>,
}

/// A rule governing what a given peer is authorized to deploy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerRule {
    /// Peer ID this rule applies to. "*" means any peer.
    pub peer_id: String,
    /// Whether this peer can deploy capabilities.
    pub can_deploy: bool,
    /// Optional: restrict to specific capability names.
    #[serde(default)]
    pub allowed_capabilities: Vec<String>,
}

impl Policy {
    /// Load policy from a TOML file.
    pub fn load(path: &std::path::Path) -> Result<Self, PolicyError> {
        if !path.exists() {
            return Err(PolicyError::PolicyNotFound(path.display().to_string()));
        }
        let contents =
            std::fs::read_to_string(path).map_err(|e| PolicyError::InvalidPolicy(e.to_string()))?;
        toml::from_str(&contents).map_err(|e| PolicyError::InvalidPolicy(e.to_string()))
    }

    /// Load policy from a TOML string.
    pub fn from_toml(toml_str: &str) -> Result<Self, PolicyError> {
        toml::from_str(toml_str).map_err(|e| PolicyError::InvalidPolicy(e.to_string()))
    }

    /// A permissive policy for development/testing. Allows everything.
    pub fn permissive() -> Self {
        Self {
            capability_rules: vec![
                CapabilityRule {
                    group: "ganglion:ros/interface".into(),
                    allowed_patterns: vec!["**".into()],
                    max_access: Some("read_write".into()),
                },
                CapabilityRule {
                    group: "ganglion:logs/stream".into(),
                    allowed_patterns: vec!["**".into()],
                    max_access: None,
                },
                CapabilityRule {
                    group: "ganglion:fs/bounded".into(),
                    allowed_patterns: vec!["/tmp/gang/**".into()],
                    max_access: None,
                },
                CapabilityRule {
                    group: "ganglion:diagnostics/collect".into(),
                    allowed_patterns: vec!["**".into()],
                    max_access: None,
                },
                CapabilityRule {
                    group: "ganglion:artifacts/publish".into(),
                    allowed_patterns: vec!["**".into()],
                    max_access: None,
                },
                CapabilityRule {
                    group: "ganglion:process/spawn".into(),
                    allowed_patterns: vec!["**".into()],
                    max_access: None,
                },
                CapabilityRule {
                    group: "ganglion:network/probe".into(),
                    allowed_patterns: vec!["**".into()],
                    max_access: None,
                },
                CapabilityRule {
                    group: "ganglion:metrics/emit".into(),
                    allowed_patterns: vec!["**".into()],
                    max_access: None,
                },
            ],
            peer_rules: vec![PeerRule {
                peer_id: "*".into(),
                can_deploy: true,
                allowed_capabilities: Vec::new(),
            }],
        }
    }

    /// Evaluate whether a set of declared capabilities is permitted by this policy.
    pub fn evaluate(
        &self,
        declared: &[CapabilityGroup],
        deployer: &PeerId,
    ) -> Result<(), PolicyError> {
        // Check peer authorization
        self.check_peer_authorized(deployer)?;

        // Check each declared capability against rules
        for cap in declared {
            self.check_capability_permitted(cap)?;
        }

        Ok(())
    }

    fn check_peer_authorized(&self, peer: &PeerId) -> Result<(), PolicyError> {
        if self.peer_rules.is_empty() {
            return Err(PolicyError::PeerNotAuthorized {
                peer: peer.to_string(),
            });
        }

        let authorized = self
            .peer_rules
            .iter()
            .any(|rule| rule.peer_id == "*" || rule.peer_id == peer.as_str())
            && self.peer_rules.iter().any(|rule| {
                (rule.peer_id == "*" || rule.peer_id == peer.as_str()) && rule.can_deploy
            });

        if !authorized {
            return Err(PolicyError::PeerNotAuthorized {
                peer: peer.to_string(),
            });
        }

        Ok(())
    }

    fn check_capability_permitted(&self, cap: &CapabilityGroup) -> Result<(), PolicyError> {
        let group_name = cap.name();

        let rule = self.capability_rules.iter().find(|r| r.group == group_name);

        let rule = match rule {
            Some(r) => r,
            None => {
                return Err(PolicyError::CapabilityDenied {
                    capability: cap.qualified_name(),
                });
            }
        };

        // Check patterns within the capability against the rule's allowed patterns
        match cap {
            CapabilityGroup::RosInterface { patterns, .. } => {
                for pattern in patterns {
                    if !pattern_matches_any(&pattern.pattern, &rule.allowed_patterns) {
                        return Err(PolicyError::PatternExceedsPolicy {
                            capability: group_name.into(),
                            pattern: pattern.pattern.clone(),
                        });
                    }
                    // Check access level
                    if pattern.access == RosAccess::ReadWrite
                        && let Some(max) = &rule.max_access
                        && max == "read_only"
                    {
                        return Err(PolicyError::PatternExceedsPolicy {
                            capability: group_name.into(),
                            pattern: format!(
                                "{} (read_write exceeds max read_only)",
                                pattern.pattern
                            ),
                        });
                    }
                }
            }
            CapabilityGroup::LogStream { patterns, .. } => {
                for pattern in patterns {
                    if !pattern_matches_any(pattern, &rule.allowed_patterns) {
                        return Err(PolicyError::PatternExceedsPolicy {
                            capability: group_name.into(),
                            pattern: pattern.clone(),
                        });
                    }
                }
            }
            CapabilityGroup::FsBounded { paths, .. } => {
                for path in paths {
                    if !pattern_matches_any(&path.pattern, &rule.allowed_patterns) {
                        return Err(PolicyError::PatternExceedsPolicy {
                            capability: group_name.into(),
                            pattern: path.pattern.clone(),
                        });
                    }
                }
            }
            CapabilityGroup::DiagnosticsCollect { .. } => {
                // No patterns to check beyond group presence.
            }
            CapabilityGroup::ArtifactsPublish { .. } => {
                // No patterns to check beyond group presence.
            }
            CapabilityGroup::ProcessSpawn {
                allowed_commands, ..
            } => {
                for cmd in allowed_commands {
                    if !pattern_matches_any(cmd, &rule.allowed_patterns) {
                        return Err(PolicyError::PatternExceedsPolicy {
                            capability: group_name.into(),
                            pattern: cmd.clone(),
                        });
                    }
                }
            }
            CapabilityGroup::NetworkProbe { .. } => {
                // No patterns to check beyond group presence.
            }
            CapabilityGroup::MetricsEmit { .. } => {
                // No patterns to check beyond group presence.
            }
        }

        Ok(())
    }
}

/// Why a specific request was denied, with enough context to name the
/// minimal widening that would permit it — and nothing more.
///
/// The point of this type is drift prevention: an unmanaged default-deny
/// policy erodes toward `allowed_patterns = ["**"]` because that is the only
/// edit a frustrated operator knows will work. When every denial names the
/// exact one-line rule that would permit exactly the denied request, the
/// narrow edit becomes the path of least resistance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DenialKind {
    /// The deploying peer has no matching `[[peer_rules]]` entry with
    /// `can_deploy = true`.
    PeerNotAuthorized,
    /// The capability group has no `[[capability_rules]]` entry at all.
    NoRuleForGroup,
    /// A rule exists for the group, but none of its `allowed_patterns`
    /// cover the requested pattern.
    PatternNotCovered {
        /// The patterns the existing rule does allow, for context.
        allowed: Vec<String>,
    },
    /// The pattern is covered, but the requested access level exceeds the
    /// rule's `max_access`.
    AccessExceedsMax {
        /// The rule's current ceiling.
        max: String,
    },
}

/// A structured account of one policy denial: what was requested, why it
/// was refused, and the smallest policy change that would permit it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenialReport {
    /// Capability group name (e.g. "ganglion:ros/interface"); empty for
    /// peer-authorization denials, which happen before any group is checked.
    pub group: String,
    /// The specific requested pattern that failed, when the group has
    /// pattern semantics.
    pub pattern: Option<String>,
    /// Requested access level, when the group distinguishes one.
    pub requested_access: Option<String>,
    /// The deploying peer.
    pub deployer: String,
    /// Why the request was refused.
    pub kind: DenialKind,
}

impl DenialReport {
    /// The minimal `policy.toml` addition that would permit exactly the
    /// denied request.
    pub fn suggested_rule(&self) -> String {
        match &self.kind {
            DenialKind::PeerNotAuthorized => format!(
                "[[peer_rules]]\npeer_id = \"{}\"\ncan_deploy = true",
                self.deployer
            ),
            DenialKind::NoRuleForGroup => {
                let pattern = self.pattern.as_deref().unwrap_or("**");
                let access_line = match self.requested_access.as_deref() {
                    Some(access) => format!("\nmax_access = \"{access}\""),
                    None => String::new(),
                };
                format!(
                    "[[capability_rules]]\ngroup = \"{}\"\nallowed_patterns = [\"{}\"]{}",
                    self.group, pattern, access_line
                )
            }
            // A rule for the group already exists, and the engine only reads
            // the FIRST rule per group — so the fix is extending that rule's
            // list, not adding a second block (which would be ignored).
            DenialKind::PatternNotCovered { allowed } => {
                let pattern = self.pattern.as_deref().unwrap_or("**");
                let mut patterns: Vec<String> =
                    allowed.iter().map(|p| format!("\"{p}\"")).collect();
                patterns.push(format!("\"{pattern}\""));
                format!(
                    "# in the EXISTING [[capability_rules]] for {}:\nallowed_patterns = [{}]",
                    self.group,
                    patterns.join(", ")
                )
            }
            DenialKind::AccessExceedsMax { .. } => format!(
                "# in the existing [[capability_rules]] for {}:\nmax_access = \"{}\"",
                self.group,
                self.requested_access.as_deref().unwrap_or("read_write")
            ),
        }
    }

    /// The one-line `gang policy allow` invocation that applies
    /// [`Self::suggested_rule`] with validation and an atomic write.
    pub fn suggested_command(&self) -> String {
        match &self.kind {
            DenialKind::PeerNotAuthorized => {
                format!("gang policy allow-peer {}", self.deployer)
            }
            _ => {
                let pattern = self.pattern.as_deref().unwrap_or("**");
                let access = match self.requested_access.as_deref() {
                    Some(access) => format!(" --access {access}"),
                    None => String::new(),
                };
                format!("gang policy allow {} \"{}\"{}", self.group, pattern, access)
            }
        }
    }

    /// The full operator-facing remedy: what was denied, why, and the two
    /// ways (command or snippet) to permit exactly that request.
    pub fn render(&self) -> String {
        let what = match (&self.pattern, &self.requested_access) {
            (Some(p), Some(a)) => format!("{} pattern \"{}\" ({})", self.group, p, a),
            (Some(p), None) => format!("{} pattern \"{}\"", self.group, p),
            _ if self.group.is_empty() => format!("deploys from peer {}", self.deployer),
            _ => self.group.clone(),
        };
        let why = match &self.kind {
            DenialKind::PeerNotAuthorized => {
                "this peer has no [[peer_rules]] entry with can_deploy = true".to_string()
            }
            DenialKind::NoRuleForGroup => {
                "no [[capability_rules]] entry exists for this group (default deny)".to_string()
            }
            DenialKind::PatternNotCovered { allowed } => {
                if allowed.is_empty() {
                    "the group's rule allows no patterns".to_string()
                } else {
                    format!(
                        "not covered by the group's allowed patterns ({})",
                        allowed
                            .iter()
                            .map(|p| format!("\"{p}\""))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            DenialKind::AccessExceedsMax { max } => {
                format!("requested access exceeds the rule's max_access = \"{max}\"")
            }
        };
        let snippet = self
            .suggested_rule()
            .lines()
            .map(|l| format!("    {l}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "policy denied: {what}\n  why: {why}\n  to permit exactly this, on the robot run:\n    {}\n  or add to policy.toml:\n{snippet}",
            self.suggested_command()
        )
    }
}

/// One `gang policy lint` finding: a rule whose breadth undermines the
/// default-deny posture, with the narrowing to consider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintFinding {
    /// Where in the policy the finding points (rule group or peer id).
    pub location: String,
    /// What is over-broad.
    pub finding: String,
    /// The narrowing to consider.
    pub suggestion: String,
}

/// What [`Policy::allow`] changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowOutcome {
    /// The request was already permitted; nothing changed.
    AlreadyAllowed,
    /// The pattern was appended to the group's existing rule.
    AddedPattern,
    /// The existing rule's `max_access` was raised.
    RaisedAccess,
    /// A new rule was created for the group.
    NewRule,
}

impl Policy {
    /// Analyze why `declared` would be denied for `deployer`, returning a
    /// structured report for the FIRST failure [`Policy::evaluate`] would
    /// hit, or `None` if the set is fully permitted.
    pub fn explain(&self, declared: &[CapabilityGroup], deployer: &PeerId) -> Option<DenialReport> {
        if self.check_peer_authorized(deployer).is_err() {
            return Some(DenialReport {
                group: String::new(),
                pattern: None,
                requested_access: None,
                deployer: deployer.to_string(),
                kind: DenialKind::PeerNotAuthorized,
            });
        }
        for cap in declared {
            if let Some(report) = self.explain_capability(cap, deployer) {
                return Some(report);
            }
        }
        None
    }

    fn explain_capability(&self, cap: &CapabilityGroup, deployer: &PeerId) -> Option<DenialReport> {
        let group_name = cap.name().to_string();
        let report = |pattern: Option<String>, access: Option<String>, kind: DenialKind| {
            Some(DenialReport {
                group: group_name.clone(),
                pattern,
                requested_access: access,
                deployer: deployer.to_string(),
                kind,
            })
        };

        let rule = self.capability_rules.iter().find(|r| r.group == group_name);
        // Collect (pattern, access) pairs the group requests, mirroring
        // check_capability_permitted's traversal order exactly.
        let requested: Vec<(String, Option<String>)> = match cap {
            CapabilityGroup::RosInterface { patterns, .. } => patterns
                .iter()
                .map(|p| {
                    let access = match p.access {
                        RosAccess::ReadWrite => "read_write",
                        RosAccess::ReadOnly => "read_only",
                    };
                    (p.pattern.clone(), Some(access.to_string()))
                })
                .collect(),
            CapabilityGroup::LogStream { patterns, .. } => {
                patterns.iter().map(|p| (p.clone(), None)).collect()
            }
            CapabilityGroup::FsBounded { paths, .. } => {
                paths.iter().map(|p| (p.pattern.clone(), None)).collect()
            }
            CapabilityGroup::ProcessSpawn {
                allowed_commands, ..
            } => allowed_commands.iter().map(|c| (c.clone(), None)).collect(),
            _ => Vec::new(),
        };

        let rule = match rule {
            Some(r) => r,
            None => {
                let (pattern, access) = requested
                    .first()
                    .map(|(p, a)| (Some(p.clone()), a.clone()))
                    .unwrap_or((None, None));
                return report(pattern, access, DenialKind::NoRuleForGroup);
            }
        };

        for (pattern, access) in &requested {
            if !pattern_matches_any(pattern, &rule.allowed_patterns) {
                return report(
                    Some(pattern.clone()),
                    access.clone(),
                    DenialKind::PatternNotCovered {
                        allowed: rule.allowed_patterns.clone(),
                    },
                );
            }
            if access.as_deref() == Some("read_write")
                && let Some(max) = &rule.max_access
                && max == "read_only"
            {
                return report(
                    Some(pattern.clone()),
                    access.clone(),
                    DenialKind::AccessExceedsMax { max: max.clone() },
                );
            }
        }
        None
    }

    /// Apply the minimal widening that permits `pattern` under `group`:
    /// append the pattern to the group's existing rule (raising `max_access`
    /// only if `access` requires it), or create a new single-pattern rule.
    /// Never touches any other rule.
    pub fn allow(&mut self, group: &str, pattern: &str, access: Option<&str>) -> AllowOutcome {
        if let Some(rule) = self.capability_rules.iter_mut().find(|r| r.group == group) {
            let pattern_covered = pattern_matches_any(pattern, &rule.allowed_patterns);
            let needs_access_raise =
                access == Some("read_write") && rule.max_access.as_deref() == Some("read_only");
            if pattern_covered && !needs_access_raise {
                return AllowOutcome::AlreadyAllowed;
            }
            if !pattern_covered {
                rule.allowed_patterns.push(pattern.to_string());
            }
            if needs_access_raise {
                rule.max_access = Some("read_write".into());
                return AllowOutcome::RaisedAccess;
            }
            return AllowOutcome::AddedPattern;
        }
        self.capability_rules.push(CapabilityRule {
            group: group.to_string(),
            allowed_patterns: vec![pattern.to_string()],
            max_access: access.map(|a| a.to_string()),
        });
        AllowOutcome::NewRule
    }

    /// Flag rules whose breadth undermines the default-deny posture. This is
    /// the drift tripwire: run it in CI or cron so the slide toward
    /// `allowed_patterns = ["**"]` is caught as a finding, not discovered in
    /// an incident.
    pub fn lint(&self) -> Vec<LintFinding> {
        let mut findings = Vec::new();
        for rule in &self.capability_rules {
            let wide = rule.allowed_patterns.iter().any(|p| p == "**");
            if wide {
                findings.push(LintFinding {
                    location: rule.group.clone(),
                    finding: "allowed_patterns contains \"**\" (matches everything)".into(),
                    suggestion: "list the specific patterns capabilities actually need; \
                                 `gang policy denials` shows what was recently requested"
                        .into(),
                });
            }
            if wide && rule.max_access.as_deref() == Some("read_write") {
                findings.push(LintFinding {
                    location: rule.group.clone(),
                    finding: "\"**\" combined with max_access = \"read_write\"".into(),
                    suggestion: "read-write on everything is the permissive dev profile; \
                                 cap it to read_only or scope the patterns"
                        .into(),
                });
            }
        }
        for rule in &self.peer_rules {
            if rule.peer_id == "*" && rule.can_deploy {
                findings.push(LintFinding {
                    location: "peer \"*\"".into(),
                    finding: "any trusted peer may deploy (peer_id = \"*\")".into(),
                    suggestion: "pin deploy rights to specific operator gang ids".into(),
                });
            }
        }
        findings
    }

    /// Serialize to pretty TOML (the `policy.toml` format).
    pub fn to_toml_pretty(&self) -> Result<String, PolicyError> {
        toml::to_string_pretty(self).map_err(|e| PolicyError::InvalidPolicy(e.to_string()))
    }
}

/// Check if a pattern is covered by any of the allowed patterns.
/// "**" matches everything. Otherwise, uses glob matching.
fn pattern_matches_any(requested: &str, allowed: &[String]) -> bool {
    for allowed_pattern in allowed {
        if allowed_pattern == "**" {
            return true;
        }
        if glob_match::glob_match(allowed_pattern, requested) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::AccessPattern;
    use crate::identity::Keypair;

    #[test]
    fn permissive_policy_allows_everything() {
        let policy = Policy::permissive();
        let peer = Keypair::generate().peer_id();

        let caps = vec![
            CapabilityGroup::RosInterface {
                version: "1.0".into(),
                patterns: vec![AccessPattern {
                    pattern: "/diagnostics".into(),
                    access: RosAccess::ReadOnly,
                }],
            },
            CapabilityGroup::DiagnosticsCollect {
                version: "1.0".into(),
            },
        ];

        assert!(policy.evaluate(&caps, &peer).is_ok());
    }

    #[test]
    fn empty_policy_denies_everything() {
        let policy = Policy::default();
        let peer = Keypair::generate().peer_id();

        let caps = vec![CapabilityGroup::DiagnosticsCollect {
            version: "1.0".into(),
        }];

        assert!(policy.evaluate(&caps, &peer).is_err());
    }

    #[test]
    fn pattern_based_ros_access() {
        let policy = Policy {
            capability_rules: vec![CapabilityRule {
                group: "ganglion:ros/interface".into(),
                allowed_patterns: vec!["/diagnostics/**".into(), "/rosout".into()],
                max_access: Some("read_only".into()),
            }],
            peer_rules: vec![PeerRule {
                peer_id: "*".into(),
                can_deploy: true,
                allowed_capabilities: Vec::new(),
            }],
        };
        let peer = Keypair::generate().peer_id();

        // Allowed pattern
        let caps = vec![CapabilityGroup::RosInterface {
            version: "1.0".into(),
            patterns: vec![AccessPattern {
                pattern: "/diagnostics/cpu".into(),
                access: RosAccess::ReadOnly,
            }],
        }];
        assert!(policy.evaluate(&caps, &peer).is_ok());

        // Disallowed pattern
        let caps = vec![CapabilityGroup::RosInterface {
            version: "1.0".into(),
            patterns: vec![AccessPattern {
                pattern: "/cmd_vel".into(),
                access: RosAccess::ReadOnly,
            }],
        }];
        assert!(policy.evaluate(&caps, &peer).is_err());

        // ReadWrite exceeds read_only max
        let caps = vec![CapabilityGroup::RosInterface {
            version: "1.0".into(),
            patterns: vec![AccessPattern {
                pattern: "/diagnostics/cpu".into(),
                access: RosAccess::ReadWrite,
            }],
        }];
        assert!(policy.evaluate(&caps, &peer).is_err());
    }

    #[test]
    fn specific_peer_authorization() {
        let allowed = Keypair::generate();
        let denied = Keypair::generate();

        let policy = Policy {
            capability_rules: vec![CapabilityRule {
                group: "ganglion:diagnostics/collect".into(),
                allowed_patterns: vec!["**".into()],
                max_access: None,
            }],
            peer_rules: vec![PeerRule {
                peer_id: allowed.peer_id().as_str().to_string(),
                can_deploy: true,
                allowed_capabilities: Vec::new(),
            }],
        };

        let caps = vec![CapabilityGroup::DiagnosticsCollect {
            version: "1.0".into(),
        }];

        assert!(policy.evaluate(&caps, &allowed.peer_id()).is_ok());
        assert!(policy.evaluate(&caps, &denied.peer_id()).is_err());
    }

    #[test]
    fn read_write_required_for_param_set() {
        // A read_only policy should block ReadWrite patterns (used by param-set).
        let policy = Policy {
            capability_rules: vec![CapabilityRule {
                group: "ganglion:ros/interface".into(),
                allowed_patterns: vec!["**".into()],
                max_access: Some("read_only".into()),
            }],
            peer_rules: vec![PeerRule {
                peer_id: "*".into(),
                can_deploy: true,
                allowed_capabilities: Vec::new(),
            }],
        };
        let peer = Keypair::generate().peer_id();

        // ReadWrite pattern (required for param-set) must be denied.
        let caps = vec![CapabilityGroup::RosInterface {
            version: "1.0".into(),
            patterns: vec![AccessPattern {
                pattern: "/my_node/max_speed".into(),
                access: RosAccess::ReadWrite,
            }],
        }];
        assert!(policy.evaluate(&caps, &peer).is_err());

        // ReadOnly (param-get) should still be allowed.
        let caps = vec![CapabilityGroup::RosInterface {
            version: "1.0".into(),
            patterns: vec![AccessPattern {
                pattern: "/my_node/max_speed".into(),
                access: RosAccess::ReadOnly,
            }],
        }];
        assert!(policy.evaluate(&caps, &peer).is_ok());

        // A read_write policy should allow both.
        let rw_policy = Policy {
            capability_rules: vec![CapabilityRule {
                group: "ganglion:ros/interface".into(),
                allowed_patterns: vec!["**".into()],
                max_access: Some("read_write".into()),
            }],
            peer_rules: vec![PeerRule {
                peer_id: "*".into(),
                can_deploy: true,
                allowed_capabilities: Vec::new(),
            }],
        };
        let caps = vec![CapabilityGroup::RosInterface {
            version: "1.0".into(),
            patterns: vec![AccessPattern {
                pattern: "/my_node/max_speed".into(),
                access: RosAccess::ReadWrite,
            }],
        }];
        assert!(rw_policy.evaluate(&caps, &peer).is_ok());
    }

    #[test]
    fn explain_mirrors_evaluate() {
        // explain() must return Some exactly when evaluate() errs.
        let policy = Policy {
            capability_rules: vec![CapabilityRule {
                group: "ganglion:ros/interface".into(),
                allowed_patterns: vec!["/diagnostics/**".into()],
                max_access: Some("read_only".into()),
            }],
            peer_rules: vec![PeerRule {
                peer_id: "*".into(),
                can_deploy: true,
                allowed_capabilities: Vec::new(),
            }],
        };
        let peer = Keypair::generate().peer_id();

        let allowed_caps = vec![CapabilityGroup::RosInterface {
            version: "1.0".into(),
            patterns: vec![AccessPattern {
                pattern: "/diagnostics/cpu".into(),
                access: RosAccess::ReadOnly,
            }],
        }];
        let denied_caps = vec![CapabilityGroup::RosInterface {
            version: "1.0".into(),
            patterns: vec![AccessPattern {
                pattern: "/cmd_vel".into(),
                access: RosAccess::ReadWrite,
            }],
        }];

        assert!(policy.evaluate(&allowed_caps, &peer).is_ok());
        assert!(policy.explain(&allowed_caps, &peer).is_none());
        assert!(policy.evaluate(&denied_caps, &peer).is_err());
        let report = policy.explain(&denied_caps, &peer).unwrap();
        assert_eq!(report.pattern.as_deref(), Some("/cmd_vel"));
        assert!(matches!(report.kind, DenialKind::PatternNotCovered { .. }));
    }

    #[test]
    fn explain_reports_each_denial_kind() {
        let peer = Keypair::generate().peer_id();
        let ros_rw = |pattern: &str| {
            vec![CapabilityGroup::RosInterface {
                version: "1.0".into(),
                patterns: vec![AccessPattern {
                    pattern: pattern.into(),
                    access: RosAccess::ReadWrite,
                }],
            }]
        };

        // Peer not authorized (empty peer rules).
        let report = Policy::default().explain(&ros_rw("/x"), &peer).unwrap();
        assert!(matches!(report.kind, DenialKind::PeerNotAuthorized));
        assert!(report.suggested_command().contains("allow-peer"));

        // No rule for group.
        let policy = Policy {
            capability_rules: Vec::new(),
            peer_rules: vec![PeerRule {
                peer_id: "*".into(),
                can_deploy: true,
                allowed_capabilities: Vec::new(),
            }],
        };
        let report = policy.explain(&ros_rw("/cmd_vel"), &peer).unwrap();
        assert!(matches!(report.kind, DenialKind::NoRuleForGroup));
        // The suggestion permits exactly the denied request, not "**".
        assert!(report.suggested_rule().contains("\"/cmd_vel\""));
        assert!(!report.suggested_rule().contains("\"**\""));

        // Access exceeds max.
        let policy = Policy {
            capability_rules: vec![CapabilityRule {
                group: "ganglion:ros/interface".into(),
                allowed_patterns: vec!["/cmd_vel".into()],
                max_access: Some("read_only".into()),
            }],
            peer_rules: policy.peer_rules.clone(),
        };
        let report = policy.explain(&ros_rw("/cmd_vel"), &peer).unwrap();
        assert!(matches!(report.kind, DenialKind::AccessExceedsMax { .. }));
        let rendered = report.render();
        assert!(rendered.contains("gang policy allow"));
        assert!(rendered.contains("policy.toml"));
    }

    #[test]
    fn allow_applies_minimal_widening() {
        let mut policy = Policy {
            capability_rules: vec![CapabilityRule {
                group: "ganglion:ros/interface".into(),
                allowed_patterns: vec!["/diagnostics".into()],
                max_access: Some("read_only".into()),
            }],
            peer_rules: vec![PeerRule {
                peer_id: "*".into(),
                can_deploy: true,
                allowed_capabilities: Vec::new(),
            }],
        };
        let peer = Keypair::generate().peer_id();

        // New pattern appends to the existing rule.
        assert_eq!(
            policy.allow("ganglion:ros/interface", "/rosout", None),
            AllowOutcome::AddedPattern
        );
        // Same request again is a no-op.
        assert_eq!(
            policy.allow("ganglion:ros/interface", "/rosout", None),
            AllowOutcome::AlreadyAllowed
        );
        // read_write on a read_only rule raises the ceiling.
        assert_eq!(
            policy.allow("ganglion:ros/interface", "/rosout", Some("read_write")),
            AllowOutcome::RaisedAccess
        );
        // Unknown group creates a narrow new rule.
        assert_eq!(
            policy.allow("ganglion:logs/stream", "journald/**", None),
            AllowOutcome::NewRule
        );

        // The widened policy now permits what explain() suggested — closing
        // the loop: denial → allow → permitted.
        let caps = vec![CapabilityGroup::RosInterface {
            version: "1.0".into(),
            patterns: vec![AccessPattern {
                pattern: "/rosout".into(),
                access: RosAccess::ReadWrite,
            }],
        }];
        assert!(policy.evaluate(&caps, &peer).is_ok());
        // And it round-trips through TOML.
        let reloaded = Policy::from_toml(&policy.to_toml_pretty().unwrap()).unwrap();
        assert!(reloaded.evaluate(&caps, &peer).is_ok());
    }

    #[test]
    fn lint_flags_wide_open_rules() {
        // The permissive dev policy is exactly what lint exists to catch.
        let findings = Policy::permissive().lint();
        assert!(
            findings
                .iter()
                .any(|f| f.finding.contains("\"**\"") && f.location.contains("ros"))
        );
        assert!(findings.iter().any(|f| f.location == "peer \"*\""));
        assert!(findings.iter().any(|f| f.finding.contains("read_write")));

        // A scoped policy lints clean.
        let scoped = Policy {
            capability_rules: vec![CapabilityRule {
                group: "ganglion:ros/interface".into(),
                allowed_patterns: vec!["/diagnostics/**".into()],
                max_access: Some("read_only".into()),
            }],
            peer_rules: vec![PeerRule {
                peer_id: "12D3-abc".into(),
                can_deploy: true,
                allowed_capabilities: Vec::new(),
            }],
        };
        assert!(scoped.lint().is_empty());
    }

    #[test]
    fn policy_toml_roundtrip() {
        let policy = Policy::permissive();
        let toml_str = toml::to_string_pretty(&policy).unwrap();
        let loaded = Policy::from_toml(&toml_str).unwrap();
        assert_eq!(loaded.capability_rules.len(), policy.capability_rules.len());
    }
}
