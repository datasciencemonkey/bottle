use cli_runner::{Version, MIN_VERSION, RECOMMENDED_VERSION};

/// [W-02c] parses the version from `databricks --version` output
#[test]
fn w_02c_parses_cli_version_output() {
    assert_eq!(
        Version::parse_cli_output("Databricks CLI v1.25.0\n"),
        Some(Version::new(1, 25, 0))
    );
    assert_eq!(
        Version::parse_cli_output("Databricks CLI v1.17.3-dev+abc123"),
        Some(Version::new(1, 17, 3))
    );
    assert_eq!(
        Version::parse_cli_output("databricks 2.0"),
        Some(Version::new(2, 0, 0))
    );
}

/// [W-02c] rejects output with no version in it
#[test]
fn w_02c_rejects_output_without_version() {
    assert_eq!(Version::parse_cli_output(""), None);
    assert_eq!(Version::parse_cli_output("Databricks CLI"), None);
    assert_eq!(Version::parse_cli_output("Error: unknown flag v"), None);
    assert_eq!(Version::parse("1"), None);
    assert_eq!(Version::parse("1.x.0"), None);
    assert_eq!(Version::parse("1.2.3.4"), None);
}

/// [W-02c] the minimum is 1.17.0 and 1.25.0 is recommended; ordering is numeric
#[test]
fn w_02c_minimum_and_recommended_versions() {
    assert_eq!(MIN_VERSION, Version::new(1, 17, 0));
    assert_eq!(RECOMMENDED_VERSION, Version::new(1, 25, 0));
    assert!(Version::parse("1.9.0").unwrap() < MIN_VERSION);
    assert!(Version::parse("1.16.99").unwrap() < MIN_VERSION);
    assert!(Version::parse("v1.17").unwrap() >= MIN_VERSION);
    assert!(Version::parse("1.100.0").unwrap() > RECOMMENDED_VERSION);
    assert_eq!(Version::new(1, 25, 0).to_string(), "1.25.0");
}
