//! Host function registration — bridges WIT imports to CapabilityHost::broker_call().
//!
//! Each WIT interface in ganglion.wit maps to a set of host functions registered
//! on the Wasmtime component linker. When a WASM component calls an imported
//! function, the linker dispatches to the closure registered here, which:
//!
//! 1. Checks capability declaration via CapabilityHost::is_declared()
//! 2. Constructs the appropriate BrokerOperation variant
//! 3. Routes through CapabilityHost::broker_call()
//! 4. Marshals the response back into WIT-compatible Val types

use std::sync::Arc;

use anyhow::{Context, bail};
use wasmtime::StoreContextMut;
use wasmtime::component::{Linker, Val};

use gang_core::broker::{BrokerOperation, CapabilityBroker, CapabilityRequest, CapabilityResponse};

use crate::host::CapabilityHost;

/// WIT package version used for all ganglion capability interfaces.
const WIT_VERSION: &str = "0.5.0";

/// Register all ganglion capability host functions on the linker.
///
/// Each WIT interface (ros-interface, logs-stream, fs-bounded, etc.) gets an
/// instance registered with the linker. Functions within each instance route
/// calls through the store's CapabilityHost to the appropriate Layer 3 broker.
///
/// This is the glue layer that bridges Layer 2 (WASM sandbox) to Layer 3
/// (native brokers). Without these registrations, any WASM component that
/// imports a ganglion:* interface would fail at instantiation.
pub fn register_capability_imports(linker: &mut Linker<CapabilityHost>) -> anyhow::Result<()> {
    register_ros_interface(linker)?;
    register_logs_stream(linker)?;
    register_fs_bounded(linker)?;
    register_diagnostics_collect(linker)?;
    register_artifacts_publish(linker)?;
    register_process_spawn(linker)?;
    register_network_probe(linker)?;
    register_metrics_emit(linker)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper: call a broker through the CapabilityHost stored in the Wasmtime Store
// ---------------------------------------------------------------------------

/// Extract broker info from the store, perform the async broker call, and
/// return the response. Clones Arc<dyn CapabilityBroker> out of the store so
/// the borrow on StoreContextMut is released before the await point.
async fn call_broker(
    group: &str,
    declared: bool,
    broker: Option<Arc<dyn CapabilityBroker>>,
    operation: BrokerOperation,
) -> Result<CapabilityResponse, String> {
    if !declared {
        return Err(format!(
            "capability group '{group}' not declared in manifest — access denied"
        ));
    }
    let broker =
        broker.ok_or_else(|| format!("no broker registered for capability group '{group}'"))?;
    let req = CapabilityRequest {
        capability_group: group.to_string(),
        operation,
    };
    broker.handle_request(req).await.map_err(|e| e.to_string())
}

/// Set a WIT result<T, string> into the output Val slot.
/// On success, serializes response data as JSON bytes encoded into a Val.
/// On error, sets the error variant with the message.
fn set_result_bytes(results: &mut [Val], resp: Result<CapabilityResponse, String>) {
    match resp {
        Ok(r) if r.success => {
            results[0] = Val::Result(Ok(Some(Box::new(Val::List(
                r.data.into_iter().map(Val::U8).collect(),
            )))));
        }
        Ok(r) => {
            let msg = r.error.unwrap_or_else(|| "broker returned failure".into());
            results[0] = Val::Result(Err(Some(Box::new(Val::String(msg)))));
        }
        Err(e) => {
            results[0] = Val::Result(Err(Some(Box::new(Val::String(e)))));
        }
    }
}

/// Set a WIT result<list<string>, string> into the output Val slot.
///
/// The broker data is a JSON array; string elements are passed through and
/// non-string elements (e.g. process-list's structured entries) are rendered
/// as compact JSON so no information is silently dropped.
fn set_result_string_list(results: &mut [Val], resp: Result<CapabilityResponse, String>) {
    match resp {
        Ok(r) if r.success => {
            let items: Vec<String> = serde_json::from_slice::<serde_json::Value>(&r.data)
                .ok()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
                .iter()
                .map(json_scalar_string)
                .collect();
            results[0] = Val::Result(Ok(Some(Box::new(Val::List(
                items.into_iter().map(Val::String).collect(),
            )))));
        }
        Ok(r) => {
            let msg = r.error.unwrap_or_else(|| "broker returned failure".into());
            results[0] = Val::Result(Err(Some(Box::new(Val::String(msg)))));
        }
        Err(e) => {
            results[0] = Val::Result(Err(Some(Box::new(Val::String(e)))));
        }
    }
}

/// Set a WIT result<_, string> (unit success) into the output Val slot.
fn set_result_unit(results: &mut [Val], resp: Result<CapabilityResponse, String>) {
    match resp {
        Ok(r) if r.success => {
            results[0] = Val::Result(Ok(None));
        }
        Ok(r) => {
            let msg = r.error.unwrap_or_else(|| "broker returned failure".into());
            results[0] = Val::Result(Err(Some(Box::new(Val::String(msg)))));
        }
        Err(e) => {
            results[0] = Val::Result(Err(Some(Box::new(Val::String(e)))));
        }
    }
}

/// Set a WIT result<bool, string> into the output Val slot.
fn set_result_bool(results: &mut [Val], resp: Result<CapabilityResponse, String>) {
    match resp {
        Ok(r) if r.success => {
            let val: bool = serde_json::from_slice(&r.data).unwrap_or(false);
            results[0] = Val::Result(Ok(Some(Box::new(Val::Bool(val)))));
        }
        Ok(r) => {
            let msg = r.error.unwrap_or_else(|| "broker returned failure".into());
            results[0] = Val::Result(Err(Some(Box::new(Val::String(msg)))));
        }
        Err(e) => {
            results[0] = Val::Result(Err(Some(Box::new(Val::String(e)))));
        }
    }
}

/// Decode a successful broker response as JSON and convert it into the
/// function's declared WIT return shape via `convert`.
///
/// Wasmtime type-checks the result Vals of a dynamic (`func_new_async`) host
/// function against the calling component's expected type, so the Val MUST
/// match the WIT signature exactly. An earlier implementation returned JSON
/// bytes (`list<u8>`) for every record-typed result, which trapped the very
/// first host call of any toolchain-built component.
fn set_result_typed(
    results: &mut [Val],
    resp: Result<CapabilityResponse, String>,
    convert: impl FnOnce(&serde_json::Value) -> Result<Val, String>,
) {
    let outcome = match resp {
        Ok(r) if r.success => serde_json::from_slice::<serde_json::Value>(&r.data)
            .map_err(|e| format!("broker returned undecodable JSON: {e}"))
            .and_then(|v| convert(&v)),
        Ok(r) => Err(r.error.unwrap_or_else(|| "broker returned failure".into())),
        Err(e) => Err(e),
    };
    match outcome {
        Ok(val) => results[0] = Val::Result(Ok(Some(Box::new(val)))),
        Err(msg) => results[0] = Val::Result(Err(Some(Box::new(Val::String(msg))))),
    }
}

// ---------------------------------------------------------------------------
// JSON → typed Val converters (one per WIT record shape)
// ---------------------------------------------------------------------------

/// Build a `Val::Record` from (WIT-field-name, value) pairs. Field names are
/// the WIT kebab-case names, in WIT declaration order.
fn record_val(fields: &[(&str, Val)]) -> Val {
    Val::Record(
        fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

/// Render a JSON value as a display string: strings pass through unquoted,
/// everything else as compact JSON.
fn json_scalar_string(v: &serde_json::Value) -> String {
    match v.as_str() {
        Some(s) => s.to_string(),
        None => v.to_string(),
    }
}

fn json_field_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key).map(json_scalar_string).unwrap_or_default()
}

fn json_field_u64(v: &serde_json::Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

fn json_field_f64(v: &serde_json::Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

fn json_field_bool(v: &serde_json::Value, key: &str) -> bool {
    v.get(key).and_then(|x| x.as_bool()).unwrap_or(false)
}

fn json_field_byte_list(v: &serde_json::Value, key: &str) -> Val {
    Val::List(
        v.get(key)
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|b| b.as_u64().map(|n| Val::U8(n as u8)))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// JSON object → `list<diagnostic-entry>` (record { key, value }), one entry
/// per top-level field. Non-string values are rendered as compact JSON.
fn kv_entries_val(v: &serde_json::Value) -> Result<Val, String> {
    let obj = v
        .as_object()
        .ok_or_else(|| "broker returned non-object JSON for key/value entries".to_string())?;
    Ok(Val::List(
        obj.iter()
            .map(|(k, val)| {
                record_val(&[
                    ("key", Val::String(k.clone())),
                    ("value", Val::String(json_scalar_string(val))),
                ])
            })
            .collect(),
    ))
}

/// RosListing { topics, services, nodes } → `list<ros-entry>`
/// (record { name, entry-type }).
fn ros_entries_val(v: &serde_json::Value) -> Result<Val, String> {
    let mut out = Vec::new();
    for (field, kind) in [
        ("topics", "topic"),
        ("services", "service"),
        ("nodes", "node"),
    ] {
        if let Some(arr) = v.get(field).and_then(|a| a.as_array()) {
            for name in arr {
                out.push(record_val(&[
                    ("name", Val::String(json_scalar_string(name))),
                    ("entry-type", Val::String(kind.to_string())),
                ]));
            }
        }
    }
    Ok(Val::List(out))
}

/// Broker log-source array → `list<log-source>` (record { name, source-type }).
fn log_sources_val(v: &serde_json::Value) -> Result<Val, String> {
    let arr = v
        .as_array()
        .ok_or_else(|| "broker returned non-array JSON for log sources".to_string())?;
    Ok(Val::List(
        arr.iter()
            .map(|item| {
                record_val(&[
                    ("name", Val::String(json_field_str(item, "name"))),
                    (
                        "source-type",
                        Val::String(json_field_str(item, "source_type")),
                    ),
                ])
            })
            .collect(),
    ))
}

/// Broker FileStat → `file-stat` (record { size, is-dir, is-file, modified }).
/// The broker does not report mtime, so `modified` is 0.
fn file_stat_val(v: &serde_json::Value) -> Result<Val, String> {
    Ok(record_val(&[
        ("size", Val::U64(json_field_u64(v, "size"))),
        ("is-dir", Val::Bool(json_field_bool(v, "is_dir"))),
        ("is-file", Val::Bool(json_field_bool(v, "is_file"))),
        ("modified", Val::U64(json_field_u64(v, "modified"))),
    ]))
}

/// Broker ProcessOutput → `process-result`
/// (record { exit-code, stdout, stderr }).
fn process_result_val(v: &serde_json::Value) -> Result<Val, String> {
    let exit_code = v.get("exit_code").and_then(|x| x.as_i64()).unwrap_or(-1) as i32;
    Ok(record_val(&[
        ("exit-code", Val::S32(exit_code)),
        ("stdout", json_field_byte_list(v, "stdout")),
        ("stderr", json_field_byte_list(v, "stderr")),
    ]))
}

/// Broker PingResult → `ping-result`.
fn ping_result_val(v: &serde_json::Value) -> Result<Val, String> {
    Ok(record_val(&[
        ("host", Val::String(json_field_str(v, "host"))),
        ("reachable", Val::Bool(json_field_bool(v, "reachable"))),
        ("rtt-ms", Val::Float64(json_field_f64(v, "rtt_ms"))),
        (
            "packets-sent",
            Val::U32(json_field_u64(v, "packets_sent") as u32),
        ),
        (
            "packets-received",
            Val::U32(json_field_u64(v, "packets_received") as u32),
        ),
    ]))
}

/// Broker DnsResult → `dns-result`.
fn dns_result_val(v: &serde_json::Value) -> Result<Val, String> {
    let answers = v
        .get("answers")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .map(|s| Val::String(json_scalar_string(s)))
                .collect()
        })
        .unwrap_or_default();
    Ok(record_val(&[
        ("hostname", Val::String(json_field_str(v, "hostname"))),
        ("record-type", Val::String(json_field_str(v, "record_type"))),
        ("answers", Val::List(answers)),
    ]))
}

/// Broker PortResult → `port-result`.
fn port_result_val(v: &serde_json::Value) -> Result<Val, String> {
    Ok(record_val(&[
        ("host", Val::String(json_field_str(v, "host"))),
        ("port", Val::U16(json_field_u64(v, "port") as u16)),
        ("open", Val::Bool(json_field_bool(v, "open"))),
        ("latency-ms", Val::Float64(json_field_f64(v, "latency_ms"))),
    ]))
}

/// Broker traceroute hops → `list<traceroute-hop>`.
fn traceroute_hops_val(v: &serde_json::Value) -> Result<Val, String> {
    let arr = v
        .as_array()
        .ok_or_else(|| "broker returned non-array JSON for traceroute hops".to_string())?;
    Ok(Val::List(
        arr.iter()
            .map(|hop| {
                record_val(&[
                    ("hop", Val::U32(json_field_u64(hop, "hop") as u32)),
                    ("address", Val::String(json_field_str(hop, "address"))),
                    ("rtt-ms", Val::Float64(json_field_f64(hop, "rtt_ms"))),
                ])
            })
            .collect(),
    ))
}

// ---------------------------------------------------------------------------
// Macro to reduce boilerplate for async host function registration
// ---------------------------------------------------------------------------

/// Extract broker state from the store for a given capability group.
/// Returns (is_declared, Option<Arc<dyn CapabilityBroker>>).
fn extract_broker_state(
    caller: &StoreContextMut<'_, CapabilityHost>,
    group: &str,
) -> (bool, Option<Arc<dyn CapabilityBroker>>) {
    let host = caller.data();
    let declared = host.is_declared(group);
    let broker = host.get_broker(group).cloned();
    (declared, broker)
}

// ---------------------------------------------------------------------------
// ganglion:capability/ros-interface@0.5.0
// ---------------------------------------------------------------------------

fn register_ros_interface(linker: &mut Linker<CapabilityHost>) -> anyhow::Result<()> {
    let iface = format!("ganglion:capability/ros-interface@{WIT_VERSION}");
    let mut inst = linker
        .instance(&iface)
        .context("registering ros-interface")?;
    const GROUP: &str = "ganglion:ros/interface";

    // list-ros: func() -> result<list<u8>, string>
    // Returns JSON-serialized list of ros-entry records as bytes.
    inst.func_new_async("list-ros", |caller, _params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        Box::new(async move {
            let resp = call_broker(GROUP, declared, broker, BrokerOperation::RosList).await;
            set_result_typed(results, resp, ros_entries_val);
            Ok(())
        })
    })?;

    // topic-subscribe: func(topic: string) -> result<list<u8>, string>
    inst.func_new_async("topic-subscribe", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let topic = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string parameter for topic") }),
        };
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::TopicSubscribe { topic },
            )
            .await;
            set_result_bytes(results, resp);
            Ok(())
        })
    })?;

    // service-call: func(service: string, request: list<u8>) -> result<list<u8>, string>
    inst.func_new_async("service-call", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let service = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for service") }),
        };
        let request = extract_byte_list(&params[1]);
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::ServiceCall { service, request },
            )
            .await;
            set_result_bytes(results, resp);
            Ok(())
        })
    })?;

    // param-get: func(name: string) -> result<list<u8>, string>
    inst.func_new_async("param-get", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let name = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for param name") }),
        };
        Box::new(async move {
            let resp =
                call_broker(GROUP, declared, broker, BrokerOperation::ParamGet { name }).await;
            set_result_bytes(results, resp);
            Ok(())
        })
    })?;

    // param-set: func(name: string, value: list<u8>) -> result<bool, string>
    inst.func_new_async("param-set", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let name = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for param name") }),
        };
        let value = extract_byte_list(&params[1]);
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::ParamSet { name, value },
            )
            .await;
            set_result_bool(results, resp);
            Ok(())
        })
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// ganglion:capability/logs-stream@0.5.0
// ---------------------------------------------------------------------------

fn register_logs_stream(linker: &mut Linker<CapabilityHost>) -> anyhow::Result<()> {
    let iface = format!("ganglion:capability/logs-stream@{WIT_VERSION}");
    let mut inst = linker.instance(&iface).context("registering logs-stream")?;
    const GROUP: &str = "ganglion:logs/stream";

    // list-sources: func() -> result<list<u8>, string>
    inst.func_new_async("list-sources", |caller, _params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        Box::new(async move {
            let resp = call_broker(GROUP, declared, broker, BrokerOperation::LogSourceList).await;
            set_result_typed(results, resp, log_sources_val);
            Ok(())
        })
    })?;

    // stream-logs: func(source: string, pattern: string) -> result<list<string>, string>
    inst.func_new_async("stream-logs", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let source = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for source") }),
        };
        let pattern = match &params[1] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for pattern") }),
        };
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::LogStream { source, pattern },
            )
            .await;
            set_result_string_list(results, resp);
            Ok(())
        })
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// ganglion:capability/fs-bounded@0.5.0
// ---------------------------------------------------------------------------

fn register_fs_bounded(linker: &mut Linker<CapabilityHost>) -> anyhow::Result<()> {
    let iface = format!("ganglion:capability/fs-bounded@{WIT_VERSION}");
    let mut inst = linker.instance(&iface).context("registering fs-bounded")?;
    const GROUP: &str = "ganglion:fs/bounded";

    // read-file: func(path: string) -> result<list<u8>, string>
    inst.func_new_async("read-file", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let path = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for path") }),
        };
        Box::new(async move {
            let resp = call_broker(GROUP, declared, broker, BrokerOperation::FsRead { path }).await;
            set_result_bytes(results, resp);
            Ok(())
        })
    })?;

    // write-file: func(path: string, data: list<u8>) -> result<_, string>
    inst.func_new_async("write-file", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let path = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for path") }),
        };
        let data = extract_byte_list(&params[1]);
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::FsWrite { path, data },
            )
            .await;
            set_result_unit(results, resp);
            Ok(())
        })
    })?;

    // list-dir: func(path: string) -> result<list<string>, string>
    inst.func_new_async("list-dir", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let path = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for path") }),
        };
        Box::new(async move {
            let resp = call_broker(GROUP, declared, broker, BrokerOperation::FsList { path }).await;
            set_result_string_list(results, resp);
            Ok(())
        })
    })?;

    // stat-file: func(path: string) -> result<file-stat, string>
    // file-stat is serialized as JSON bytes for cross-boundary transfer.
    inst.func_new_async("stat-file", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let path = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for path") }),
        };
        Box::new(async move {
            let resp = call_broker(GROUP, declared, broker, BrokerOperation::FsStat { path }).await;
            set_result_typed(results, resp, file_stat_val);
            Ok(())
        })
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// ganglion:capability/diagnostics-collect@0.5.0
// ---------------------------------------------------------------------------

fn register_diagnostics_collect(linker: &mut Linker<CapabilityHost>) -> anyhow::Result<()> {
    let iface = format!("ganglion:capability/diagnostics-collect@{WIT_VERSION}");
    let mut inst = linker
        .instance(&iface)
        .context("registering diagnostics-collect")?;
    const GROUP: &str = "ganglion:diagnostics/collect";

    // system-info: func() -> result<list<u8>, string>
    inst.func_new_async("system-info", |caller, _params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        Box::new(async move {
            let resp = call_broker(GROUP, declared, broker, BrokerOperation::SystemInfo).await;
            set_result_typed(results, resp, kv_entries_val);
            Ok(())
        })
    })?;

    // process-list: func() -> result<list<string>, string>
    inst.func_new_async("process-list", |caller, _params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        Box::new(async move {
            let resp = call_broker(GROUP, declared, broker, BrokerOperation::ProcessList).await;
            set_result_string_list(results, resp);
            Ok(())
        })
    })?;

    // network-state: func() -> result<list<u8>, string>
    inst.func_new_async("network-state", |caller, _params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        Box::new(async move {
            let resp = call_broker(GROUP, declared, broker, BrokerOperation::NetworkState).await;
            set_result_typed(results, resp, kv_entries_val);
            Ok(())
        })
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// ganglion:capability/artifacts-publish@0.5.0
// ---------------------------------------------------------------------------

fn register_artifacts_publish(linker: &mut Linker<CapabilityHost>) -> anyhow::Result<()> {
    let iface = format!("ganglion:capability/artifacts-publish@{WIT_VERSION}");
    let mut inst = linker
        .instance(&iface)
        .context("registering artifacts-publish")?;
    const GROUP: &str = "ganglion:artifacts/publish";

    // publish: func(data: list<u8>, filename: option<string>, content-type: option<string>)
    //   -> result<string, string>
    inst.func_new_async("publish", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let data = extract_byte_list(&params[0]);
        let filename = extract_option_string(&params[1]);
        let content_type = extract_option_string(&params[2]);
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::ArtifactPublish {
                    data,
                    filename,
                    content_type,
                },
            )
            .await;
            // result<string, string> — success returns CID as string
            match resp {
                Ok(r) if r.success => {
                    let cid = String::from_utf8(r.data).unwrap_or_default();
                    results[0] = Val::Result(Ok(Some(Box::new(Val::String(cid)))));
                }
                Ok(r) => {
                    let msg = r.error.unwrap_or_else(|| "publish failed".into());
                    results[0] = Val::Result(Err(Some(Box::new(Val::String(msg)))));
                }
                Err(e) => {
                    results[0] = Val::Result(Err(Some(Box::new(Val::String(e)))));
                }
            }
            Ok(())
        })
    })?;

    // exists: func(cid: string) -> bool
    inst.func_new_async("exists", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let cid = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for cid") }),
        };
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::ArtifactExists { cid },
            )
            .await;
            // exists returns a plain bool, not a result
            let exists = resp
                .ok()
                .and_then(|r| serde_json::from_slice::<bool>(&r.data).ok())
                .unwrap_or(false);
            results[0] = Val::Bool(exists);
            Ok(())
        })
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// ganglion:capability/process-spawn@0.5.0
// ---------------------------------------------------------------------------

fn register_process_spawn(linker: &mut Linker<CapabilityHost>) -> anyhow::Result<()> {
    let iface = format!("ganglion:capability/process-spawn@{WIT_VERSION}");
    let mut inst = linker
        .instance(&iface)
        .context("registering process-spawn")?;
    const GROUP: &str = "ganglion:process/spawn";

    // spawn: func(command: string, args: list<string>, timeout-secs: u64)
    //   -> result<process-result, string>
    inst.func_new_async("spawn", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let command = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for command") }),
        };
        let args = extract_string_list(&params[1]);
        let timeout_secs = match &params[2] {
            Val::U64(n) => *n,
            _ => 30,
        };
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::ProcessSpawn {
                    command,
                    args,
                    cwd: None,
                    env: vec![],
                    timeout_secs,
                },
            )
            .await;
            set_result_typed(results, resp, process_result_val);
            Ok(())
        })
    })?;

    // spawn-with-env: func(command, args, env, cwd, timeout-secs)
    //   -> result<process-result, string>
    inst.func_new_async("spawn-with-env", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let command = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for command") }),
        };
        let args = extract_string_list(&params[1]);
        let env = extract_tuple_list(&params[2]);
        let cwd = extract_option_string(&params[3]);
        let timeout_secs = match &params[4] {
            Val::U64(n) => *n,
            _ => 30,
        };
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::ProcessSpawn {
                    command,
                    args,
                    cwd,
                    env,
                    timeout_secs,
                },
            )
            .await;
            set_result_typed(results, resp, process_result_val);
            Ok(())
        })
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// ganglion:capability/network-probe@0.5.0
// ---------------------------------------------------------------------------

fn register_network_probe(linker: &mut Linker<CapabilityHost>) -> anyhow::Result<()> {
    let iface = format!("ganglion:capability/network-probe@{WIT_VERSION}");
    let mut inst = linker
        .instance(&iface)
        .context("registering network-probe")?;
    const GROUP: &str = "ganglion:network/probe";

    // ping: func(host: string, count: u32) -> result<ping-result, string>
    inst.func_new_async("ping", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let host = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for host") }),
        };
        let count = match &params[1] {
            Val::U32(n) => *n,
            _ => 4,
        };
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::NetPing { host, count },
            )
            .await;
            set_result_typed(results, resp, ping_result_val);
            Ok(())
        })
    })?;

    // dns-lookup: func(hostname: string, record-type: string) -> result<dns-result, string>
    inst.func_new_async("dns-lookup", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let hostname = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for hostname") }),
        };
        let record_type = match &params[1] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for record-type") }),
        };
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::NetDnsLookup {
                    hostname,
                    record_type,
                },
            )
            .await;
            set_result_typed(results, resp, dns_result_val);
            Ok(())
        })
    })?;

    // port-check: func(host: string, port: u16, timeout-secs: u64) -> result<port-result, string>
    inst.func_new_async("port-check", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let host = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for host") }),
        };
        let port = match &params[1] {
            Val::U16(n) => *n,
            _ => return Box::new(async { bail!("expected u16 for port") }),
        };
        let timeout_secs = match &params[2] {
            Val::U64(n) => *n,
            _ => 10,
        };
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::NetPortCheck {
                    host,
                    port,
                    timeout_secs,
                },
            )
            .await;
            set_result_typed(results, resp, port_result_val);
            Ok(())
        })
    })?;

    // traceroute: func(host: string, max-hops: u32) -> result<list<traceroute-hop>, string>
    inst.func_new_async("traceroute", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let host = match &params[0] {
            Val::String(s) => s.clone(),
            _ => return Box::new(async { bail!("expected string for host") }),
        };
        let max_hops = match &params[1] {
            Val::U32(n) => *n,
            _ => 30,
        };
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::NetTraceroute { host, max_hops },
            )
            .await;
            set_result_typed(results, resp, traceroute_hops_val);
            Ok(())
        })
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// ganglion:capability/metrics-emit@0.5.0
// ---------------------------------------------------------------------------

fn register_metrics_emit(linker: &mut Linker<CapabilityHost>) -> anyhow::Result<()> {
    let iface = format!("ganglion:capability/metrics-emit@{WIT_VERSION}");
    let mut inst = linker
        .instance(&iface)
        .context("registering metrics-emit")?;
    const GROUP: &str = "ganglion:metrics/emit";

    // emit: func(metric: metric-point) -> result<_, string>
    // metric-point is received as a record Val; we extract fields manually.
    inst.func_new_async("emit", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let (name, value, unit, tags) = extract_metric_point(&params[0]);
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::MetricEmit {
                    name,
                    value,
                    unit,
                    tags,
                },
            )
            .await;
            set_result_unit(results, resp);
            Ok(())
        })
    })?;

    // emit-batch: func(metrics: list<metric-point>) -> result<_, string>
    inst.func_new_async("emit-batch", |caller, params, results| {
        let (declared, broker) = extract_broker_state(&caller, GROUP);
        let metrics = extract_metric_points_batch(&params[0]);
        Box::new(async move {
            let resp = call_broker(
                GROUP,
                declared,
                broker,
                BrokerOperation::MetricEmitBatch { metrics },
            )
            .await;
            set_result_unit(results, resp);
            Ok(())
        })
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Val extraction helpers
// ---------------------------------------------------------------------------

/// Extract a list<u8> from a Val::List.
fn extract_byte_list(val: &Val) -> Vec<u8> {
    match val {
        Val::List(items) => items
            .iter()
            .filter_map(|v| match v {
                Val::U8(b) => Some(*b),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Extract a list<string> from a Val::List.
fn extract_string_list(val: &Val) -> Vec<String> {
    match val {
        Val::List(items) => items
            .iter()
            .filter_map(|v| match v {
                Val::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Extract an option<string> from a Val::Option.
fn extract_option_string(val: &Val) -> Option<String> {
    match val {
        Val::Option(Some(inner)) => match inner.as_ref() {
            Val::String(s) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Extract a list<tuple<string, string>> from a Val::List.
fn extract_tuple_list(val: &Val) -> Vec<(String, String)> {
    match val {
        Val::List(items) => items
            .iter()
            .filter_map(|v| match v {
                Val::Tuple(fields) if fields.len() == 2 => {
                    let k = match &fields[0] {
                        Val::String(s) => s.clone(),
                        _ => return None,
                    };
                    let v = match &fields[1] {
                        Val::String(s) => s.clone(),
                        _ => return None,
                    };
                    Some((k, v))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Extract fields from a metric-point record Val.
/// metric-point { name: string, value: f64, unit: option<string>, tags: list<tuple<string, string>> }
fn extract_metric_point(val: &Val) -> (String, f64, Option<String>, Vec<(String, String)>) {
    match val {
        Val::Record(fields) => {
            let name = fields
                .iter()
                .find(|(k, _)| k == "name")
                .and_then(|(_, v)| match v {
                    Val::String(s) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let value = fields
                .iter()
                .find(|(k, _)| k == "value")
                .and_then(|(_, v)| match v {
                    Val::Float64(f) => Some(*f),
                    _ => None,
                })
                .unwrap_or(0.0);
            let unit = fields
                .iter()
                .find(|(k, _)| k == "unit")
                .and_then(|(_, v)| extract_option_string(v));
            let tags = fields
                .iter()
                .find(|(k, _)| k == "tags")
                .map(|(_, v)| extract_tuple_list(v))
                .unwrap_or_default();
            (name, value, unit, tags)
        }
        _ => (String::new(), 0.0, None, Vec::new()),
    }
}

/// Extract a list of metric-points for batch emission.
fn extract_metric_points_batch(val: &Val) -> Vec<gang_core::broker::MetricPoint> {
    match val {
        Val::List(items) => items
            .iter()
            .map(|v| {
                let (name, value, unit, tags) = extract_metric_point(v);
                gang_core::broker::MetricPoint {
                    name,
                    value,
                    unit,
                    tags,
                    timestamp_ms: 0, // host fills in current time
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::GanglionEngine;

    #[test]
    fn host_functions_register_without_error() {
        let engine = GanglionEngine::new().unwrap();
        let mut linker = Linker::<CapabilityHost>::new(engine.engine());
        let result = register_capability_imports(&mut linker);
        assert!(result.is_ok(), "registration failed: {result:?}");
    }

    #[tokio::test]
    async fn undeclared_capability_rejected_via_broker_call() {
        // Simulate what happens when a WASM component calls a host function
        // for a capability group that was not declared in its manifest.
        let resp = call_broker(
            "ganglion:diagnostics/collect",
            false, // not declared
            None,
            BrokerOperation::SystemInfo,
        )
        .await;

        assert!(resp.is_err());
        let msg = resp.unwrap_err();
        assert!(
            msg.contains("not declared"),
            "expected 'not declared' in error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn declared_capability_without_broker_returns_error() {
        // Capability is declared but no broker is registered — should return
        // a clear error rather than panicking.
        let resp = call_broker(
            "ganglion:diagnostics/collect",
            true, // declared
            None, // but no broker
            BrokerOperation::SystemInfo,
        )
        .await;

        assert!(resp.is_err());
        let msg = resp.unwrap_err();
        assert!(
            msg.contains("no broker registered"),
            "expected 'no broker registered' in error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn declared_capability_routes_through_broker() {
        use gang_core::broker::{CapabilityRequest, CapabilityResponse};
        use gang_core::error::BrokerError;

        /// A mock broker that always returns a fixed response.
        struct MockBroker;

        #[async_trait::async_trait]
        impl CapabilityBroker for MockBroker {
            async fn handle_request(
                &self,
                _req: CapabilityRequest,
            ) -> Result<CapabilityResponse, BrokerError> {
                Ok(CapabilityResponse {
                    success: true,
                    data: b"mock-data".to_vec(),
                    error: None,
                    bytes_in: 0,
                    bytes_out: 9,
                })
            }

            fn capability_group(&self) -> &str {
                "ganglion:diagnostics/collect"
            }
        }

        let broker: Arc<dyn CapabilityBroker> = Arc::new(MockBroker);

        let resp = call_broker(
            "ganglion:diagnostics/collect",
            true,
            Some(broker),
            BrokerOperation::SystemInfo,
        )
        .await;

        assert!(resp.is_ok());
        let r = resp.unwrap();
        assert!(r.success);
        assert_eq!(r.data, b"mock-data");
    }

    #[test]
    fn extract_byte_list_works() {
        let val = Val::List(vec![Val::U8(1), Val::U8(2), Val::U8(3)]);
        assert_eq!(extract_byte_list(&val), vec![1, 2, 3]);
    }

    #[test]
    fn extract_string_list_works() {
        let val = Val::List(vec![Val::String("a".into()), Val::String("b".into())]);
        assert_eq!(extract_string_list(&val), vec!["a", "b"]);
    }

    #[test]
    fn extract_option_string_none() {
        let val = Val::Option(None);
        assert_eq!(extract_option_string(&val), None);
    }

    #[test]
    fn extract_option_string_some() {
        let val = Val::Option(Some(Box::new(Val::String("hello".into()))));
        assert_eq!(extract_option_string(&val), Some("hello".into()));
    }
}
