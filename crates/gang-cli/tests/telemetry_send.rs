//! `gang telemetry show` and `gang telemetry status` must never send.
//!
//! The telemetry client honours `ALL_PROXY`/`HTTPS_PROXY`, so each run points
//! them at a local listener that counts connection attempts (and answers
//! none, so nothing ever leaves the machine). The state is primed so a send
//! is due; an allowlisted command is the positive control that the counter
//! sees a send.
#![cfg(feature = "telemetry")]

use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// A local "proxy" that accepts and drops connections, counting them.
fn counting_proxy() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&count);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if stream.is_ok() {
                seen.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    (addr, count)
}

/// A GANG_HOME whose notice was shown on an earlier day, so the next
/// allowlisted command is due to send.
fn primed_home() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("telemetry");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("notice-shown"), "1").unwrap();
    std::fs::write(state.join("last-check"), "2000-01-01").unwrap();
    tmp
}

/// Run `gang <args>` with telemetry enabled, through the counting proxy.
fn gang(home: &Path, proxy: &str, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_gang"))
        .args(args)
        .env("GANG_HOME", home)
        // Every opt-out layer cleared, or nothing would ever be due.
        .env_remove("DO_NOT_TRACK")
        .env_remove("GANG_TELEMETRY")
        .env_remove("CI")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env("ALL_PROXY", proxy)
        .env("HTTPS_PROXY", proxy)
        .env("https_proxy", proxy)
        .output()
        .expect("spawning `gang`");
    assert!(
        out.status.success(),
        "`gang {}` failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Wait briefly for the listener thread to count a connection.
fn connections_after(count: &AtomicUsize, wait: Duration) -> usize {
    let deadline = Instant::now() + wait;
    while count.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    count.load(Ordering::SeqCst)
}

#[test]
fn telemetry_show_and_status_never_send() {
    for sub in ["show", "status"] {
        let home = primed_home();
        let (proxy, count) = counting_proxy();

        gang(home.path(), &proxy, &["telemetry", sub]);

        assert_eq!(
            connections_after(&count, Duration::from_millis(500)),
            0,
            "`gang telemetry {sub}` attempted a send"
        );
        assert_eq!(
            std::fs::read_to_string(home.path().join("telemetry/last-check")).unwrap(),
            "2000-01-01",
            "`gang telemetry {sub}` ran the daily checkpoint"
        );
    }
}

#[test]
fn an_allowlisted_command_does_send_through_the_counter() {
    // Positive control: without it, a broken counter would make the test
    // above pass vacuously.
    let home = primed_home();
    let (proxy, count) = counting_proxy();

    gang(home.path(), &proxy, &["config", "path"]);

    assert!(
        connections_after(&count, Duration::from_secs(3)) >= 1,
        "the allowlisted command's send never reached the counting proxy"
    );
}
