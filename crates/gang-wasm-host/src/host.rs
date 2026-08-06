//! Capability host state — bridges WASM component calls to Layer 3 brokers.
//!
//! The host state is attached to each Wasmtime Store. When a WASM component
//! calls an imported function (e.g., `diagnostics-collect.system-info`), the
//! call routes through this host to the appropriate CapabilityBroker.

use std::collections::HashMap;
use std::sync::Arc;

use gang_core::broker::{BrokerOperation, CapabilityBroker, CapabilityRequest, CapabilityResponse};
use gang_core::capability::CapabilityGroup;
use wasmtime::component::ResourceTable;
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

/// Upper bound on captured component stdout/stderr, each. Components built
/// with standard toolchains write panic messages and debug prints through
/// WASI stdio; capturing them (bounded) makes failures diagnosable. A
/// component that writes more than this traps on the overflowing write.
const STDIO_CAPTURE_BYTES: usize = 1024 * 1024;

/// Host state attached to every Wasmtime Store during capability execution.
pub struct CapabilityHost {
    /// Brokers keyed by capability group name.
    brokers: HashMap<String, Arc<dyn CapabilityBroker>>,
    /// Which capability groups this component declared — only these are callable.
    declared_capabilities: Vec<CapabilityGroup>,
    /// Resource limits (linear memory, tables, instances) enforced by the
    /// Wasmtime store. Installed via `Store::limiter` so that a component that
    /// tries to grow memory beyond the manifest-derived cap is trapped rather
    /// than allowed to OOM the host.
    pub limits: wasmtime::StoreLimits,
    /// Locked-down WASI context. Components built with standard toolchains
    /// (cargo-component, componentize-py, TinyGo) wrap a WASI core module and
    /// import `wasi:*` interfaces even when they never touch the system, so
    /// the runtime must provide them — with everything denied: no
    /// environment, no arguments, no preopened directories, all socket
    /// addresses denied. Stdin is closed; stdout/stderr are captured
    /// (bounded) for diagnostics. This preserves the no-ambient-authority
    /// model: system access still flows only through declared capability
    /// brokers.
    wasi: WasiCtx,
    /// Resource table backing the WASI implementation (streams etc.).
    wasi_table: ResourceTable,
    /// Bounded capture of the component's WASI stdout.
    stdout_pipe: MemoryOutputPipe,
    /// Bounded capture of the component's WASI stderr.
    stderr_pipe: MemoryOutputPipe,
}

impl CapabilityHost {
    /// Create a new host with the given brokers and declared capabilities.
    /// The store limits default to unlimited; use [`Self::with_limits`] to install a
    /// memory cap derived from the component manifest.
    pub fn new(
        brokers: HashMap<String, Arc<dyn CapabilityBroker>>,
        declared_capabilities: Vec<CapabilityGroup>,
    ) -> Self {
        let stdout_pipe = MemoryOutputPipe::new(STDIO_CAPTURE_BYTES);
        let stderr_pipe = MemoryOutputPipe::new(STDIO_CAPTURE_BYTES);
        let mut wasi_builder = WasiCtxBuilder::new();
        wasi_builder.stdout(stdout_pipe.clone());
        wasi_builder.stderr(stderr_pipe.clone());
        // Everything else stays at the deny-by-default builder settings:
        // stdin closed, no env, no args, no preopens, sockets deny-all.
        let wasi = wasi_builder.build();
        Self {
            brokers,
            declared_capabilities,
            limits: wasmtime::StoreLimits::default(),
            wasi,
            wasi_table: ResourceTable::new(),
            stdout_pipe,
            stderr_pipe,
        }
    }

    /// Bytes the component wrote to WASI stdout (bounded capture).
    pub fn stdout_contents(&self) -> Vec<u8> {
        self.stdout_pipe.contents().to_vec()
    }

    /// Bytes the component wrote to WASI stderr (bounded capture).
    pub fn stderr_contents(&self) -> Vec<u8> {
        self.stderr_pipe.contents().to_vec()
    }

    /// Attach resource limits to this host. The limits are consulted by the
    /// Wasmtime store via `Store::limiter(|h| &mut h.limits)`.
    pub fn with_limits(mut self, limits: wasmtime::StoreLimits) -> Self {
        self.limits = limits;
        self
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

/// Exposes the locked-down WASI state to `wasmtime-wasi`'s host functions
/// (registered on the runtime's linker in `ComponentRuntime::new`).
impl WasiView for CapabilityHost {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.wasi_table,
        }
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
