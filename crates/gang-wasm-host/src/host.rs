//! Capability host state — bridges WASM component calls to Layer 3 brokers.
//!
//! The host state is attached to each Wasmtime Store. When a WASM component
//! calls an imported function (e.g., `diagnostics-collect.system-info`), the
//! call routes through this host to the appropriate CapabilityBroker.

use std::collections::HashMap;
use std::sync::Arc;

use gang_core::broker::{BrokerOperation, CapabilityBroker, CapabilityRequest, CapabilityResponse};
use gang_core::capability::CapabilityGroup;

/// Host state attached to every Wasmtime Store during capability execution.
pub struct CapabilityHost {
    /// Brokers keyed by capability group name.
    brokers: HashMap<String, Arc<dyn CapabilityBroker>>,
    /// Which capability groups this component declared — only these are callable.
    declared_capabilities: Vec<CapabilityGroup>,
    /// Stdout captured from the component.
    pub stdout: Vec<u8>,
    /// Stderr captured from the component.
    pub stderr: Vec<u8>,
}

impl CapabilityHost {
    /// Create a new host with the given brokers and declared capabilities.
    pub fn new(
        brokers: HashMap<String, Arc<dyn CapabilityBroker>>,
        declared_capabilities: Vec<CapabilityGroup>,
    ) -> Self {
        Self {
            brokers,
            declared_capabilities,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    /// Check whether a capability group is declared by this component.
    pub fn is_declared(&self, group_name: &str) -> bool {
        self.declared_capabilities
            .iter()
            .any(|g| g.name() == group_name)
    }

    /// Route a broker operation to the appropriate broker.
    /// Returns an error if the capability group is not declared (undeclared = trap).
    pub async fn broker_call(
        &self,
        capability_group: &str,
        operation: BrokerOperation,
    ) -> Result<CapabilityResponse, String> {
        // Enforce declaration — undeclared capability calls are denied
        if !self.is_declared(capability_group) {
            return Err(format!(
                "capability group '{capability_group}' not declared in manifest — access denied"
            ));
        }

        let broker = self.brokers.get(capability_group).ok_or_else(|| {
            format!("no broker registered for capability group '{capability_group}'")
        })?;

        let req = CapabilityRequest {
            capability_group: capability_group.to_string(),
            operation,
        };

        broker.handle_request(req).await.map_err(|e| e.to_string())
    }

    /// Get a reference to a broker by capability group name.
    pub fn get_broker(&self, group: &str) -> Option<&Arc<dyn CapabilityBroker>> {
        self.brokers.get(group)
    }

    /// List all registered broker capability groups.
    pub fn registered_groups(&self) -> Vec<&str> {
        self.brokers.keys().map(|s| s.as_str()).collect()
    }

    /// List all declared capability groups.
    pub fn declared_groups(&self) -> Vec<String> {
        self.declared_capabilities
            .iter()
            .map(|g| g.qualified_name())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gang_core::capability::CapabilityGroup;

    #[test]
    fn undeclared_capability_rejected() {
        let host = CapabilityHost::new(HashMap::new(), vec![]);
        assert!(!host.is_declared("ganglion:ros/interface"));
    }

    #[test]
    fn declared_capability_accepted() {
        let caps = vec![CapabilityGroup::DiagnosticsCollect {
            version: "1.0".into(),
        }];
        let host = CapabilityHost::new(HashMap::new(), caps);
        assert!(host.is_declared("ganglion:diagnostics/collect"));
        assert!(!host.is_declared("ganglion:ros/interface"));
    }

    #[test]
    fn registered_groups_lists_brokers() {
        let brokers: HashMap<String, Arc<dyn CapabilityBroker>> = HashMap::new();
        assert!(
            CapabilityHost::new(brokers, vec![])
                .registered_groups()
                .is_empty()
        );
    }
}
