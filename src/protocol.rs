use serde::{Deserialize, Serialize};
use eyre::{eyre, Result};

/// Shared protocol definitions for daemon-client communication
/// This module provides a single source of truth for all IPC message types
/// to ensure consistency between daemon and direct execution modes.

// Basic size limits to prevent technical issues
const MAX_MESSAGE_SIZE: usize = 1_000_000;   // 1MB max total message

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum DaemonRequest {
    /// Query for alias expansion
    Query {
        cmdline: String,
        /// End-of-line processing flag - critical for consistent behavior
        eol: bool,
    },
    /// List aliases with optional filtering
    List {
        global: bool,
        patterns: Vec<String>
    },
    /// Show alias usage frequency statistics
    Freq {
        count: usize,
    },
    /// Health check request
    Health,
    /// Request daemon to reload configuration
    ReloadConfig,
    /// Request daemon shutdown
    Shutdown,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum DaemonResponse {
    /// Successful operation with data
    Success {
        data: String
    },
    /// Error occurred during processing
    Error {
        message: String
    },
    /// Health check response
    Health {
        status: String
    },
    /// Configuration reload response
    ConfigReloaded {
        success: bool,
        message: String
    },
    /// Shutdown acknowledgment
    ShutdownAck,
}

/// Validate the total message size before processing
pub fn validate_message_size(json: &str) -> Result<()> {
    if json.len() > MAX_MESSAGE_SIZE {
        return Err(eyre!("Message too large: {} bytes (max: {})", json.len(), MAX_MESSAGE_SIZE));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_request_serialization() {
        let query = DaemonRequest::Query {
            cmdline: "test command".to_string(),
            eol: true,
        };

        let serialized = serde_json::to_string(&query).expect("Failed to serialize");
        let deserialized: DaemonRequest = serde_json::from_str(&serialized).expect("Failed to deserialize");

        match deserialized {
            DaemonRequest::Query { cmdline, eol } => {
                assert_eq!(cmdline, "test command");
                assert_eq!(eol, true);
            }
            _ => panic!("Wrong variant deserialized"),
        }
    }

    #[test]
    fn test_daemon_response_serialization() {
        let response = DaemonResponse::Success {
            data: "test result".to_string(),
        };

        let serialized = serde_json::to_string(&response).expect("Failed to serialize");
        let deserialized: DaemonResponse = serde_json::from_str(&serialized).expect("Failed to deserialize");

        match deserialized {
            DaemonResponse::Success { data } => {
                assert_eq!(data, "test result");
            }
            _ => panic!("Wrong variant deserialized"),
        }
    }

    #[test]
    fn test_all_request_variants_serialize() {
        let requests = vec![
            DaemonRequest::Query { cmdline: "test".to_string(), eol: false },
            DaemonRequest::List { global: true, patterns: vec!["pattern".to_string()] },
            DaemonRequest::Freq { top: Some(10) },
            DaemonRequest::Health,
            DaemonRequest::ReloadConfig,
            DaemonRequest::Shutdown,
        ];

        for request in requests {
            let serialized = serde_json::to_string(&request).expect("Failed to serialize request");
            let _: DaemonRequest = serde_json::from_str(&serialized).expect("Failed to deserialize request");
        }
    }

    #[test]
    fn test_all_response_variants_serialize() {
        let responses = vec![
            DaemonResponse::Success { data: "data".to_string() },
            DaemonResponse::Error { message: "error".to_string() },
            DaemonResponse::Health { status: "healthy:5:aliases:abc123:synced".to_string() },
            DaemonResponse::ConfigReloaded { success: true, message: "reloaded".to_string() },
            DaemonResponse::ShutdownAck,
        ];

        for response in responses {
            let serialized = serde_json::to_string(&response).expect("Failed to serialize response");
            let _: DaemonResponse = serde_json::from_str(&serialized).expect("Failed to deserialize response");
        }
    }

    #[test]
    fn test_message_size_validation() {
        // Valid message size
        assert!(validate_message_size("small message").is_ok());

        // Too large message should fail
        let large_message = "a".repeat(MAX_MESSAGE_SIZE + 1);
        assert!(validate_message_size(&large_message).is_err());
    }
}
