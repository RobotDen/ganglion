//! WASM component runtime and capability enforcement for Ganglion.
//!
//! Hosts WASM components using Wasmtime's component model. Enforces memory limits,
//! CPU budgets (fuel metering), and wall-clock deadlines (epoch interruption) from
//! the component manifest. Connects component capability calls to Layer 3 brokers.

pub mod engine;
pub mod host;
pub mod runtime;

pub use engine::GanglionEngine;
pub use host::CapabilityHost;
pub use runtime::{ComponentResult, InvocationError};
