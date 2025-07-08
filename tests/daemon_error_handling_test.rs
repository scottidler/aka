use std::fs;
use std::time::Duration;
use tempfile::TempDir;
use aka_lib::{
    DaemonError,
    validate_socket_path,
    should_retry_daemon_error,
    categorize_daemon_error,
    DAEMON_CONNECTION_TIMEOUT_MS,
    DAEMON_READ_TIMEOUT_MS,
    DAEMON_WRITE_TIMEOUT_MS,
    DAEMON_TOTAL_TIMEOUT_MS,
    DAEMON_RETRY_DELAY_MS,
    DAEMON_MAX_RETRIES,
};

/// Test that timeout constants are set for CLI performance
#[test]
fn test_timeout_constants_are_aggressive() {
    // Verify aggressive timeout values for CLI performance
    assert_eq!(DAEMON_CONNECTION_TIMEOUT_MS, 100, "Connection timeout should be 100ms");
    assert_eq!(DAEMON_READ_TIMEOUT_MS, 200, "Read timeout should be 200ms");
    assert_eq!(DAEMON_WRITE_TIMEOUT_MS, 50, "Write timeout should be 50ms");
    assert_eq!(DAEMON_TOTAL_TIMEOUT_MS, 300, "Total timeout should be 300ms");
    assert_eq!(DAEMON_RETRY_DELAY_MS, 50, "Retry delay should be 50ms");
    assert_eq!(DAEMON_MAX_RETRIES, 1, "Should only retry once");
}

/// Test that DaemonError implements proper Display and Debug traits
#[test]
fn test_daemon_error_display() {
    let errors = vec![
        (DaemonError::ConnectionTimeout, "Daemon connection timeout"),
        (DaemonError::ReadTimeout, "Daemon read timeout"),
        (DaemonError::WriteTimeout, "Daemon write timeout"),
        (DaemonError::ConnectionRefused, "Daemon connection refused"),
        (DaemonError::SocketNotFound, "Daemon socket not found"),
        (DaemonError::SocketPermissionDenied, "Daemon socket permission denied"),
        (DaemonError::ProtocolError("test".to_string()), "Daemon protocol error: test"),
        (DaemonError::DaemonShutdown, "Daemon is shutting down"),
        (DaemonError::TotalOperationTimeout, "Total daemon operation timeout"),
        (DaemonError::UnknownError("test".to_string()), "Unknown daemon error: test"),
    ];

    for (error, expected_message) in errors {
        assert_eq!(error.to_string(), expected_message);
        // Also verify Debug trait works
        let debug_str = format!("{:?}", error);
        assert!(!debug_str.is_empty());
    }
}

/// Test retry logic for different error types
#[test]
fn test_should_retry_daemon_error() {
    // Errors that should be retried
    assert!(should_retry_daemon_error(&DaemonError::ConnectionTimeout));
    assert!(should_retry_daemon_error(&DaemonError::ConnectionRefused));

    // Errors that should NOT be retried
    assert!(!should_retry_daemon_error(&DaemonError::ReadTimeout));
    assert!(!should_retry_daemon_error(&DaemonError::WriteTimeout));
    assert!(!should_retry_daemon_error(&DaemonError::SocketNotFound));
    assert!(!should_retry_daemon_error(&DaemonError::SocketPermissionDenied));
    assert!(!should_retry_daemon_error(&DaemonError::ProtocolError("test".to_string())));
    assert!(!should_retry_daemon_error(&DaemonError::DaemonShutdown));
    assert!(!should_retry_daemon_error(&DaemonError::TotalOperationTimeout));
    assert!(!should_retry_daemon_error(&DaemonError::UnknownError("test".to_string())));
}

/// Test error categorization from std::io::Error
#[test]
fn test_categorize_daemon_error() {
    use std::io::{Error, ErrorKind};

    let test_cases = vec![
        (ErrorKind::TimedOut, DaemonError::ConnectionTimeout),
        (ErrorKind::ConnectionRefused, DaemonError::ConnectionRefused),
        (ErrorKind::NotFound, DaemonError::SocketNotFound),
        (ErrorKind::PermissionDenied, DaemonError::SocketPermissionDenied),
        (ErrorKind::WouldBlock, DaemonError::ReadTimeout),
    ];

    for (error_kind, expected_daemon_error) in test_cases {
        let io_error = Error::new(error_kind, "test error");
        let daemon_error = categorize_daemon_error(&io_error);

        // Compare the discriminant since we can't easily compare the content
        match (&daemon_error, &expected_daemon_error) {
            (DaemonError::ConnectionTimeout, DaemonError::ConnectionTimeout) => {},
            (DaemonError::ConnectionRefused, DaemonError::ConnectionRefused) => {},
            (DaemonError::SocketNotFound, DaemonError::SocketNotFound) => {},
            (DaemonError::SocketPermissionDenied, DaemonError::SocketPermissionDenied) => {},
            (DaemonError::ReadTimeout, DaemonError::ReadTimeout) => {},
            (DaemonError::UnknownError(_), DaemonError::UnknownError(_)) => {},
            _ => panic!("Error categorization mismatch: got {:?}, expected {:?}", daemon_error, expected_daemon_error),
        }
    }

    // Test unknown error kind
    let unknown_error = Error::new(ErrorKind::Other, "unknown error");
    let daemon_error = categorize_daemon_error(&unknown_error);
    match daemon_error {
        DaemonError::UnknownError(msg) => assert!(msg.contains("unknown error")),
        _ => panic!("Expected UnknownError for Other error kind"),
    }
}

/// Test socket path validation
#[test]
fn test_validate_socket_path() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Test non-existent socket
    let non_existent = temp_dir.path().join("non_existent_socket");
    let result = validate_socket_path(&non_existent);
    assert!(matches!(result, Err(DaemonError::SocketNotFound)));

    // Test regular file (not a socket)
    let regular_file = temp_dir.path().join("regular_file");
    fs::write(&regular_file, "test content").expect("Failed to write regular file");
    let result = validate_socket_path(&regular_file);
    assert!(matches!(result, Err(DaemonError::SocketNotFound)));

    // We can't easily test actual socket validation in unit tests since
    // creating Unix sockets requires more complex setup
}

/// Test timeout timing accuracy
#[test]
fn test_timeout_timing_accuracy() {
    // Test that timeout constants translate to correct Duration values
    let connection_timeout = Duration::from_millis(DAEMON_CONNECTION_TIMEOUT_MS);
    let read_timeout = Duration::from_millis(DAEMON_READ_TIMEOUT_MS);
    let write_timeout = Duration::from_millis(DAEMON_WRITE_TIMEOUT_MS);
    let total_timeout = Duration::from_millis(DAEMON_TOTAL_TIMEOUT_MS);
    let retry_delay = Duration::from_millis(DAEMON_RETRY_DELAY_MS);

    assert_eq!(connection_timeout.as_millis(), 100);
    assert_eq!(read_timeout.as_millis(), 200);
    assert_eq!(write_timeout.as_millis(), 50);
    assert_eq!(total_timeout.as_millis(), 300);
    assert_eq!(retry_delay.as_millis(), 50);

    // Verify total timeout is reasonable for CLI operations
    assert!(total_timeout.as_millis() <= 300, "Total timeout should be 300ms or less for CLI performance");
}

/// Test that retry logic respects max retries
#[test]
fn test_retry_limits() {
    // Test that we don't retry more than DAEMON_MAX_RETRIES times
    assert_eq!(DAEMON_MAX_RETRIES, 1, "Should only retry once for fast CLI response");

    // Test that retry delay is fast enough for CLI
    let retry_delay = Duration::from_millis(DAEMON_RETRY_DELAY_MS);
    assert!(retry_delay.as_millis() <= 50, "Retry delay should be 50ms or less");
}

/// Test error handling preserves context
#[test]
fn test_error_context_preservation() {
    let protocol_error = DaemonError::ProtocolError("Invalid JSON response".to_string());
    let error_message = protocol_error.to_string();
    assert!(error_message.contains("Invalid JSON response"));

    let unknown_error = DaemonError::UnknownError("Connection reset by peer".to_string());
    let error_message = unknown_error.to_string();
    assert!(error_message.contains("Connection reset by peer"));
}

/// Test that timing constraints are realistic for interactive use
#[test]
fn test_timing_constraints_for_interactive_use() {
    // Total operation should complete within 300ms for good UX
    let total_time = DAEMON_CONNECTION_TIMEOUT_MS + DAEMON_WRITE_TIMEOUT_MS + DAEMON_READ_TIMEOUT_MS;
    assert!(total_time <= 350, "Single attempt should complete within 350ms");

    // With retry, total time should still be reasonable
    let total_with_retry = total_time + DAEMON_RETRY_DELAY_MS + total_time;
    assert!(total_with_retry <= 750, "Total with retry should complete within 750ms");

    // But we enforce a hard limit
    assert!(DAEMON_TOTAL_TIMEOUT_MS <= 300, "Hard limit should be 300ms for CLI responsiveness");
}

/// Test error type categorization for fallback decisions
#[test]
fn test_error_categorization_for_fallback() {
    // These errors should trigger immediate fallback to direct mode
    let non_retryable_errors = vec![
        DaemonError::ReadTimeout,
        DaemonError::WriteTimeout,
        DaemonError::SocketNotFound,
        DaemonError::SocketPermissionDenied,
        DaemonError::ProtocolError("test".to_string()),
        DaemonError::DaemonShutdown,
        DaemonError::TotalOperationTimeout,
        DaemonError::UnknownError("test".to_string()),
    ];

    for error in non_retryable_errors {
        assert!(!should_retry_daemon_error(&error),
            "Error {:?} should not be retried and should trigger immediate fallback", error);
    }

    // These errors should be retried once before fallback
    let retryable_errors = vec![
        DaemonError::ConnectionTimeout,
        DaemonError::ConnectionRefused,
    ];

    for error in retryable_errors {
        assert!(should_retry_daemon_error(&error),
            "Error {:?} should be retried before fallback", error);
    }
}

/// Test that DaemonError implements std::error::Error trait
#[test]
fn test_daemon_error_is_std_error() {
    let error = DaemonError::ConnectionTimeout;

    // Should implement std::error::Error
    fn check_error_trait<T: std::error::Error>(_: &T) {}
    check_error_trait(&error);

    // Should implement Display
    fn check_display_trait<T: std::fmt::Display>(_: &T) {}
    check_display_trait(&error);

    // Should implement Debug
    fn check_debug_trait<T: std::fmt::Debug>(_: &T) {}
    check_debug_trait(&error);
}

/// Test timeout values are optimized for different scenarios
#[test]
fn test_timeout_optimization() {
    // Connection timeout should be short - if daemon isn't responding quickly, fall back
    assert!(DAEMON_CONNECTION_TIMEOUT_MS <= 100, "Connection should be fast or fail fast");

    // Write timeout should be very short - writing request should be nearly instant
    assert!(DAEMON_WRITE_TIMEOUT_MS <= 50, "Writing request should be nearly instant");

    // Read timeout can be slightly longer - daemon might need time to process
    assert!(DAEMON_READ_TIMEOUT_MS <= 200, "Read timeout should still be fast for CLI");

    // Total timeout should be aggressive for CLI responsiveness
    assert!(DAEMON_TOTAL_TIMEOUT_MS <= 300, "Total operation should complete quickly");
}

/// Test that error messages are user-friendly
#[test]
fn test_error_messages_are_user_friendly() {
    let errors = vec![
        DaemonError::ConnectionTimeout,
        DaemonError::ReadTimeout,
        DaemonError::WriteTimeout,
        DaemonError::ConnectionRefused,
        DaemonError::SocketNotFound,
        DaemonError::SocketPermissionDenied,
        DaemonError::DaemonShutdown,
        DaemonError::TotalOperationTimeout,
    ];

    for error in errors {
        let message = error.to_string();

        // Should be lowercase and descriptive
        assert!(message.starts_with("Daemon") || message.starts_with("Total"));
        assert!(!message.is_empty());

        // Should not contain technical jargon that confuses users
        assert!(!message.contains("errno"));
        assert!(!message.contains("syscall"));
        assert!(!message.contains("fd"));
    }
}
