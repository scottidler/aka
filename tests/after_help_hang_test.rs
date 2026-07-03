//! Regression test for the per-invocation daemon hang (design doc Phase 1).
//!
//! Before the fix, building `--help`'s `after_help` probed the daemon on EVERY
//! invocation - including every keystroke `aka query` - with an unbounded
//! `read_line`. A daemon (or any listener) that accepts connections but never
//! replies could wedge the shell forever.
//!
//! This test stands up a Unix socket at the exact path `aka` resolves and
//! spawns a listener that accepts connections but never writes a byte back.
//! `aka query foo` against that wedged socket must complete well under 2s
//! (previously it could hang indefinitely).

use std::fs;
use std::io::Read;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// Path to the debug `aka` binary, built by the test harness.
fn aka_binary() -> PathBuf {
    let output = Command::new("cargo")
        .args(["build", "--bin", "aka"])
        .output()
        .expect("Failed to build aka binary");
    assert!(
        output.status.success(),
        "Failed to build aka binary: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut path = std::env::current_dir().expect("Failed to get current dir");
    path.push("target");
    path.push("debug");
    path.push("aka");
    path
}

#[test]
fn query_completes_quickly_against_wedged_daemon_socket() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // aka resolves the socket as $XDG_RUNTIME_DIR/aka/daemon.sock.
    let runtime_dir = temp_dir.path().join("run");
    let socket_dir = runtime_dir.join("aka");
    fs::create_dir_all(&socket_dir).expect("Failed to create socket dir");
    let socket_path = socket_dir.join("daemon.sock");

    // A listener that accepts connections but NEVER responds - the wedge.
    let listener = UnixListener::bind(&socket_path).expect("Failed to bind wedged socket");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = Arc::clone(&stop);
    let accept_thread = thread::spawn(move || {
        // Keep accepted streams alive (holding, never replying) until stopped.
        let mut held = Vec::new();
        for stream in listener.incoming() {
            if stop_for_thread.load(Ordering::Relaxed) {
                break;
            }
            match stream {
                Ok(mut s) => {
                    // Drain whatever the client writes, but never write back.
                    let mut buf = [0u8; 64];
                    let _ = s.set_read_timeout(Some(Duration::from_millis(50)));
                    let _ = s.read(&mut buf);
                    held.push(s);
                }
                Err(_) => break,
            }
        }
    });

    // Minimal valid config.
    let config_dir = temp_dir.path().join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");
    let config_file = config_dir.join("aka.yml");
    fs::write(
        &config_file,
        r#"
lookups: {}

aliases:
  foo:
    value: echo "foo expanded"
    global: true
"#,
    )
    .expect("Failed to write config");

    let mut cmd = Command::new(aka_binary());
    cmd.args(["--config", config_file.to_str().unwrap(), "query", "foo"])
        .env("HOME", temp_dir.path())
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .env("AKA_CACHE_DIR", temp_dir.path().join("cache"))
        .env("AKA_LOG_FILE", temp_dir.path().join("aka.log"));

    let start = Instant::now();
    let mut child = cmd.spawn().expect("Failed to spawn aka query");

    // Poll for completion with a hard 2s ceiling; kill + fail on timeout.
    let deadline = Duration::from_secs(2);
    let completed = loop {
        match child.try_wait().expect("try_wait failed") {
            Some(_status) => break true,
            None => {
                if start.elapsed() >= deadline {
                    break false;
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
    };

    let elapsed = start.elapsed();

    // Tear the listener down regardless of outcome.
    stop.store(true, Ordering::Relaxed);
    if !completed {
        let _ = child.kill();
        let _ = child.wait();
    }
    // Nudge the accept loop out of its blocking accept() so the thread can exit.
    let _ = std::os::unix::net::UnixStream::connect(&socket_path);
    let _ = accept_thread.join();

    assert!(
        completed,
        "aka query hung against a wedged daemon socket (>{deadline:?}); \
         the after_help/health probe must be bounded"
    );
    assert!(
        elapsed < deadline,
        "aka query took {elapsed:?}, expected well under {deadline:?}"
    );
}
