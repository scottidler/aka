//! Daemon client with dependency injection for testability
//!
//! This module provides a testable daemon client that uses trait abstractions
//! for external dependencies (socket connections).

use crate::protocol::{DaemonRequest, DaemonResponse};
use crate::system::{RealSocketConnector, SocketConnector};
use log::debug;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::time::{Duration, Instant};

// Default timeouts
const DEFAULT_CONNECTION_TIMEOUT_MS: u64 = 100;
const DEFAULT_READ_TIMEOUT_MS: u64 = 200;
const DEFAULT_WRITE_TIMEOUT_MS: u64 = 50;
const DEFAULT_TOTAL_TIMEOUT_MS: u64 = 300;
const DEFAULT_RETRY_DELAY_MS: u64 = 50;
const DEFAULT_MAX_RETRIES: u32 = 1;

/// Configuration for daemon client timeouts and retries
#[derive(Debug, Clone)]
pub struct DaemonClientConfig {
    pub connection_timeout_ms: u64,
    pub read_timeout_ms: u64,
    pub write_timeout_ms: u64,
    pub total_timeout_ms: u64,
    pub retry_delay_ms: u64,
    pub max_retries: u32,
}

impl Default for DaemonClientConfig {
    fn default() -> Self {
        Self {
            connection_timeout_ms: DEFAULT_CONNECTION_TIMEOUT_MS,
            read_timeout_ms: DEFAULT_READ_TIMEOUT_MS,
            write_timeout_ms: DEFAULT_WRITE_TIMEOUT_MS,
            total_timeout_ms: DEFAULT_TOTAL_TIMEOUT_MS,
            retry_delay_ms: DEFAULT_RETRY_DELAY_MS,
            max_retries: DEFAULT_MAX_RETRIES,
        }
    }
}

/// Error types for daemon client operations
#[derive(Debug, Clone, PartialEq)]
pub enum DaemonError {
    ConnectionTimeout,
    ReadTimeout,
    WriteTimeout,
    ConnectionRefused,
    SocketNotFound,
    SocketPermissionDenied,
    ProtocolError(String),
    DaemonShutdown,
    TotalOperationTimeout,
    UnknownError(String),
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonError::ConnectionTimeout => write!(f, "Daemon connection timeout"),
            DaemonError::ReadTimeout => write!(f, "Daemon read timeout"),
            DaemonError::WriteTimeout => write!(f, "Daemon write timeout"),
            DaemonError::ConnectionRefused => write!(f, "Daemon connection refused"),
            DaemonError::SocketNotFound => write!(f, "Daemon socket not found"),
            DaemonError::SocketPermissionDenied => write!(f, "Daemon socket permission denied"),
            DaemonError::ProtocolError(msg) => write!(f, "Daemon protocol error: {msg}"),
            DaemonError::DaemonShutdown => write!(f, "Daemon is shutting down"),
            DaemonError::TotalOperationTimeout => write!(f, "Total daemon operation timeout"),
            DaemonError::UnknownError(msg) => write!(f, "Unknown daemon error: {msg}"),
        }
    }
}

impl std::error::Error for DaemonError {}

/// Determines if an error should trigger a retry
pub fn should_retry(error: &DaemonError) -> bool {
    matches!(error, DaemonError::ConnectionTimeout | DaemonError::ConnectionRefused)
}

/// Categorizes an IO error into a DaemonError
pub fn categorize_io_error(error: &std::io::Error) -> DaemonError {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::TimedOut => DaemonError::ConnectionTimeout,
        ErrorKind::ConnectionRefused => DaemonError::ConnectionRefused,
        ErrorKind::NotFound => DaemonError::SocketNotFound,
        ErrorKind::PermissionDenied => DaemonError::SocketPermissionDenied,
        ErrorKind::WouldBlock => DaemonError::ReadTimeout,
        _ => DaemonError::UnknownError(error.to_string()),
    }
}

/// Daemon client with injectable socket connector
pub struct DaemonClient<C: SocketConnector> {
    connector: C,
    config: DaemonClientConfig,
}

impl DaemonClient<RealSocketConnector> {
    /// Create a new daemon client with real socket connector
    pub fn new() -> Self {
        Self {
            connector: RealSocketConnector,
            config: DaemonClientConfig::default(),
        }
    }

    /// Create a new daemon client with custom config
    pub fn with_config(config: DaemonClientConfig) -> Self {
        Self {
            connector: RealSocketConnector,
            config,
        }
    }
}

impl Default for DaemonClient<RealSocketConnector> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: SocketConnector> DaemonClient<C> {
    /// Create a new daemon client with a custom connector (for testing)
    pub fn with_connector(connector: C) -> Self {
        Self {
            connector,
            config: DaemonClientConfig::default(),
        }
    }

    /// Create with both custom connector and config
    pub fn with_connector_and_config(connector: C, config: DaemonClientConfig) -> Self {
        Self { connector, config }
    }

    /// Validate that the socket path exists and is a socket
    pub fn validate_socket_path(&self, socket_path: &Path) -> Result<(), DaemonError> {
        if !self.connector.path_exists(socket_path) {
            return Err(DaemonError::SocketNotFound);
        }

        match self.connector.is_socket(socket_path) {
            Ok(true) => Ok(()),
            Ok(false) => Err(DaemonError::SocketNotFound),
            Err(e) => Err(categorize_io_error(&e)),
        }
    }

    /// Send a request to the daemon with retries and timeout handling
    pub fn send_request(&self, request: DaemonRequest, socket_path: &Path) -> Result<DaemonResponse, DaemonError> {
        let operation_start = Instant::now();
        let total_timeout = Duration::from_millis(self.config.total_timeout_ms);

        // Pre-validate socket before attempting connection
        self.validate_socket_path(socket_path)?;

        let mut last_error = None;

        for attempt in 0..=self.config.max_retries {
            // Check total operation timeout
            if operation_start.elapsed() >= total_timeout {
                debug!(
                    "🚨 Total daemon operation timeout exceeded: {}ms",
                    operation_start.elapsed().as_millis()
                );
                return Err(DaemonError::TotalOperationTimeout);
            }

            if attempt > 0 {
                debug!(
                    "🔄 Daemon retry attempt {} after {}ms",
                    attempt,
                    operation_start.elapsed().as_millis()
                );
                std::thread::sleep(Duration::from_millis(self.config.retry_delay_ms));
            }

            match self.attempt_single_request(&request, socket_path, &operation_start, &total_timeout) {
                Ok(response) => {
                    debug!(
                        "✅ Daemon request succeeded on attempt {} in {}ms",
                        attempt + 1,
                        operation_start.elapsed().as_millis()
                    );
                    return Ok(response);
                }
                Err(error) => {
                    debug!("❌ Daemon attempt {} failed: {}", attempt + 1, error);

                    // Check if we should retry this error type
                    if !should_retry(&error) || attempt >= self.config.max_retries {
                        return Err(error);
                    }

                    last_error = Some(error);
                }
            }
        }

        Err(last_error.unwrap_or(DaemonError::UnknownError("All retry attempts failed".to_string())))
    }

    fn attempt_single_request(
        &self,
        request: &DaemonRequest,
        socket_path: &Path,
        operation_start: &Instant,
        total_timeout: &Duration,
    ) -> Result<DaemonResponse, DaemonError> {
        // Check timeout before connection
        if operation_start.elapsed() >= *total_timeout {
            return Err(DaemonError::TotalOperationTimeout);
        }

        debug!("📡 Connecting to daemon at: {socket_path:?}");

        // Connect
        let mut stream = self
            .connector
            .connect(socket_path)
            .map_err(|e| categorize_io_error(&e))?;

        // Check timeout after connection
        if operation_start.elapsed() >= *total_timeout {
            return Err(DaemonError::TotalOperationTimeout);
        }

        // Set socket timeouts
        stream
            .set_read_timeout(Some(Duration::from_millis(self.config.read_timeout_ms)))
            .map_err(|e| categorize_io_error(&e))?;
        stream
            .set_write_timeout(Some(Duration::from_millis(self.config.write_timeout_ms)))
            .map_err(|e| categorize_io_error(&e))?;

        // Send request
        let request_json = serde_json::to_string(&request)
            .map_err(|e| DaemonError::ProtocolError(format!("Failed to serialize request: {e}")))?;

        debug!("📤 Sending request: {request_json}");
        writeln!(stream, "{request_json}").map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut {
                DaemonError::WriteTimeout
            } else {
                categorize_io_error(&e)
            }
        })?;

        // Check timeout after write
        if operation_start.elapsed() >= *total_timeout {
            return Err(DaemonError::TotalOperationTimeout);
        }

        // Read response
        let mut reader = BufReader::new(&mut *stream);
        let mut response_line = String::new();
        reader.read_line(&mut response_line).map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut {
                DaemonError::ReadTimeout
            } else {
                categorize_io_error(&e)
            }
        })?;

        debug!("📥 Received response: {}", response_line.trim());

        // Validate response size
        if let Err(e) = crate::protocol::validate_message_size(&response_line) {
            return Err(DaemonError::ProtocolError(format!("Response validation failed: {e}")));
        }

        // Parse response
        let response: DaemonResponse = serde_json::from_str(response_line.trim())
            .map_err(|e| DaemonError::ProtocolError(format!("Failed to parse response: {e}")))?;

        // Check for daemon shutdown response
        if let DaemonResponse::ShutdownAck = response {
            return Err(DaemonError::DaemonShutdown);
        }

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::mock::MockSocketConnector;

    // ========================================================================
    // DaemonClientConfig tests
    // ========================================================================

    #[test]
    fn test_daemon_client_config_default() {
        let config = DaemonClientConfig::default();
        assert_eq!(config.connection_timeout_ms, 100);
        assert_eq!(config.read_timeout_ms, 200);
        assert_eq!(config.write_timeout_ms, 50);
        assert_eq!(config.total_timeout_ms, 300);
        assert_eq!(config.retry_delay_ms, 50);
        assert_eq!(config.max_retries, 1);
    }

    #[test]
    fn test_daemon_client_config_clone() {
        let config = DaemonClientConfig::default();
        let cloned = config.clone();
        assert_eq!(config.max_retries, cloned.max_retries);
    }

    #[test]
    fn test_daemon_client_config_debug() {
        let config = DaemonClientConfig::default();
        let debug = format!("{:?}", config);
        assert!(debug.contains("connection_timeout_ms"));
    }

    // ========================================================================
    // DaemonError tests
    // ========================================================================

    #[test]
    fn test_daemon_error_display_all_variants() {
        assert_eq!(DaemonError::ConnectionTimeout.to_string(), "Daemon connection timeout");
        assert_eq!(DaemonError::ReadTimeout.to_string(), "Daemon read timeout");
        assert_eq!(DaemonError::WriteTimeout.to_string(), "Daemon write timeout");
        assert_eq!(DaemonError::ConnectionRefused.to_string(), "Daemon connection refused");
        assert_eq!(DaemonError::SocketNotFound.to_string(), "Daemon socket not found");
        assert_eq!(
            DaemonError::SocketPermissionDenied.to_string(),
            "Daemon socket permission denied"
        );
        assert_eq!(
            DaemonError::ProtocolError("test".to_string()).to_string(),
            "Daemon protocol error: test"
        );
        assert_eq!(DaemonError::DaemonShutdown.to_string(), "Daemon is shutting down");
        assert_eq!(
            DaemonError::TotalOperationTimeout.to_string(),
            "Total daemon operation timeout"
        );
        assert_eq!(
            DaemonError::UnknownError("test".to_string()).to_string(),
            "Unknown daemon error: test"
        );
    }

    #[test]
    fn test_daemon_error_clone() {
        let err = DaemonError::ConnectionTimeout;
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }

    #[test]
    fn test_daemon_error_partial_eq() {
        assert_eq!(DaemonError::ConnectionTimeout, DaemonError::ConnectionTimeout);
        assert_ne!(DaemonError::ConnectionTimeout, DaemonError::ReadTimeout);
    }

    #[test]
    fn test_daemon_error_is_std_error() {
        // Verify DaemonError implements std::error::Error
        fn assert_error<E: std::error::Error>(_: &E) {}
        let err = DaemonError::ConnectionTimeout;
        assert_error(&err);
    }

    // ========================================================================
    // should_retry tests
    // ========================================================================

    #[test]
    fn test_should_retry_connection_timeout() {
        assert!(should_retry(&DaemonError::ConnectionTimeout));
    }

    #[test]
    fn test_should_retry_connection_refused() {
        assert!(should_retry(&DaemonError::ConnectionRefused));
    }

    #[test]
    fn test_should_not_retry_read_timeout() {
        assert!(!should_retry(&DaemonError::ReadTimeout));
    }

    #[test]
    fn test_should_not_retry_write_timeout() {
        assert!(!should_retry(&DaemonError::WriteTimeout));
    }

    #[test]
    fn test_should_not_retry_socket_not_found() {
        assert!(!should_retry(&DaemonError::SocketNotFound));
    }

    #[test]
    fn test_should_not_retry_socket_permission_denied() {
        assert!(!should_retry(&DaemonError::SocketPermissionDenied));
    }

    #[test]
    fn test_should_not_retry_protocol_error() {
        assert!(!should_retry(&DaemonError::ProtocolError("test".to_string())));
    }

    #[test]
    fn test_should_not_retry_daemon_shutdown() {
        assert!(!should_retry(&DaemonError::DaemonShutdown));
    }

    #[test]
    fn test_should_not_retry_total_operation_timeout() {
        assert!(!should_retry(&DaemonError::TotalOperationTimeout));
    }

    #[test]
    fn test_should_not_retry_unknown_error() {
        assert!(!should_retry(&DaemonError::UnknownError("test".to_string())));
    }

    // ========================================================================
    // categorize_io_error tests
    // ========================================================================

    #[test]
    fn test_categorize_io_error_timed_out() {
        let err = std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout");
        assert_eq!(categorize_io_error(&err), DaemonError::ConnectionTimeout);
    }

    #[test]
    fn test_categorize_io_error_connection_refused() {
        let err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        assert_eq!(categorize_io_error(&err), DaemonError::ConnectionRefused);
    }

    #[test]
    fn test_categorize_io_error_not_found() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        assert_eq!(categorize_io_error(&err), DaemonError::SocketNotFound);
    }

    #[test]
    fn test_categorize_io_error_permission_denied() {
        let err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        assert_eq!(categorize_io_error(&err), DaemonError::SocketPermissionDenied);
    }

    #[test]
    fn test_categorize_io_error_would_block() {
        let err = std::io::Error::new(std::io::ErrorKind::WouldBlock, "would block");
        assert_eq!(categorize_io_error(&err), DaemonError::ReadTimeout);
    }

    #[test]
    fn test_categorize_io_error_other() {
        let err = std::io::Error::other("other error");
        match categorize_io_error(&err) {
            DaemonError::UnknownError(msg) => assert!(msg.contains("other error")),
            _ => panic!("Expected UnknownError"),
        }
    }

    // ========================================================================
    // DaemonClient tests with mocks
    // ========================================================================

    #[test]
    fn test_daemon_client_validate_socket_path_not_found() {
        let connector = MockSocketConnector::not_found();
        let client = DaemonClient::with_connector(connector);

        let result = client.validate_socket_path(Path::new("/tmp/test.sock"));
        assert_eq!(result, Err(DaemonError::SocketNotFound));
    }

    #[test]
    fn test_daemon_client_validate_socket_path_success() {
        let connector = MockSocketConnector::new("{}\n");
        let client = DaemonClient::with_connector(connector);

        let result = client.validate_socket_path(Path::new("/tmp/test.sock"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_daemon_client_send_request_socket_not_found() {
        let connector = MockSocketConnector::not_found();
        let client = DaemonClient::with_connector(connector);

        let result = client.send_request(DaemonRequest::Health, Path::new("/tmp/test.sock"));
        assert_eq!(result, Err(DaemonError::SocketNotFound));
    }

    #[test]
    fn test_daemon_client_send_request_connection_refused() {
        let connector = MockSocketConnector::connection_refused();
        let client = DaemonClient::with_connector(connector);

        // Socket exists but connection is refused - this tests the retry logic
        let result = client.send_request(DaemonRequest::Health, Path::new("/tmp/test.sock"));
        assert_eq!(result, Err(DaemonError::ConnectionRefused));
    }

    #[test]
    fn test_daemon_client_send_request_timeout() {
        let connector = MockSocketConnector::timed_out();
        let client = DaemonClient::with_connector(connector);

        let result = client.send_request(DaemonRequest::Health, Path::new("/tmp/test.sock"));
        // Should retry once then fail with ConnectionTimeout
        assert_eq!(result, Err(DaemonError::ConnectionTimeout));
    }

    #[test]
    fn test_daemon_client_send_request_permission_denied() {
        let connector = MockSocketConnector::permission_denied();
        let client = DaemonClient::with_connector(connector);

        let result = client.send_request(DaemonRequest::Health, Path::new("/tmp/test.sock"));
        assert_eq!(result, Err(DaemonError::SocketPermissionDenied));
    }

    #[test]
    fn test_daemon_client_send_request_health_success() {
        // Mock a successful health response
        let response = r#"{"type":"Health","status":"healthy:5:synced"}"#;
        let connector = MockSocketConnector::new(&format!("{}\n", response));
        let client = DaemonClient::with_connector(connector);

        let result = client.send_request(DaemonRequest::Health, Path::new("/tmp/test.sock"));

        match result {
            Ok(DaemonResponse::Health { status }) => {
                assert_eq!(status, "healthy:5:synced");
            }
            other => panic!("Expected Health response, got {:?}", other),
        }
    }

    #[test]
    fn test_daemon_client_send_request_success_response() {
        let response = r#"{"type":"Success","data":"test output"}"#;
        let connector = MockSocketConnector::new(&format!("{}\n", response));
        let client = DaemonClient::with_connector(connector);

        let request = DaemonRequest::Query {
            version: "v0.5.0".to_string(),
            cmdline: "test".to_string(),
            eol: false,
            config: None,
        };

        let result = client.send_request(request, Path::new("/tmp/test.sock"));

        match result {
            Ok(DaemonResponse::Success { data }) => {
                assert_eq!(data, "test output");
            }
            other => panic!("Expected Success response, got {:?}", other),
        }
    }

    #[test]
    fn test_daemon_client_send_request_error_response() {
        let response = r#"{"type":"Error","message":"test error"}"#;
        let connector = MockSocketConnector::new(&format!("{}\n", response));
        let client = DaemonClient::with_connector(connector);

        let result = client.send_request(DaemonRequest::Health, Path::new("/tmp/test.sock"));

        match result {
            Ok(DaemonResponse::Error { message }) => {
                assert_eq!(message, "test error");
            }
            other => panic!("Expected Error response, got {:?}", other),
        }
    }

    #[test]
    fn test_daemon_client_send_request_shutdown_ack() {
        let response = r#"{"type":"ShutdownAck"}"#;
        let connector = MockSocketConnector::new(&format!("{}\n", response));
        let client = DaemonClient::with_connector(connector);

        let result = client.send_request(DaemonRequest::Shutdown, Path::new("/tmp/test.sock"));

        // ShutdownAck should be converted to DaemonShutdown error
        assert_eq!(result, Err(DaemonError::DaemonShutdown));
    }

    #[test]
    fn test_daemon_client_send_request_invalid_json() {
        let connector = MockSocketConnector::new("not valid json\n");
        let client = DaemonClient::with_connector(connector);

        let result = client.send_request(DaemonRequest::Health, Path::new("/tmp/test.sock"));

        match result {
            Err(DaemonError::ProtocolError(msg)) => {
                assert!(msg.contains("parse") || msg.contains("Parse"));
            }
            other => panic!("Expected ProtocolError, got {:?}", other),
        }
    }

    #[test]
    fn test_daemon_client_with_config() {
        let config = DaemonClientConfig {
            connection_timeout_ms: 500,
            read_timeout_ms: 1000,
            write_timeout_ms: 100,
            total_timeout_ms: 2000,
            retry_delay_ms: 100,
            max_retries: 3,
        };

        let response = r#"{"type":"Health","status":"healthy:5:synced"}"#;
        let connector = MockSocketConnector::new(&format!("{}\n", response));
        let client = DaemonClient::with_connector_and_config(connector, config);

        let result = client.send_request(DaemonRequest::Health, Path::new("/tmp/test.sock"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_daemon_client_default() {
        let _client = DaemonClient::default();
        // Just verify it can be created without panicking
    }

    #[test]
    fn test_daemon_client_new() {
        let _client = DaemonClient::new();
        // Just verify it can be created without panicking
    }

    #[test]
    fn test_daemon_client_with_real_config() {
        let config = DaemonClientConfig::default();
        let _client = DaemonClient::with_config(config);
        // Just verify it can be created without panicking
    }

    #[test]
    fn test_daemon_client_version_mismatch_response() {
        let response = r#"{"type":"VersionMismatch","daemon_version":"v1.0.0","client_version":"v0.9.0","message":"Version mismatch detected"}"#;
        let connector = MockSocketConnector::new(&format!("{}\n", response));
        let client = DaemonClient::with_connector(connector);

        let result = client.send_request(DaemonRequest::Health, Path::new("/tmp/test.sock"));

        match result {
            Ok(DaemonResponse::VersionMismatch {
                daemon_version,
                client_version,
                message,
            }) => {
                assert_eq!(daemon_version, "v1.0.0");
                assert_eq!(client_version, "v0.9.0");
                assert!(message.contains("mismatch"));
            }
            other => panic!("Expected VersionMismatch response, got {:?}", other),
        }
    }

    #[test]
    fn test_daemon_client_config_reloaded_response() {
        let response = r#"{"type":"ConfigReloaded","success":true,"message":"Config reloaded successfully"}"#;
        let connector = MockSocketConnector::new(&format!("{}\n", response));
        let client = DaemonClient::with_connector(connector);

        let result = client.send_request(DaemonRequest::ReloadConfig, Path::new("/tmp/test.sock"));

        match result {
            Ok(DaemonResponse::ConfigReloaded { success, message }) => {
                assert!(success);
                assert!(message.contains("successfully"));
            }
            other => panic!("Expected ConfigReloaded response, got {:?}", other),
        }
    }
}
