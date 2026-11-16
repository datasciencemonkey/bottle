//! Tests for the Databricks CLI `SHA256SUMS` parser.

use artifact_verify::sums::sums_file_name;
use artifact_verify::{Sha256Sums, SumsError};

const A: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const B: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// [W-17d] the checksum file name follows databricks_cli_<ver>_SHA256SUMS
#[test]
fn checksum_file_name() {
    assert_eq!(
        sums_file_name("0.250.0"),
        "databricks_cli_0.250.0_SHA256SUMS"
    );
    assert_eq!(
        sums_file_name("v0.250.0"),
        "databricks_cli_0.250.0_SHA256SUMS"
    );
}

/// [W-17d] [SEC-20b] parses "<hex>  <filename>" lines and looks up the expected hash
#[test]
fn parses_release_sums() {
    let text = format!(
        "{A}  databricks_cli_0.250.0_darwin_arm64.zip\n\
         {B}  databricks_cli_0.250.0_linux_amd64.tar.gz\r\n\
         \n\
         {}  *databricks_cli_0.250.0_windows_amd64.zip\n",
        A.to_uppercase()
    );
    let sums = Sha256Sums::parse(&text).unwrap();
    assert_eq!(sums.len(), 3);
    assert_eq!(
        sums.get("databricks_cli_0.250.0_darwin_arm64.zip")
            .unwrap()
            .to_hex(),
        A
    );
    assert_eq!(
        sums.expected_for("databricks_cli_0.250.0_linux_amd64.tar.gz")
            .unwrap()
            .to_hex(),
        B
    );
    assert_eq!(
        sums.get("databricks_cli_0.250.0_windows_amd64.zip")
            .unwrap()
            .to_hex(),
        A
    );
    assert!(sums.get("databricks_cli_0.250.0_darwin").is_none());
    assert_eq!(
        sums.expected_for("nope.zip"),
        Err(SumsError::NotListed {
            file_name: "nope.zip".into()
        })
    );
}

/// [W-17d] [SEC-20b] malformed or ambiguous SHA256SUMS files are rejected
#[test]
fn rejects_malformed_sums() {
    assert!(matches!(
        Sha256Sums::parse("garbage\n"),
        Err(SumsError::BadLine { line: 1 })
    ));
    assert!(matches!(
        Sha256Sums::parse(&format!("{A}  ok.zip\n{A}\n")),
        Err(SumsError::BadLine { line: 2 })
    ));
    assert!(matches!(
        Sha256Sums::parse("abcd  x.zip\n"),
        Err(SumsError::BadDigest { line: 1, .. })
    ));
    assert!(matches!(
        Sha256Sums::parse(&format!("{A}  x.zip\n{B}  x.zip\n")),
        Err(SumsError::Duplicate { .. })
    ));
    assert!(Sha256Sums::parse("").unwrap().is_empty());
}
