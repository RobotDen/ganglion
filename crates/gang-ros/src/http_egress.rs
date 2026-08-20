//! `ganglion:http/egress` broker (ADR-025, #41) — the Layer 3 mechanics half
//! of URL-pattern-allowlisted outbound HTTP.
//!
//! The *authorization* half — is this URL + method covered by the calling
//! component's declared endpoints? — happens at the imports layer in
//! `gang-wasm-host` (via `gang_core::capability::http_request_permitted`),
//! where the caller's declaration is visible. This broker enforces what
//! remains true for every caller:
//!
//! - scheme is `http` or `https` and nothing else (which of the two a
//!   component may use is decided by its declared URL patterns);
//! - the response body is capped ([`MAX_RESPONSE_BYTES`]) — an oversized
//!   body is an error, not a truncation, so a component can never mistake a
//!   partial payload for the real one;
//! - the whole exchange is bounded by a wall-clock deadline
//!   ([`REQUEST_TIMEOUT`]);
//! - redirects are **never followed** — a 3xx is returned to the component,
//!   because following one could silently cross the URL allowlist;
//! - request body and headers are size-bounded, and hop-controlled headers
//!   (`host`, `content-length`, `transfer-encoding`, `connection`) cannot be
//!   overridden by the component.
//!
//! TLS terminates here, host-side (rustls); the component never holds a
//! socket. Credentials belong in credential slots (#43) — the component sets
//! its own `Authorization` header from the injected value; this broker treats
//! headers as opaque and never logs them.

use async_trait::async_trait;
use gang_core::broker::{
    BrokerOperation, CapabilityBroker, CapabilityRequest, CapabilityResponse, HttpResponseData,
};
use gang_core::error::BrokerError;
use std::time::Duration;

/// Maximum response body size. An over-limit response is an error.
pub const MAX_RESPONSE_BYTES: u64 = 256 * 1024;

/// Maximum request body size.
pub const MAX_REQUEST_BYTES: usize = 256 * 1024;

/// Whole-exchange deadline.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum number of request headers.
const MAX_HEADERS: usize = 32;

/// Maximum bytes for one header name + value.
const MAX_HEADER_BYTES: usize = 4 * 1024;

/// Headers the transport owns; component-supplied values are refused (not
/// silently dropped — a component relying on them should learn immediately).
const FORBIDDEN_HEADERS: &[&str] = &["host", "content-length", "transfer-encoding", "connection"];

/// Broker for `ganglion:http/egress`.
pub struct HttpEgressBroker {
    agent: ureq::Agent,
}

impl Default for HttpEgressBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpEgressBroker {
    /// Construct the broker with its bounded rustls-backed client.
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            // ADR-025: never follow redirects — a redirect could cross the
            // URL allowlist. The 3xx goes back to the component as data.
            .max_redirects(0)
            // 4xx/5xx are DATA for the component, not transport errors.
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.into(),
        }
    }

    /// Validate mechanics and perform the request. Blocking (ureq); the
    /// async trait method wraps this in `spawn_blocking`.
    fn perform(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
    ) -> Result<HttpResponseData, String> {
        // Scheme gate: nothing but http/https ever leaves this broker.
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(format!(
                "unsupported scheme (only http/https): {}",
                url.split(':').next().unwrap_or("?")
            ));
        }
        let method = method.to_ascii_uppercase();
        let known = matches!(
            method.as_str(),
            "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS"
        );
        if !known {
            return Err(format!("unsupported HTTP method: {method}"));
        }
        if body.len() > MAX_REQUEST_BYTES {
            return Err(format!(
                "request body exceeds {MAX_REQUEST_BYTES} byte cap ({} bytes)",
                body.len()
            ));
        }
        if headers.len() > MAX_HEADERS {
            return Err(format!("more than {MAX_HEADERS} request headers"));
        }
        for (name, value) in headers {
            if name.len() + value.len() > MAX_HEADER_BYTES {
                return Err(format!("header '{name}' exceeds {MAX_HEADER_BYTES} bytes"));
            }
            if FORBIDDEN_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                return Err(format!("header '{name}' is transport-owned and refused"));
            }
        }

        let mut builder = ureq::http::Request::builder()
            .method(
                ureq::http::Method::from_bytes(method.as_bytes())
                    .map_err(|e| format!("invalid method: {e}"))?,
            )
            .uri(url);
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        let request = builder
            .body(body.to_vec())
            .map_err(|e| format!("building request: {e}"))?;

        let mut response = self
            .agent
            .run(request)
            .map_err(|e| format!("request failed: {e}"))?;

        let status = response.status().as_u16();
        let resp_headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    v.to_str().unwrap_or("<non-utf8>").to_string(),
                )
            })
            .collect();
        // Size-capped read: limit + 1 so an at-limit body passes and an
        // over-limit body is detected as an error rather than truncated.
        let resp_body = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES + 1)
            .read_to_vec()
            .map_err(|e| format!("reading response body: {e}"))?;
        if resp_body.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(format!(
                "response body exceeds {MAX_RESPONSE_BYTES} byte cap"
            ));
        }

        Ok(HttpResponseData {
            status,
            headers: resp_headers,
            body: resp_body,
        })
    }
}

#[async_trait]
impl CapabilityBroker for HttpEgressBroker {
    async fn handle_request(
        &self,
        req: CapabilityRequest,
    ) -> Result<CapabilityResponse, BrokerError> {
        let BrokerOperation::HttpRequest {
            method,
            url,
            headers,
            body,
        } = req.operation
        else {
            return Err(BrokerError::AccessDenied {
                broker: "http-egress".into(),
                resource: "operation".into(),
                reason: "http/egress broker handles HttpRequest only".into(),
            });
        };

        let bytes_in = body.len() as u64;
        // ureq is blocking by design; run it off the async executor. The
        // agent is cheap to clone (internal Arc).
        let agent = self.agent.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let broker = HttpEgressBroker { agent };
            broker.perform(&method, &url, &headers, &body)
        })
        .await
        .map_err(|e| BrokerError::Unavailable {
            broker: "http-egress".into(),
            reason: format!("http task failed: {e}"),
        })?;

        match outcome {
            Ok(data) => {
                let bytes_out = data.body.len() as u64;
                let mut encoded = Vec::new();
                ciborium::into_writer(&data, &mut encoded).map_err(|e| {
                    BrokerError::Unavailable {
                        broker: "http-egress".into(),
                        reason: format!("encoding response: {e}"),
                    }
                })?;
                Ok(CapabilityResponse {
                    success: true,
                    data: encoded,
                    error: None,
                    bytes_in,
                    bytes_out,
                })
            }
            Err(msg) => Ok(CapabilityResponse {
                success: false,
                data: Vec::new(),
                error: Some(msg),
                bytes_in,
                bytes_out: 0,
            }),
        }
    }

    fn capability_group(&self) -> &str {
        "ganglion:http/egress"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Minimal single-request HTTP server on loopback: reads one request,
    /// answers with the given status/body, records the request head.
    fn one_shot_server(
        status_line: &'static str,
        extra_headers: &'static str,
        body: &'static [u8],
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap();
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            let response = format!(
                "HTTP/1.1 {status_line}\r\ncontent-length: {}\r\n{extra_headers}connection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
            head
        });
        (format!("http://{addr}"), handle)
    }

    fn op(method: &str, url: &str) -> CapabilityRequest {
        CapabilityRequest {
            capability_group: "ganglion:http/egress".into(),
            operation: BrokerOperation::HttpRequest {
                method: method.into(),
                url: url.into(),
                headers: vec![("x-test".into(), "1".into())],
                body: Vec::new(),
            },
        }
    }

    #[tokio::test]
    async fn get_roundtrip_with_headers_and_body() {
        let (base, server) = one_shot_server("200 OK", "x-served-by: test\r\n", b"hello");
        let broker = HttpEgressBroker::new();
        let resp = broker
            .handle_request(op("GET", &format!("{base}/v1/x")))
            .await
            .unwrap();
        assert!(resp.success, "{:?}", resp.error);
        let data: HttpResponseData = ciborium::from_reader(resp.data.as_slice()).unwrap();
        assert_eq!(data.status, 200);
        assert_eq!(data.body, b"hello");
        assert!(
            data.headers
                .iter()
                .any(|(k, v)| k == "x-served-by" && v == "test")
        );
        let head = server.join().unwrap();
        assert!(head.starts_with("GET /v1/x"));
        assert!(head.to_lowercase().contains("x-test: 1"));
    }

    #[tokio::test]
    async fn error_statuses_are_data_not_errors() {
        let (base, _server) = one_shot_server("503 Service Unavailable", "", b"nope");
        let broker = HttpEgressBroker::new();
        let resp = broker.handle_request(op("GET", &base)).await.unwrap();
        assert!(resp.success);
        let data: HttpResponseData = ciborium::from_reader(resp.data.as_slice()).unwrap();
        assert_eq!(data.status, 503);
    }

    #[tokio::test]
    async fn redirects_are_returned_not_followed() {
        let (base, server) = one_shot_server(
            "302 Found",
            "location: http://255.255.255.255/evil\r\n",
            b"",
        );
        let broker = HttpEgressBroker::new();
        let resp = broker.handle_request(op("GET", &base)).await.unwrap();
        assert!(resp.success, "{:?}", resp.error);
        let data: HttpResponseData = ciborium::from_reader(resp.data.as_slice()).unwrap();
        assert_eq!(data.status, 302, "3xx must come back as data");
        server.join().unwrap(); // exactly one request reached the server
    }

    #[tokio::test]
    async fn oversized_response_is_an_error_not_a_truncation() {
        static BIG: &[u8] = &[b'x'; (MAX_RESPONSE_BYTES + 10) as usize];
        let (base, _server) = one_shot_server("200 OK", "", BIG);
        let broker = HttpEgressBroker::new();
        let resp = broker.handle_request(op("GET", &base)).await.unwrap();
        assert!(!resp.success);
        // ureq's reader limit fires first ("body exceeds limit"), our own
        // check is the backstop ("byte cap") — either way it is an ERROR,
        // never a truncated success.
        let msg = resp.error.unwrap();
        assert!(
            msg.contains("exceeds") || msg.contains("larger than"),
            "unexpected error: {msg}"
        );
    }

    #[tokio::test]
    async fn scheme_method_and_header_hygiene() {
        let broker = HttpEgressBroker::new();
        // Non-http scheme refused before any I/O.
        let resp = broker
            .handle_request(op("GET", "ftp://example.com/x"))
            .await
            .unwrap();
        assert!(!resp.success);
        assert!(resp.error.unwrap().contains("unsupported scheme"));
        // Unknown method refused.
        let resp = broker
            .handle_request(op("BREW", "http://127.0.0.1:1/x"))
            .await
            .unwrap();
        assert!(!resp.success);
        assert!(resp.error.unwrap().contains("unsupported HTTP method"));
        // Transport-owned header refused.
        let req = CapabilityRequest {
            capability_group: "ganglion:http/egress".into(),
            operation: BrokerOperation::HttpRequest {
                method: "GET".into(),
                url: "http://127.0.0.1:1/x".into(),
                headers: vec![("Host".into(), "evil".into())],
                body: Vec::new(),
            },
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(!resp.success);
        assert!(resp.error.unwrap().contains("transport-owned"));
    }
}
