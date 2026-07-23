use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use gang_core::broker::{
    BrokerOperation, CapabilityBroker, CapabilityRequest, CapabilityResponse, MetricPoint,
};
use gang_core::error::BrokerError;

/// Metrics broker — receives structured metrics emitted by capabilities
/// and stores them for retrieval by the operator.
///
/// The broker accumulates metrics in memory. In a production deployment,
/// these would be forwarded to a time-series database or streamed to the
/// operator via the bulk protocol.
pub struct MetricsBroker {
    /// Accumulated metrics, protected by a mutex for concurrent capability
    /// access. A `VecDeque` gives O(1) eviction of the oldest entry instead of
    /// the O(n) element shift a `Vec` incurs on `drain(..n)` (CODE-24).
    store: Arc<Mutex<VecDeque<StoredMetric>>>,
    /// Maximum metrics to retain (ring buffer semantics).
    max_retained: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMetric {
    pub name: String,
    pub value: f64,
    pub unit: Option<String>,
    pub tags: Vec<(String, String)>,
    pub timestamp_ms: u64,
    pub capability_source: Option<String>,
}

impl MetricsBroker {
    pub fn new(max_retained: usize) -> Self {
        Self {
            store: Arc::new(Mutex::new(VecDeque::new())),
            max_retained,
        }
    }

    /// Lock the store, recovering the guard if the mutex was poisoned by a
    /// panic in another thread rather than propagating the panic (CODE-24).
    /// The stored metrics are plain data, so a poisoned lock is safe to reuse.
    fn lock_store(&self) -> MutexGuard<'_, VecDeque<StoredMetric>> {
        self.store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Retrieve all stored metrics (for operator consumption).
    pub fn drain(&self) -> Vec<StoredMetric> {
        let mut store = self.lock_store();
        Vec::from(std::mem::take(&mut *store))
    }

    /// Current number of stored metrics.
    pub fn count(&self) -> usize {
        self.lock_store().len()
    }

    fn store_metric(&self, point: MetricPoint) {
        let now_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let stored = StoredMetric {
            name: point.name,
            value: point.value,
            unit: point.unit,
            tags: point.tags,
            timestamp_ms: if point.timestamp_ms > 0 {
                point.timestamp_ms
            } else {
                now_ms
            },
            capability_source: None,
        };

        let mut store = self.lock_store();
        store.push_back(stored);

        // Ring buffer: drop oldest entries when full — O(1) each via pop_front.
        while store.len() > self.max_retained {
            store.pop_front();
        }
    }
}

#[async_trait]
impl CapabilityBroker for MetricsBroker {
    async fn handle_request(
        &self,
        req: CapabilityRequest,
    ) -> Result<CapabilityResponse, BrokerError> {
        match req.operation {
            BrokerOperation::MetricEmit {
                name,
                value,
                unit,
                tags,
            } => {
                self.store_metric(MetricPoint {
                    name,
                    value,
                    unit,
                    tags,
                    timestamp_ms: 0,
                });

                Ok(CapabilityResponse {
                    success: true,
                    data: Vec::new(),
                    error: None,
                    bytes_in: 0,
                    bytes_out: 0,
                })
            }
            BrokerOperation::MetricEmitBatch { metrics } => {
                let count = metrics.len();
                for point in metrics {
                    self.store_metric(point);
                }

                let data = serde_json::to_vec(&count).unwrap_or_default();
                Ok(CapabilityResponse {
                    success: true,
                    data,
                    error: None,
                    bytes_in: 0,
                    bytes_out: 0,
                })
            }
            _ => Err(BrokerError::AccessDenied {
                broker: "metrics".into(),
                resource: format!("{:?}", req.operation),
                reason: "operation not supported by metrics broker".into(),
            }),
        }
    }

    fn capability_group(&self) -> &str {
        "ganglion:metrics/emit"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn emit_single_metric() {
        let broker = MetricsBroker::new(1000);
        let req = CapabilityRequest {
            capability_group: "ganglion:metrics/emit".into(),
            operation: BrokerOperation::MetricEmit {
                name: "cpu.usage".into(),
                value: 42.5,
                unit: Some("percent".into()),
                tags: vec![("host".into(), "robot-1".into())],
            },
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);
        assert_eq!(broker.count(), 1);

        let metrics = broker.drain();
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].name, "cpu.usage");
        assert_eq!(metrics[0].value, 42.5);
        assert_eq!(metrics[0].unit.as_deref(), Some("percent"));
    }

    #[tokio::test]
    async fn emit_batch_metrics() {
        let broker = MetricsBroker::new(1000);
        let req = CapabilityRequest {
            capability_group: "ganglion:metrics/emit".into(),
            operation: BrokerOperation::MetricEmitBatch {
                metrics: vec![
                    MetricPoint {
                        name: "mem.used".into(),
                        value: 1024.0,
                        unit: Some("MB".into()),
                        tags: vec![],
                        timestamp_ms: 0,
                    },
                    MetricPoint {
                        name: "disk.free".into(),
                        value: 50.0,
                        unit: Some("GB".into()),
                        tags: vec![],
                        timestamp_ms: 0,
                    },
                ],
            },
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);
        assert_eq!(broker.count(), 2);
    }

    #[tokio::test]
    async fn ring_buffer_eviction() {
        let broker = MetricsBroker::new(3);
        for i in 0..5 {
            let req = CapabilityRequest {
                capability_group: "ganglion:metrics/emit".into(),
                operation: BrokerOperation::MetricEmit {
                    name: format!("metric.{i}"),
                    value: i as f64,
                    unit: None,
                    tags: vec![],
                },
            };
            broker.handle_request(req).await.unwrap();
        }
        // Only the last 3 should remain
        assert_eq!(broker.count(), 3);
        let metrics = broker.drain();
        assert_eq!(metrics[0].name, "metric.2");
        assert_eq!(metrics[1].name, "metric.3");
        assert_eq!(metrics[2].name, "metric.4");
    }

    #[tokio::test]
    async fn drain_clears_store() {
        let broker = MetricsBroker::new(1000);
        let req = CapabilityRequest {
            capability_group: "ganglion:metrics/emit".into(),
            operation: BrokerOperation::MetricEmit {
                name: "test".into(),
                value: 1.0,
                unit: None,
                tags: vec![],
            },
        };
        broker.handle_request(req).await.unwrap();
        assert_eq!(broker.count(), 1);
        let _ = broker.drain();
        assert_eq!(broker.count(), 0);
    }

    #[test]
    fn lock_recovers_from_poison() {
        // CODE-24: a panic in another thread while holding the lock must not
        // make every later access panic on a poisoned mutex.
        let broker = MetricsBroker::new(10);
        let store = broker.store.clone();
        let _ = std::thread::spawn(move || {
            let _guard = store.lock().unwrap();
            panic!("poison the mutex");
        })
        .join();

        // These would panic if we used lock().unwrap().
        assert_eq!(broker.count(), 0);
        broker.store_metric(MetricPoint {
            name: "x".into(),
            value: 1.0,
            unit: None,
            tags: vec![],
            timestamp_ms: 0,
        });
        assert_eq!(broker.count(), 1);
    }

    #[tokio::test]
    async fn broker_rejects_unknown_op() {
        let broker = MetricsBroker::new(1000);
        let req = CapabilityRequest {
            capability_group: "ganglion:metrics/emit".into(),
            operation: BrokerOperation::SystemInfo,
        };
        assert!(broker.handle_request(req).await.is_err());
    }
}
