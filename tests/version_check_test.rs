use aka_lib::{DaemonRequest, DaemonResponse};
use serde_json;

#[test]
fn test_version_field_in_query_request() {
    let request = DaemonRequest::Query {
        version: "v0.5.0-test".to_string(),
        cmdline: "test command".to_string(),
        eol: true,
        config: None,
    };

    // Serialize and verify version is included
    let json = serde_json::to_string(&request).expect("Failed to serialize");
    assert!(
        json.contains("\"version\":\"v0.5.0-test\""),
        "Version field should be in JSON: {}",
        json
    );

    // Verify deserialization works
    let deserialized: DaemonRequest = serde_json::from_str(&json).expect("Failed to deserialize");

    match deserialized {
        DaemonRequest::Query {
            version,
            cmdline,
            eol,
            config,
        } => {
            assert_eq!(version, "v0.5.0-test");
            assert_eq!(cmdline, "test command");
            assert_eq!(eol, true);
            assert_eq!(config, None);
        }
        _ => panic!("Wrong variant deserialized"),
    }
}

#[test]
fn test_version_field_in_list_request() {
    let request = DaemonRequest::List {
        version: "v0.5.1".to_string(),
        global: true,
        patterns: vec!["test".to_string()],
        config: None,
    };

    let json = serde_json::to_string(&request).expect("Failed to serialize");
    assert!(json.contains("\"version\":\"v0.5.1\""));

    let deserialized: DaemonRequest = serde_json::from_str(&json).expect("Failed to deserialize");
    match deserialized {
        DaemonRequest::List { version, .. } => {
            assert_eq!(version, "v0.5.1");
        }
        _ => panic!("Wrong variant deserialized"),
    }
}

#[test]
fn test_version_field_in_freq_request() {
    let request = DaemonRequest::Freq {
        version: "v0.5.2".to_string(),
        all: false,
        config: None,
    };

    let json = serde_json::to_string(&request).expect("Failed to serialize");
    assert!(json.contains("\"version\":\"v0.5.2\""));

    let deserialized: DaemonRequest = serde_json::from_str(&json).expect("Failed to deserialize");
    match deserialized {
        DaemonRequest::Freq { version, .. } => {
            assert_eq!(version, "v0.5.2");
        }
        _ => panic!("Wrong variant deserialized"),
    }
}

#[test]
fn test_version_field_in_complete_aliases_request() {
    let request = DaemonRequest::CompleteAliases {
        version: "v0.5.3".to_string(),
        config: None,
    };

    let json = serde_json::to_string(&request).expect("Failed to serialize");
    assert!(json.contains("\"version\":\"v0.5.3\""));

    let deserialized: DaemonRequest = serde_json::from_str(&json).expect("Failed to deserialize");
    match deserialized {
        DaemonRequest::CompleteAliases { version, .. } => {
            assert_eq!(version, "v0.5.3");
        }
        _ => panic!("Wrong variant deserialized"),
    }
}

#[test]
fn test_version_mismatch_response_serialization() {
    let response = DaemonResponse::VersionMismatch {
        daemon_version: "v0.5.0".to_string(),
        client_version: "v0.5.1".to_string(),
        message: "Daemon restarting to match client version".to_string(),
    };

    let json = serde_json::to_string(&response).expect("Failed to serialize");
    assert!(json.contains("VersionMismatch"), "Should contain variant name");
    assert!(json.contains("v0.5.0"), "Should contain daemon version");
    assert!(json.contains("v0.5.1"), "Should contain client version");
    assert!(json.contains("Daemon restarting"), "Should contain message");
}

#[test]
fn test_version_mismatch_response_deserialization() {
    let json = r#"{
        "type": "VersionMismatch",
        "daemon_version": "v0.5.0-1-g1234567",
        "client_version": "v0.5.1-2-gabcdefg",
        "message": "Version mismatch detected"
    }"#;

    let response: DaemonResponse = serde_json::from_str(json).expect("Failed to deserialize VersionMismatch response");

    match response {
        DaemonResponse::VersionMismatch {
            daemon_version,
            client_version,
            message,
        } => {
            assert_eq!(daemon_version, "v0.5.0-1-g1234567");
            assert_eq!(client_version, "v0.5.1-2-gabcdefg");
            assert_eq!(message, "Version mismatch detected");
        }
        _ => panic!("Wrong variant deserialized: {:?}", response),
    }
}

#[test]
fn test_version_mismatch_response_round_trip() {
    let original = DaemonResponse::VersionMismatch {
        daemon_version: "v0.5.0-test".to_string(),
        client_version: "v0.5.1-test".to_string(),
        message: "Test message".to_string(),
    };

    let json = serde_json::to_string(&original).expect("Failed to serialize");
    let deserialized: DaemonResponse = serde_json::from_str(&json).expect("Failed to deserialize");

    match (original, deserialized) {
        (
            DaemonResponse::VersionMismatch {
                daemon_version: dv1,
                client_version: cv1,
                message: m1,
            },
            DaemonResponse::VersionMismatch {
                daemon_version: dv2,
                client_version: cv2,
                message: m2,
            },
        ) => {
            assert_eq!(dv1, dv2);
            assert_eq!(cv1, cv2);
            assert_eq!(m1, m2);
        }
        _ => panic!("Round trip failed"),
    }
}

#[test]
fn test_admin_requests_no_version() {
    // Health, ReloadConfig, and Shutdown don't need version fields
    let health = DaemonRequest::Health;
    let reload = DaemonRequest::ReloadConfig;
    let shutdown = DaemonRequest::Shutdown;

    // These should all serialize successfully
    assert!(serde_json::to_string(&health).is_ok());
    assert!(serde_json::to_string(&reload).is_ok());
    assert!(serde_json::to_string(&shutdown).is_ok());

    // Verify they don't contain version field
    let health_json = serde_json::to_string(&health).unwrap();
    assert!(
        !health_json.contains("version"),
        "Admin commands should not have version field"
    );
}

#[test]
fn test_git_describe_version_format() {
    // Test that various git-describe format versions work
    let test_versions = vec![
        "v0.5.0",
        "v0.5.0-1-g1a2b3c4",
        "v0.5.1-10-gabcdefg",
        "v1.0.0-rc1",
        "v1.0.0-rc1-5-g9876543",
    ];

    for version in test_versions {
        let request = DaemonRequest::Query {
            version: version.to_string(),
            cmdline: "test".to_string(),
            eol: false,
            config: None,
        };

        let json = serde_json::to_string(&request).expect(&format!("Failed to serialize version: {}", version));
        let deserialized: DaemonRequest =
            serde_json::from_str(&json).expect(&format!("Failed to deserialize version: {}", version));

        match deserialized {
            DaemonRequest::Query { version: v, .. } => {
                assert_eq!(v, version);
            }
            _ => panic!("Wrong variant for version: {}", version),
        }
    }
}
