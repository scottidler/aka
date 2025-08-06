use std::fs;
use tempfile::TempDir;
use aka_lib::{DaemonRequest, DaemonResponse, AKA, ProcessingMode, get_config_path};

/// Test that protocol definitions are consistent across the codebase
#[test]
fn test_protocol_consistency() {
    // Test that we can create all request types
    let requests = vec![
        DaemonRequest::Query { config: None,
            cmdline: "test command".to_string(),
            eol: true,
        },
        DaemonRequest::Query { config: None,
            cmdline: "test command".to_string(),
            eol: false,
        },
        DaemonRequest::List { config: None,
            global: true,
            patterns: vec!["pattern".to_string()]
        },
        DaemonRequest::Freq { config: None,
            all: false,
        },
        DaemonRequest::Freq { config: None,
            all: true,
        },
        DaemonRequest::Health,
        DaemonRequest::ReloadConfig,
        DaemonRequest::Shutdown,
    ];

    // Test that all requests serialize/deserialize correctly
    for request in requests {
        let serialized = serde_json::to_string(&request)
            .expect("Request should serialize");
        let deserialized: DaemonRequest = serde_json::from_str(&serialized)
            .expect("Request should deserialize");

        // Verify the round-trip worked
        match (&request, &deserialized) {
            (DaemonRequest::Query { cmdline: c1, eol: e1, config: _ }, DaemonRequest::Query { cmdline: c2, eol: e2, config: _ }) => {
                assert_eq!(c1, c2);
                assert_eq!(e1, e2);
            },
            (DaemonRequest::List { global: g1, patterns: p1, config: _ }, DaemonRequest::List { global: g2, patterns: p2, config: _ }) => {
                assert_eq!(g1, g2);
                assert_eq!(p1, p2);
            },
            (DaemonRequest::Freq { all: a1, config: _ }, DaemonRequest::Freq { all: a2, config: _ }) => {
                assert_eq!(a1, a2);
            },
            (DaemonRequest::Health, DaemonRequest::Health) => {},
            (DaemonRequest::ReloadConfig, DaemonRequest::ReloadConfig) => {},
            (DaemonRequest::Shutdown, DaemonRequest::Shutdown) => {},
            _ => panic!("Request types don't match after round-trip"),
        }
    }
}

/// Test that responses serialize/deserialize correctly
#[test]
fn test_response_consistency() {
    let responses = vec![
        DaemonResponse::Success { data: "test data".to_string() },
        DaemonResponse::Error { message: "test error".to_string() },
        DaemonResponse::Health { status: "healthy:5:aliases".to_string() },
        DaemonResponse::ConfigReloaded { success: true, message: "reloaded".to_string() },
        DaemonResponse::ConfigReloaded { success: false, message: "failed".to_string() },
        DaemonResponse::ShutdownAck,
    ];

    for response in responses {
        let serialized = serde_json::to_string(&response)
            .expect("Response should serialize");
        let deserialized: DaemonResponse = serde_json::from_str(&serialized)
            .expect("Response should deserialize");

        // Verify the round-trip worked
        match (&response, &deserialized) {
            (DaemonResponse::Success { data: d1 }, DaemonResponse::Success { data: d2 }) => {
                assert_eq!(d1, d2);
            },
            (DaemonResponse::Error { message: m1 }, DaemonResponse::Error { message: m2 }) => {
                assert_eq!(m1, m2);
            },
            (DaemonResponse::Health { status: s1 }, DaemonResponse::Health { status: s2 }) => {
                assert_eq!(s1, s2);
            },
            (DaemonResponse::ConfigReloaded { success: s1, message: m1 }, DaemonResponse::ConfigReloaded { success: s2, message: m2 }) => {
                assert_eq!(s1, s2);
                assert_eq!(m1, m2);
            },
            (DaemonResponse::ShutdownAck, DaemonResponse::ShutdownAck) => {},
            _ => panic!("Response types don't match after round-trip"),
        }
    }
}

/// Test that eol parameter is handled consistently between daemon and direct modes
#[test]
fn test_eol_parameter_consistency() {
    // Create a temporary config for testing
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Create the expected config directory structure
    let config_dir = temp_dir.path().join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config directory");

    let config_path = config_dir.join("aka.yml");

    // Create a simple test config
    let test_config = r#"
aliases:
  test_alias:
    value: "echo hello"
    global: false
"#;
    fs::write(&config_path, test_config).expect("Failed to write test config");

    // Test both eol values in direct mode
    let test_cases = vec![
        ("test_alias", true),
        ("test_alias", false),
    ];

    for (cmdline, eol) in test_cases {
        // Test direct mode
        let home_dir = temp_dir.path().to_path_buf();
        let config_path = get_config_path(&home_dir).expect("Failed to get config path");
        let mut aka_direct = AKA::new(eol, home_dir.clone(), config_path.clone()).expect("Failed to create AKA for direct mode");
        let direct_result = aka_direct.replace_with_mode(cmdline, ProcessingMode::Direct)
            .expect("Direct mode should work");

        // Test daemon mode (simulated - we can't easily test the actual daemon here)
        let mut aka_daemon = AKA::new(eol, home_dir, config_path).expect("Failed to create AKA for daemon mode");
        let daemon_result = aka_daemon.replace_with_mode(cmdline, ProcessingMode::Daemon)
            .expect("Daemon mode should work");

        // Results should be identical
        assert_eq!(direct_result, daemon_result,
            "Direct and daemon modes should produce identical results for eol={}", eol);
    }
}

/// Test that the Query request includes eol parameter
#[test]
fn test_query_request_has_eol() {
    // Create a query request
    let query = DaemonRequest::Query { config: None,
        cmdline: "test command".to_string(),
        eol: true,
    };

    // Serialize it
    let serialized = serde_json::to_string(&query).expect("Should serialize");

    // Check that the serialized JSON contains the eol field
    assert!(serialized.contains("eol"), "Serialized query should contain eol field");
    assert!(serialized.contains("true"), "Serialized query should contain eol value");

    // Test with eol=false
    let query_false = DaemonRequest::Query { config: None,
        cmdline: "test command".to_string(),
        eol: false,
    };

    let serialized_false = serde_json::to_string(&query_false).expect("Should serialize");
    assert!(serialized_false.contains("eol"), "Serialized query should contain eol field");
    assert!(serialized_false.contains("false"), "Serialized query should contain eol value");
}

/// Test that protocol messages are tagged correctly for serde
#[test]
fn test_protocol_message_tags() {
    // Test that request messages have correct type tags
    let health = DaemonRequest::Health;
    let serialized = serde_json::to_string(&health).expect("Should serialize");
    assert!(serialized.contains("\"type\":\"Health\""), "Health request should have correct type tag");

    let query = DaemonRequest::Query { cmdline: "test".to_string(), eol: false, config: None };
    let serialized = serde_json::to_string(&query).expect("Should serialize");
    assert!(serialized.contains("\"type\":\"Query\""), "Query request should have correct type tag");

    // Test that response messages have correct type tags
    let success = DaemonResponse::Success { data: "test".to_string() };
    let serialized = serde_json::to_string(&success).expect("Should serialize");
    assert!(serialized.contains("\"type\":\"Success\""), "Success response should have correct type tag");

    let error = DaemonResponse::Error { message: "test".to_string() };
    let serialized = serde_json::to_string(&error).expect("Should serialize");
    assert!(serialized.contains("\"type\":\"Error\""), "Error response should have correct type tag");
}

/// Test that we can differentiate between different request types
#[test]
fn test_request_type_differentiation() {
    let requests_json = vec![
        r#"{"type":"Health"}"#,
        r#"{"type":"Query","cmdline":"test","eol":true}"#,
        r#"{"type":"List","global":false,"patterns":[]}"#,
        r#"{"type":"Freq","all":false}"#,
        r#"{"type":"ReloadConfig"}"#,
        r#"{"type":"Shutdown"}"#,
    ];

    for json in requests_json {
        let request: DaemonRequest = serde_json::from_str(json)
            .expect(&format!("Should deserialize: {}", json));

        // Verify we can match on the deserialized type
        match request {
            DaemonRequest::Health => {},
            DaemonRequest::Query { cmdline, eol, config: _ } => {
                assert_eq!(cmdline, "test");
                assert_eq!(eol, true);
            },
            DaemonRequest::List { global, patterns, config: _ } => {
                assert_eq!(global, false);
                assert_eq!(patterns.len(), 0);
            },
            DaemonRequest::Freq { all: _, config: _ } => {},
            DaemonRequest::ReloadConfig => {},
            DaemonRequest::Shutdown => {},
            DaemonRequest::CompleteAliases { config: _ } => {},
        }
    }
}
