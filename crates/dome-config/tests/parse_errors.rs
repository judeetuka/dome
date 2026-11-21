use dome_config::{Blueprint, ConfigError};

#[test]
fn rejects_invalid_toml() {
    let error = Blueprint::from_toml("this is not toml = = =").unwrap_err();
    assert!(matches!(error, ConfigError::Parse(_)), "got {error:?}");
}

#[test]
fn rejects_missing_limits_table() {
    let input = r#"
[cell]
foundation = "dome/buildroot:minimal"
"#;
    let error = Blueprint::from_toml(input).unwrap_err();
    assert!(matches!(error, ConfigError::Parse(_)), "got {error:?}");
    assert!(
        error.to_string().contains("limits"),
        "message should name the missing table: {error}"
    );
}

#[test]
fn rejects_unknown_keys() {
    // A typo must fail loudly instead of being silently ignored.
    let input = r#"
[cell]
foundation = "dome/buildroot:minimal"

[limits]
memroy_mib = 512
storage_mib = 2048
cpus = 1
"#;
    let error = Blueprint::from_toml(input).unwrap_err();
    assert!(matches!(error, ConfigError::Parse(_)), "got {error:?}");
    assert!(
        error.to_string().contains("memroy_mib"),
        "message should name the offending key: {error}"
    );
}

#[test]
fn rejects_zero_memory() {
    let input = r#"
[cell]
foundation = "dome/buildroot:minimal"

[limits]
memory_mib = 0
storage_mib = 2048
cpus = 1
"#;
    match Blueprint::from_toml(input).unwrap_err() {
        ConfigError::InvalidValue { field, .. } => assert_eq!(field, "limits.memory_mib"),
        other => panic!("expected InvalidValue, got {other:?}"),
    }
}

#[test]
fn rejects_empty_foundation() {
    let input = r#"
[cell]
foundation = "   "

[limits]
memory_mib = 512
storage_mib = 2048
cpus = 1
"#;
    match Blueprint::from_toml(input).unwrap_err() {
        ConfigError::InvalidValue { field, .. } => assert_eq!(field, "cell.foundation"),
        other => panic!("expected InvalidValue, got {other:?}"),
    }
}

#[test]
fn rejects_unknown_protocol() {
    let input = r#"
[cell]
foundation = "dome/buildroot:minimal"

[limits]
memory_mib = 512
storage_mib = 2048
cpus = 1

[[perimeter.ports]]
host_addr = "127.0.0.1"
host_port = 8080
guest_port = 80
protocol = "sctp"
"#;
    match Blueprint::from_toml(input).unwrap_err() {
        ConfigError::Unknown { kind, value } => {
            assert_eq!(kind, "protocol");
            assert_eq!(value, "sctp");
        }
        other => panic!("expected Unknown, got {other:?}"),
    }
}

#[test]
fn rejects_unknown_aperture_mode() {
    let input = r#"
[cell]
foundation = "dome/buildroot:minimal"

[limits]
memory_mib = 512
storage_mib = 2048
cpus = 1

[[apertures.files]]
host = "/host/src"
guest = "/work"
mode = "write-only"
"#;
    match Blueprint::from_toml(input).unwrap_err() {
        ConfigError::Unknown { kind, value } => {
            assert_eq!(kind, "file aperture mode");
            assert_eq!(value, "write-only");
        }
        other => panic!("expected Unknown, got {other:?}"),
    }
}

#[test]
fn rejects_invalid_gpio_line() {
    let input = r#"
[cell]
foundation = "dome/buildroot:minimal"

[limits]
memory_mib = 512
storage_mib = 2048
cpus = 1

[[apertures.gpio]]
name = "status-led"
guest_chip = "gpiochip0"
guest_line = 0
host = "gpiochip1:22"
direction = "output"
"#;
    match Blueprint::from_toml(input).unwrap_err() {
        ConfigError::InvalidValue { field, .. } => assert_eq!(field, "apertures.gpio"),
        other => panic!("expected InvalidValue, got {other:?}"),
    }
}

#[test]
fn accepts_a_minimal_blueprint() {
    let input = r#"
[cell]
foundation = "dome/buildroot:minimal"

[limits]
memory_mib = 512
storage_mib = 2048
cpus = 1
"#;
    let blueprint = Blueprint::from_toml(input).expect("minimal blueprint should parse");
    assert_eq!(
        blueprint.cell().foundation().as_str(),
        "dome/buildroot:minimal"
    );
    assert_eq!(blueprint.cell().limits().memory().as_mib(), 512);
    assert!(blueprint.cell().ports().is_empty());
}
