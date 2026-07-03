//! Direct probe tests for `aka_lib::probe_daemon_health` (design doc Phase 2).
//!
//! Phase 2 routes the CLI's `--help` status emoji and the router's health check
//! through the single, bounded `probe_daemon_health`. These tests exercise it
//! directly against real Unix sockets:
//!   - happy path: a well-formed `healthy:<n>:synced` / `:stale` -> Synced/Stale,
//!   - strict parse: the legacy `healthy:<n>:aliases` shape -> Unhealthy,
//!   - bounded completion: a listener that accepts but never replies must not
//!     hang; the probe returns quickly with Unreachable,
//!   - closed/absent socket -> Unreachable.

use aka_lib::{probe_daemon_health, DaemonHealth};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// Spawn a one-shot listener that accepts a single connection, reads the request
/// line, and writes back the given canned Health status. Returns immediately;
/// the accept happens on a background thread.
fn spawn_responder(listener: UnixListener, status: &'static str) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let response = format!(r#"{{"type":"Health","status":"{status}"}}"#);
            let mut w = &stream;
            let _ = writeln!(w, "{response}");
            let _ = w.flush();
        }
    })
}

#[test]
fn test_probe_daemon_health_synced() {
    let dir = TempDir::new().unwrap();
    let socket_path = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();
    let handle = spawn_responder(listener, "healthy:3:synced");

    let health = probe_daemon_health(&socket_path);
    assert_eq!(health, DaemonHealth::Synced { aliases: 3 });
    let _ = handle.join();
}

#[test]
fn test_probe_daemon_health_stale() {
    let dir = TempDir::new().unwrap();
    let socket_path = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();
    let handle = spawn_responder(listener, "healthy:7:stale");

    let health = probe_daemon_health(&socket_path);
    assert_eq!(health, DaemonHealth::Stale { aliases: 7 });
    let _ = handle.join();
}

#[test]
fn test_probe_daemon_health_strict_parse_rejects_legacy_aliases_shape() {
    // The legacy `healthy:<n>:aliases` payload is not a valid synced/stale status
    // and must be treated as Unhealthy by the strict parser.
    let dir = TempDir::new().unwrap();
    let socket_path = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();
    let handle = spawn_responder(listener, "healthy:5:aliases");

    let health = probe_daemon_health(&socket_path);
    assert_eq!(health, DaemonHealth::Unhealthy);
    let _ = handle.join();
}

#[test]
fn test_probe_daemon_health_never_responds_is_bounded_and_unreachable() {
    // A listener that accepts but never writes must not hang the probe. It must
    // complete well under 2s (the read timeout fires) and report Unreachable.
    let dir = TempDir::new().unwrap();
    let socket_path = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let handle = thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            // Hold the connection open without ever replying, long enough that
            // the client's bounded read must time out first.
            thread::sleep(Duration::from_millis(700));
            drop(stream);
        }
    });

    let start = Instant::now();
    let health = probe_daemon_health(&socket_path);
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(2),
        "probe must be bounded, took {elapsed:?}"
    );
    assert_eq!(health, DaemonHealth::Unreachable);
    let _ = handle.join();
}

#[test]
fn test_probe_daemon_health_absent_socket_is_unreachable() {
    let dir = TempDir::new().unwrap();
    // Path that was never bound - no socket file exists here.
    let socket_path = dir.path().join("does-not-exist.sock");

    let health = probe_daemon_health(&socket_path);
    assert_eq!(health, DaemonHealth::Unreachable);
}
