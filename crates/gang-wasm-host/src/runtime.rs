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

use tokio::sync::RwLock;
use wasmtime::component::{Component, Linker};

use gang_core::broker::CapabilityBroker;
use gang_core::capability::CapabilityGroup;
use gang_core::error::CapabilityError;
use gang_core::manifest::ResourceLimits;

use crate::engine::GanglionEngine;
use crate::host::CapabilityHost;
use crate::imports::register_capability_imports;

/// Host default linear-memory ceiling applied when a manifest does not declare
/// `max_memory_bytes` (or declares 0). 256 MiB is generous for field tooling
/// while still bounding a runaway component.
pub const DEFAULT_MAX_MEMORY_BYTES: u64 = 256 * 1024 * 1024;

/// Absolute hard cap on linear memory. A manifest may request less, but never
/// more than this — a hostile or buggy manifest cannot raise its own ceiling.
pub const HARD_MAX_MEMORY_BYTES: u64 = 1024 * 1024 * 1024;

/// Default fuel budget when a manifest does not declare `cpu_fuel`.
pub const DEFAULT_CPU_FUEL: u64 = 1_000_000;

/// Absolute hard cap on fuel. Bounds worst-case CPU per invocation.
pub const HARD_MAX_CPU_FUEL: u64 = 10_000_000_000;

/// Default wall-clock deadline (seconds) when a manifest does not declare one.
pub const DEFAULT_WALL_CLOCK_SECS: u64 = 300;

/// Absolute hard cap on the wall-clock deadline (seconds).
pub const HARD_MAX_WALL_CLOCK_SECS: u64 = 3600;

/// Resolve the effective linear-memory ceiling (bytes) from manifest limits,
/// applying the host default for unset values and clamping to the hard cap.
pub fn effective_memory_bytes(limits: &ResourceLimits) -> usize {
    let requested = if limits.max_memory_bytes == 0 {
        DEFAULT_MAX_MEMORY_BYTES
    } else {
        limits.max_memory_bytes
    };
    requested.min(HARD_MAX_MEMORY_BYTES) as usize
}

/// Resolve the effective fuel budget from manifest limits, applying the host
/// default for unset values and clamping to the hard cap.
pub fn effective_fuel(limits: &ResourceLimits) -> u64 {
    let requested = if limits.cpu_fuel == 0 {
        DEFAULT_CPU_FUEL
    } else {
        limits.cpu_fuel
    };
    requested.min(HARD_MAX_CPU_FUEL)
}

/// Resolve the effective wall-clock deadline (seconds) from manifest limits.
pub fn effective_wall_clock_secs(limits: &ResourceLimits) -> u64 {
    let requested = if limits.wall_clock_secs == 0 {
        DEFAULT_WALL_CLOCK_SECS
    } else {
        limits.wall_clock_secs
    };
    requested.min(HARD_MAX_WALL_CLOCK_SECS)
}

/// Classify a Wasmtime invocation error into a structured [`InvocationError`].
///
/// Classification downcasts to concrete `wasmtime::Trap` variants rather than
/// matching on rendered error text, so it is robust to message wording changes:
/// - [`wasmtime::Trap::OutOfFuel`] → [`InvocationError::FuelExhausted`]
/// - [`wasmtime::Trap::Interrupt`] (epoch deadline) → [`InvocationError::DeadlineExceeded`]
/// - any other trap / error → [`InvocationError::Trapped`]
pub fn classify_invocation_error(
    err: &anyhow::Error,
    elapsed: Duration,
    fuel_consumed: Option<u64>,
    fuel_budget: u64,
) -> InvocationError {
    if let Some(trap) = err.downcast_ref::<wasmtime::Trap>() {
        match trap {
            wasmtime::Trap::OutOfFuel => {
                return InvocationError::FuelExhausted {
                    consumed: fuel_consumed.unwrap_or(fuel_budget),
                };
            }
            wasmtime::Trap::Interrupt => {
                return InvocationError::DeadlineExceeded { elapsed };
            }
            other => {
                return InvocationError::Trapped(format!("wasm trap: {other:?}"));
            }
        }
    }
    InvocationError::Trapped(format!("{err:#}"))
}

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
            InvocationError::InstantiationFailed(msg) => CapabilityError::InstantiationFailed(msg),
            other => CapabilityError::Trapped {
                name: String::new(),
                message: other.to_string(),
            },
        }
    }
}

/// The component runtime — manages loading and invoking WASM components.
///
/// Constructed once per agent (see CODE-06): it owns the shared engine, the
/// capability linker (host imports are registered a single time), and a cache
/// of compiled [`Component`]s keyed by component hash so that repeated
/// invocations of the same capability do not recompile the module.
pub struct ComponentRuntime {
    engine: GanglionEngine,
    brokers: HashMap<String, Arc<dyn CapabilityBroker>>,
    /// Capability host imports, registered once at construction and reused for
    /// every instantiation.
    linker: Linker<CapabilityHost>,
    /// Compiled component cache keyed by Blake3 component hash.
    component_cache: RwLock<HashMap<String, Component>>,
}

impl ComponentRuntime {
    /// Create a new runtime with the given engine and brokers.
    ///
    /// Registers the capability imports on a single shared linker. Returns an
    /// error if import registration fails.
    pub fn new(
        engine: GanglionEngine,
        brokers: HashMap<String, Arc<dyn CapabilityBroker>>,
    ) -> anyhow::Result<Self> {
        let mut linker = Linker::<CapabilityHost>::new(engine.engine());
        register_capability_imports(&mut linker)?;
        Ok(Self {
            engine,
            brokers,
            linker,
            component_cache: RwLock::new(HashMap::new()),
        })
    }

    /// Compile a component, using the cache keyed by `component_hash`. On a
    /// cache miss the module is compiled once and stored for reuse.
    async fn compile_cached(
        &self,
        component_bytes: &[u8],
        component_hash: &str,
    ) -> Result<Component, InvocationError> {
        if let Some(component) = self.component_cache.read().await.get(component_hash) {
            return Ok(component.clone());
        }
        let component = Component::new(self.engine.engine(), component_bytes).map_err(|e| {
            InvocationError::InstantiationFailed(format!("failed to compile component: {e}"))
        })?;
        self.component_cache
            .write()
            .await
            .insert(component_hash.to_string(), component.clone());
        Ok(component)
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
        component_hash: &str,
        declared_capabilities: Vec<CapabilityGroup>,
        limits: &ResourceLimits,
        args: Vec<String>,
    ) -> Result<ComponentResult, InvocationError> {
        let start = Instant::now();

        // SEC-04: derive a linear-memory ceiling from the manifest, clamped to
        // the host hard cap, and install it on the store so a component that
        // grows memory past the limit traps instead of OOMing the host.
        let memory_bytes = effective_memory_bytes(limits);
        let store_limits = wasmtime::StoreLimitsBuilder::new()
            .memory_size(memory_bytes)
            .build();

        let host = CapabilityHost::new(self.brokers.clone(), declared_capabilities)
            .with_limits(store_limits);

        // Create the Wasmtime Store with our host state
        let mut store = wasmtime::Store::new(self.engine.engine(), host);
        store.limiter(|h| &mut h.limits);

        // Configure fuel metering (SEC-05: manifest-derived, clamped)
        let fuel_budget = effective_fuel(limits);
        store.set_fuel(fuel_budget).map_err(|e| {
            InvocationError::InstantiationFailed(format!("failed to set fuel: {e}"))
        })?;

        // Configure epoch deadline for wall-clock timeout (SEC-05: clamped)
        let deadline_secs = effective_wall_clock_secs(limits);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(deadline_secs);

        // Load the component (CODE-06: compiled once, cached by hash).
        let component = self.compile_cached(component_bytes, component_hash).await?;

        // The capability linker is registered once at construction and reused.
        // Undeclared capabilities are rejected at call time (not link time),
        // so all interfaces are registered regardless of what the component
        // declares — the CapabilityHost enforces the manifest's declaration
        // list when the function is actually invoked.
        let instance = self
            .linker
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
        let run_func = instance.get_func(&mut store, "run").ok_or_else(|| {
            InvocationError::InstantiationFailed(
                "component does not export a 'run' function".into(),
            )
        })?;

        // Prepare arguments
        let args_val: Vec<wasmtime::component::Val> = vec![wasmtime::component::Val::List(
            args.into_iter()
                .map(wasmtime::component::Val::String)
                .collect(),
        )];

        // Invoke
        let mut results = vec![wasmtime::component::Val::Bool(false)]; // placeholder
        let invoke_result = run_func
            .call_async(&mut store, &args_val, &mut results)
            .await;

        let elapsed = start.elapsed();
        let fuel_consumed = store
            .get_fuel()
            .ok()
            .map(|remaining| fuel_budget - remaining);

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
                // CODE-09: classify by downcasting to concrete wasmtime::Trap
                // variants rather than substring matching on error text.
                Err(classify_invocation_error(
                    &e,
                    elapsed,
                    fuel_consumed,
                    fuel_budget,
                ))
            }
        }
    }

    /// Validate that a component can be compiled without actually running it.
    /// Useful for pre-flight checks during `gang deploy`.
    pub fn validate_component(&self, component_bytes: &[u8]) -> Result<(), InvocationError> {
        wasmtime::component::Component::new(self.engine.engine(), component_bytes).map_err(
            |e| InvocationError::InstantiationFailed(format!("component validation failed: {e}")),
        )?;
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
        let runtime = ComponentRuntime::new(engine, HashMap::new()).unwrap();
        let result = runtime.validate_component(b"not valid wasm");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn invoke_invalid_wasm_fails() {
        let engine = GanglionEngine::new().unwrap();
        let runtime = ComponentRuntime::new(engine, HashMap::new()).unwrap();
        let result = runtime
            .invoke(
                b"not valid wasm",
                "deadbeef",
                vec![],
                &ResourceLimits::default(),
                vec![],
            )
            .await;
        assert!(matches!(
            result,
            Err(InvocationError::InstantiationFailed(_))
        ));
    }

    #[tokio::test]
    async fn runtime_creates_with_brokers() {
        let engine = GanglionEngine::new().unwrap();
        let brokers: HashMap<String, Arc<dyn CapabilityBroker>> = HashMap::new();
        let _runtime = ComponentRuntime::new(engine, brokers).unwrap();
    }

    // --- SEC-04: memory limit clamping ---

    #[test]
    fn memory_limit_uses_default_when_unset() {
        let limits = ResourceLimits::default();
        assert_eq!(
            effective_memory_bytes(&limits),
            DEFAULT_MAX_MEMORY_BYTES as usize
        );
    }

    #[test]
    fn memory_limit_clamped_to_hard_cap() {
        let limits = ResourceLimits {
            max_memory_bytes: u64::MAX,
            ..Default::default()
        };
        assert_eq!(
            effective_memory_bytes(&limits),
            HARD_MAX_MEMORY_BYTES as usize
        );
    }

    #[test]
    fn memory_limit_honors_manifest_request() {
        let limits = ResourceLimits {
            max_memory_bytes: 8 * 1024 * 1024,
            ..Default::default()
        };
        assert_eq!(effective_memory_bytes(&limits), 8 * 1024 * 1024);
    }

    #[test]
    fn fuel_and_wall_clock_clamped() {
        let limits = ResourceLimits {
            cpu_fuel: u64::MAX,
            wall_clock_secs: u64::MAX,
            ..Default::default()
        };
        assert_eq!(effective_fuel(&limits), HARD_MAX_CPU_FUEL);
        assert_eq!(effective_wall_clock_secs(&limits), HARD_MAX_WALL_CLOCK_SECS);
    }

    /// SEC-04: a module whose declared memory exceeds the store limit must be
    /// stopped at instantiation (the ResourceLimiter denies the growth) rather
    /// than allowed to allocate and OOM the host. We exercise the exact store
    /// configuration the runtime uses: a `CapabilityHost` carrying `StoreLimits`
    /// with `Store::limiter` installed.
    #[tokio::test]
    async fn store_limiter_denies_oversized_memory() {
        let engine = GanglionEngine::new().unwrap();

        // 1 MiB ceiling.
        let store_limits = wasmtime::StoreLimitsBuilder::new()
            .memory_size(1024 * 1024)
            .build();
        let host = CapabilityHost::new(HashMap::new(), vec![]).with_limits(store_limits);
        let mut store = wasmtime::Store::new(engine.engine(), host);
        store.limiter(|h| &mut h.limits);

        // A core module that demands 100 pages (6.4 MiB) of linear memory.
        let wasm = wat::parse_str("(module (memory 100))").unwrap();
        let module = wasmtime::Module::new(engine.engine(), &wasm).unwrap();
        let result = wasmtime::Instance::new_async(&mut store, &module, &[]).await;
        assert!(
            result.is_err(),
            "instantiation should be denied by the memory limiter"
        );

        // A module within the limit (4 pages = 256 KiB) instantiates fine.
        let small = wat::parse_str("(module (memory 4))").unwrap();
        let small_module = wasmtime::Module::new(engine.engine(), &small).unwrap();
        assert!(
            wasmtime::Instance::new_async(&mut store, &small_module, &[])
                .await
                .is_ok()
        );
    }

    // --- CODE-09: trap classification by downcast, not substring ---

    #[test]
    fn classify_out_of_fuel() {
        let err = anyhow::Error::from(wasmtime::Trap::OutOfFuel);
        let classified =
            classify_invocation_error(&err, Duration::from_secs(1), Some(42), 1000);
        match classified {
            InvocationError::FuelExhausted { consumed } => assert_eq!(consumed, 42),
            other => panic!("expected FuelExhausted, got {other:?}"),
        }
    }

    #[test]
    fn classify_epoch_interrupt() {
        let err = anyhow::Error::from(wasmtime::Trap::Interrupt);
        let classified =
            classify_invocation_error(&err, Duration::from_secs(7), None, 1000);
        match classified {
            InvocationError::DeadlineExceeded { elapsed } => {
                assert_eq!(elapsed, Duration::from_secs(7))
            }
            other => panic!("expected DeadlineExceeded, got {other:?}"),
        }
    }

    #[test]
    fn classify_generic_trap() {
        let err = anyhow::Error::from(wasmtime::Trap::MemoryOutOfBounds);
        let classified =
            classify_invocation_error(&err, Duration::from_secs(1), None, 1000);
        assert!(matches!(classified, InvocationError::Trapped(_)));
    }

    #[test]
    fn classify_non_trap_error() {
        // An error that does not carry a wasmtime::Trap falls through to Trapped
        // rather than being misclassified via text matching (e.g. a message that
        // happens to contain the word "fuel").
        let err = anyhow::anyhow!("some unrelated fuel-tank failure");
        let classified =
            classify_invocation_error(&err, Duration::from_secs(1), None, 1000);
        assert!(matches!(classified, InvocationError::Trapped(_)));
    }
}
