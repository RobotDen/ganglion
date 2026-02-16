use serde::{Deserialize, Serialize};

use crate::capability::{CapabilityGroup, RosAccess};
use crate::error::PolicyError;
use crate::identity::PeerId;

/// The policy engine evaluates capability declarations against active policy
/// at load time. Default-deny: anything not explicitly permitted is rejected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    /// Rules per capability group. If a group has no entry, it is denied.
    #[serde(default)]
    pub capability_rules: Vec<CapabilityRule>,

    /// Per-peer rules: which operators can deploy capabilities.
    #[serde(default)]
    pub peer_rules: Vec<PeerRule>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            capability_rules: Vec::new(),
            peer_rules: Vec::new(),
        }
    }
}

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
            return Err(PolicyError::PolicyNotFound(
                path.display().to_string(),
            ));
        }
        let contents = std::fs::read_to_string(path)
            .map_err(|e| PolicyError::InvalidPolicy(e.to_string()))?;
        toml::from_str(&contents)
            .map_err(|e| PolicyError::InvalidPolicy(e.to_string()))
    }

    /// Load policy from a TOML string.
    pub fn from_toml(toml_str: &str) -> Result<Self, PolicyError> {
        toml::from_str(toml_str)
            .map_err(|e| PolicyError::InvalidPolicy(e.to_string()))
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

        let authorized = self.peer_rules.iter().any(|rule| {
            rule.peer_id == "*" || rule.peer_id == peer.as_str()
        }) && self.peer_rules.iter().any(|rule| {
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

        let rule = self
            .capability_rules
            .iter()
            .find(|r| r.group == group_name);

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
                    if pattern.access == RosAccess::ReadWrite {
                        if let Some(max) = &rule.max_access {
                            if max == "read_only" {
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
        }

        Ok(())
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
    fn policy_toml_roundtrip() {
        let policy = Policy::permissive();
        let toml_str = toml::to_string_pretty(&policy).unwrap();
        let loaded = Policy::from_toml(&toml_str).unwrap();
        assert_eq!(loaded.capability_rules.len(), policy.capability_rules.len());
    }
}
