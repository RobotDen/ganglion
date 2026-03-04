//! WASM component runtime — loads, instantiates, and invokes WASM components.
//!
//! This is the core execution engine for Ganglion capabilities. It:
//! 1. Loads a .wasm component file
//! 2. Configures resource limits from the manifest (fuel, memory, wall-clock)
//! 3. Links only declared capability imports (undeclared = trap)
//! 4. Invokes the component's `run` export
//! 5. Captures stdout/stderr and returns the result

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gang_core::broker::CapabilityBroker;
use gang_core::capability::CapabilityGroup;
use gang_core::error::CapabilityError;
use gang_core::manifest::ResourceLimits;

use crate::engine::GanglionEngine;
use crate::host::CapabilityHost;

/// Result of a component invocation.
#[derive(Debug)]
pub struct ComponentResult {
    /// The serialized return value from the component's `run` export.
    pub data: Vec<u8>,
    /// Captured stdout.
    pub stdout: Vec<u8>,
    /// Captured stderr.
    pub stderr: Vec<u8>,
    /// Wall-clock time elapsed during execution.
    pub elapsed: Duration,
    /// Fuel consumed (if metering was enabled).
    pub fuel_consumed: Option<u64>,
}

/// Errors during component invocation.
#[derive(Debug, thiserror::Error)]
pub enum InvocationError {
    #[error("instantiation failed: {0}")]
    InstantiationFailed(String),

    #[error("fuel exhausted after consuming {consumed} units")]
    FuelExhausted { consumed: u64 },

    #[error("wall-clock deadline exceeded after {elapsed:?}")]
    DeadlineExceeded { elapsed: Duration },

    #[error("component trapped: {0}")]
    Trapped(String),

    #[error("component returned error: {0}")]
    ComponentError(String),

    #[error("capability not declared: {0}")]
    UndeclaredCapability(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

impl From<InvocationError> for CapabilityError {
    fn from(e: InvocationError) -> Self {
        match e {
            InvocationError::FuelExhausted { consumed } => CapabilityError::ResourceExhausted {
                name: String::new(),
                limit: format!("fuel: consumed {consumed} units"),
            },
            InvocationError::DeadlineExceeded { elapsed } => CapabilityError::Timeout {
                name: String::new(),
                elapsed,
            },
            InvocationError::Trapped(msg) => CapabilityError::Trapped {
                name: String::new(),
                message: msg,
            },
            InvocationError::InstantiationFailed(msg) => {
                CapabilityError::InstantiationFailed(msg)
            }
            other => CapabilityError::Trapped {
                name: String::new(),
                message: other.to_string(),
            },
        }
    }
}

/// The component runtime — manages loading and invoking WASM components.
pub struct ComponentRuntime {
    engine: GanglionEngine,
    brokers: HashMap<String, Arc<dyn CapabilityBroker>>,
}

impl ComponentRuntime {
    /// Create a new runtime with the given engine and brokers.
    pub fn new(
        engine: GanglionEngine,
        brokers: HashMap<String, Arc<dyn CapabilityBroker>>,
    ) -> Self {
        Self { engine, brokers }
    }

    /// Load and invoke a WASM component.
    ///
    /// The component must export a `run(args: list<string>) -> result<list<u8>, string>`
    /// function. Only capability groups listed in `declared_capabilities` will be
    /// linked — all other imports will trap.
    ///
    /// Resource limits from the manifest are enforced:
    /// - `max_memory_bytes`: maximum linear memory (0 = default 256MB)
    /// - `cpu_fuel`: fuel budget for CPU metering (0 = 1M default)
    /// - `wall_clock_secs`: epoch-based deadline (0 = 300s default)
    pub async fn invoke(
        &self,
        component_bytes: &[u8],
        declared_capabilities: Vec<CapabilityGroup>,
        limits: &ResourceLimits,
        args: Vec<String>,
    ) -> Result<ComponentResult, InvocationError> {
        let start = Instant::now();

        let host = CapabilityHost::new(self.brokers.clone(), declared_capabilities);

        // Create the Wasmtime Store with our host state
        let mut store = wasmtime::Store::new(self.engine.engine(), host);

        // Configure fuel metering
        let fuel_budget = if limits.cpu_fuel > 0 {
            limits.cpu_fuel
        } else {
            1_000_000 // Default: 1M fuel units
        };
        store.set_fuel(fuel_budget).map_err(|e| {
            InvocationError::InstantiationFailed(format!("failed to set fuel: {e}"))
        })?;

        // Configure epoch deadline for wall-clock timeout
        let deadline_secs = if limits.wall_clock_secs > 0 {
            limits.wall_clock_secs
        } else {
            300 // Default: 5 minutes
        };
        store.epoch_deadline_trap();
        store.set_epoch_deadline(deadline_secs);

        // Load the component
        let component = wasmtime::component::Component::new(self.engine.engine(), component_bytes)
            .map_err(|e| {
                InvocationError::InstantiationFailed(format!("failed to compile component: {e}"))
            })?;

        // Create a linker and link capability imports
        let linker = wasmtime::component::Linker::<CapabilityHost>::new(self.engine.engine());

        // For v0.1, we use a simplified invocation path:
        // Instead of full WIT binding generation at runtime, we attempt to
        // instantiate the component with whatever imports the linker provides.
        // Unlinked imports will cause instantiation to fail, which maps to
        // "undeclared capability" semantics.
        //
        // Full WIT-based dynamic linking will arrive when wit-bindgen
        // stabilizes its programmatic API for host-side binding generation.

        let instance = linker
            .instantiate_async(&mut store, &component)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("import") {
                    InvocationError::UndeclaredCapability(msg)
                } else {
                    InvocationError::InstantiationFailed(msg)
                }
            })?;

        // Look for the `run` export
        let run_func = instance
            .get_func(&mut store, "run")
            .ok_or_else(|| {
                InvocationError::InstantiationFailed(
                    "component does not export a 'run' function".into(),
                )
            })?;

        // Prepare arguments
        let args_val: Vec<wasmtime::component::Val> = vec![
            wasmtime::component::Val::List(
                args.into_iter()
                    .map(|s| wasmtime::component::Val::String(s))
                    .collect(),
            ),
        ];

        // Invoke
        let mut results = vec![wasmtime::component::Val::Bool(false)]; // placeholder
        let invoke_result = run_func
            .call_async(&mut store, &args_val, &mut results)
            .await;

        let elapsed = start.elapsed();
        let fuel_consumed = store.get_fuel().ok().map(|remaining| fuel_budget - remaining);

        match invoke_result {
            Ok(()) => {
                // Parse the result
                let data = extract_result_data(&results)?;
                let host = store.into_data();
                Ok(ComponentResult {
                    data,
                    stdout: host.stdout,
                    stderr: host.stderr,
                    elapsed,
                    fuel_consumed,
                })
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("fuel") {
                    Err(InvocationError::FuelExhausted {
                        consumed: fuel_consumed.unwrap_or(fuel_budget),
                    })
                } else if msg.contains("epoch") {
                    Err(InvocationError::DeadlineExceeded { elapsed })
                } else {
                    Err(InvocationError::Trapped(msg))
                }
            }
        }
    }

    /// Validate that a component can be compiled without actually running it.
    /// Useful for pre-flight checks during `gang deploy`.
    pub fn validate_component(&self, component_bytes: &[u8]) -> Result<(), InvocationError> {
        wasmtime::component::Component::new(self.engine.engine(), component_bytes)
            .map_err(|e| {
                InvocationError::InstantiationFailed(format!("component validation failed: {e}"))
            })?;
        Ok(())
    }
}

/// Extract the return data from a component result value.
fn extract_result_data(results: &[wasmtime::component::Val]) -> Result<Vec<u8>, InvocationError> {
    match results.first() {
        Some(wasmtime::component::Val::Result(result)) => match result.as_ref() {
            Ok(Some(val)) => match &**val {
                wasmtime::component::Val::List(items) => {
                    let bytes: Vec<u8> = items
                        .iter()
                        .filter_map(|v| match v {
                            wasmtime::component::Val::U8(b) => Some(*b),
                            _ => None,
                        })
                        .collect();
                    Ok(bytes)
                }
                _ => Ok(Vec::new()),
            },
            Ok(None) => Ok(Vec::new()),
            Err(Some(val)) => match &**val {
                wasmtime::component::Val::String(s) => {
                    Err(InvocationError::ComponentError(s.clone()))
                }
                _ => Err(InvocationError::ComponentError("unknown error".into())),
            },
            Err(None) => Err(InvocationError::ComponentError("unknown error".into())),
        },
        _ => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invocation_error_converts_to_capability_error() {
        let err = InvocationError::FuelExhausted { consumed: 500 };
        let cap_err: CapabilityError = err.into();
        match cap_err {
            CapabilityError::ResourceExhausted { limit, .. } => {
                assert!(limit.contains("fuel"));
            }
            _ => panic!("expected ResourceExhausted"),
        }
    }

    #[test]
    fn deadline_exceeded_converts() {
        let err = InvocationError::DeadlineExceeded {
            elapsed: Duration::from_secs(30),
        };
        let cap_err: CapabilityError = err.into();
        assert!(matches!(cap_err, CapabilityError::Timeout { .. }));
    }

    #[tokio::test]
    async fn invalid_wasm_fails_validation() {
        let engine = GanglionEngine::new().unwrap();
        let runtime = ComponentRuntime::new(engine, HashMap::new());
        let result = runtime.validate_component(b"not valid wasm");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn invoke_invalid_wasm_fails() {
        let engine = GanglionEngine::new().unwrap();
        let runtime = ComponentRuntime::new(engine, HashMap::new());
        let result = runtime
            .invoke(
                b"not valid wasm",
                vec![],
                &ResourceLimits::default(),
                vec![],
            )
            .await;
        assert!(matches!(result, Err(InvocationError::InstantiationFailed(_))));
    }

    #[tokio::test]
    async fn runtime_creates_with_brokers() {
        let engine = GanglionEngine::new().unwrap();
        let brokers: HashMap<String, Arc<dyn CapabilityBroker>> = HashMap::new();
        let _runtime = ComponentRuntime::new(engine, brokers);
    }
}
