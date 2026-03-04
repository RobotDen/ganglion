//! Wasmtime engine configuration with fuel metering and epoch interruption.

use std::sync::Arc;
use std::time::Duration;

use wasmtime::{Config, Engine};

/// Shared Wasmtime engine configured for Ganglion capability execution.
///
/// One engine is shared across all component invocations on a node.
/// The engine owns the compilation cache and epoch ticker.
#[derive(Clone)]
pub struct GanglionEngine {
    engine: Engine,
    /// Kept alive to prevent the epoch ticker thread from exiting.
    #[allow(dead_code)]
    epoch_handle: Arc<EpochHandle>,
}

struct EpochHandle {
    shutdown_tx: std::sync::mpsc::Sender<()>,
}

impl Drop for EpochHandle {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(());
    }
}

impl GanglionEngine {
    /// Create a new engine with Ganglion's default configuration.
    ///
    /// Enables:
    /// - Component model
    /// - Fuel metering for CPU budgets
    /// - Epoch interruption for wall-clock deadlines
    pub fn new() -> anyhow::Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        config.epoch_interruption(true);

        // Reasonable defaults for field deployments
        config.cranelift_opt_level(wasmtime::OptLevel::Speed);

        let engine = Engine::new(&config)?;

        // Start epoch ticker thread — increments every 1 second.
        // Wall-clock deadlines are enforced by setting epoch_deadline on the Store.
        let engine_clone = engine.clone();
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();

        std::thread::Builder::new()
            .name("ganglion-epoch-ticker".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    engine_clone.increment_epoch();
                    if shutdown_rx.try_recv().is_ok() {
                        break;
                    }
                }
            })?;

        Ok(Self {
            engine,
            epoch_handle: Arc::new(EpochHandle { shutdown_tx }),
        })
    }

    /// Get the underlying Wasmtime engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_creates_successfully() {
        let engine = GanglionEngine::new().unwrap();
        // Verify engine was created and is usable
        let _ = engine.engine();
    }

    #[test]
    fn engine_is_cloneable() {
        let engine = GanglionEngine::new().unwrap();
        let _clone = engine.clone();
    }
}
