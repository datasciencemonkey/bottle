//! T7 ProfileService, host side: `profiles_list`, the auth-type filter and the token storage
//! check (#17, ADR 0094).
//!
//! Requirement IDs: W-01a, W-03a, W-03b, W-08, W-10a, W-10b.
//!
//! Every config text is fake and is built in the test. No test reads the real
//! `~/.databrickscfg` and no test runs a real Databricks CLI: R5 is a fake that answers
//! `auth describe` from a table. Every fake secret is built at run time from pieces, so secret
//! scanners do not flag the repo.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mog_core::cli_runner::{refuse_token_command, CliRunner, StubCliRunner};
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::ProfileHosts;
use mog_core::ipc_gateway::{hardened_gateway_with, GatewayComponents, IpcGateway, NoLog};
use mog_core::profile_service::{
    config_path, describe_args, parse_config, ConfigFile, ConfigProfile, ConfigSource,
    ProfileService, U2M_AUTH_TYPE,
};
use mog_core::types::Profile;
use serde_json::{json, Value};

// ---------- fakes ----------

/// A fake secret value, built at run time so no file holds a credential-shaped string.
fn fake_secret(tag: &str) -> String {
    format!("{}-{}-{}", "test", tag, "not-real")
}

const HOST_A: &str = "https://example-a.cloud.databricks.com";
const HOST_B: &str = "https://example-b.cloud.databricks.com";
const HOST_C: &str = "https://example-c.cloud.databricks.com";

/// A fake CLI config with one U2M, one PAT, one service-principal and one profile with no
/// auth type. Values that are secrets are fake and built at run time.
fn fake_config() -> String {
    format!(
        "; a comment\n\
         [__settings__]\n\
         default_profile = fake-u2m\n\
         \n\
         [fake-u2m]\n\
         host      = {HOST_A}/\n\
         auth_type = databricks-cli\n\
         \n\
         [fake-pat]\n\
         host  = {HOST_B}\n\
         token = {pat}\n\
         \n\
         [fake-sp]\n\
         host          = {HOST_C}\n\
         auth_type     = oauth-m2m\n\
         client_id     = fake-client\n\
         client_secret = {sp}\n\
         \n\
         [fake-no-host]\n\
         auth_type = databricks-cli\n",
        pat = fake_secret("pat"),
        sp = fake_secret("sp"),
    )
}

/// A config source over fake text, in memory.
struct FakeConfig(Mutex<Option<String>>);

impl FakeConfig {
    fn new(text: Option<String>) -> Arc<Self> {
        Arc::new(Self(Mutex::new(text)))
    }
}

impl ConfigSource for FakeConfig {
    fn profiles(&self) -> CoreResult<Vec<ConfigProfile>> {
        Ok(match self.0.lock().unwrap().as_deref() {
            Some(text) => parse_config(text.as_bytes()),
            None => Vec::new(),
        })
    }
}

/// What the fake R5 answers to `auth describe` for one profile.
#[derive(Clone)]
enum Describe {
    Mode(&'static str),
    NoMode,
    Fails,
}

/// A fake R5: `auth describe` answers from a table; every call is recorded.
struct FakeCli {
    describe: Mutex<BTreeMap<String, Describe>>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl FakeCli {
    fn new(table: &[(&str, Describe)]) -> Arc<Self> {
        Arc::new(Self {
            describe: Mutex::new(
                table
                    .iter()
                    .map(|(n, d)| ((*n).to_owned(), d.clone()))
                    .collect(),
            ),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn set(&self, profile: &str, d: Describe) {
        self.describe.lock().unwrap().insert(profile.to_owned(), d);
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

impl CliRunner for FakeCli {
    fn resolve(&self) -> CoreResult<PathBuf> {
        Ok(PathBuf::from("/fake/databricks"))
    }

    fn run_json(&self, args: &[&str]) -> CoreResult<Value> {
        refuse_token_command(args)?;
        self.calls
            .lock()
            .unwrap()
            .push(args.iter().map(|a| (*a).to_owned()).collect());
        let profile = args
            .iter()
            .position(|a| *a == "--profile")
            .and_then(|i| args.get(i + 1))
            .copied()
            .unwrap_or_default();
        match self.describe.lock().unwrap().get(profile).cloned() {
            Some(Describe::Mode(mode)) => Ok(json!({
                "status": "success",
                "details": {
                    "host": HOST_A,
                    "profile": profile,
                    "auth_type": U2M_AUTH_TYPE,
                    "token_storage": { "mode": mode, "location": "fake", "source": "default" }
                }
            })),
            Some(Describe::NoMode) => Ok(json!({ "status": "success", "details": {} })),
            Some(Describe::Fails) | None => Err(CoreError::Unavailable {
                component: "R5",
                reason: "cli_failed",
            }),
        }
    }
}

fn service(text: Option<String>, cli: Arc<dyn CliRunner>) -> ProfileService {
    ProfileService::new(FakeConfig::new(text), cli)
}

fn secure_cli() -> Arc<FakeCli> {
    FakeCli::new(&[("fake-u2m", Describe::Mode("secure"))])
}

fn by_name<'a>(list: &'a [Profile], name: &str) -> &'a Profile {
    list.iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("{name} is listed"))
}

// ---------- the parser: names, host and auth_type only ----------

#[test]
fn w_03a_parse_keeps_only_name_host_and_auth_type() {
    let parsed = parse_config(fake_config().as_bytes());
    let names: Vec<&str> = parsed.iter().map(|p| p.name.as_str()).collect();
    // `[__settings__]` is the CLI's own section; a section without a host is not a profile.
    assert_eq!(names, ["fake-u2m", "fake-pat", "fake-sp"]);
    let u2m = &parsed[0];
    assert_eq!(
        u2m.host, HOST_A,
        "the host is the bare https://host, with no path"
    );
    assert_eq!(u2m.auth_type, U2M_AUTH_TYPE);
    assert_eq!(
        parsed[1].auth_type, "",
        "a PAT profile has no auth_type key"
    );
    assert_eq!(parsed[2].auth_type, "oauth-m2m");
}

#[test]
fn w_01a_parse_never_keeps_a_secret_value() {
    let parsed = parse_config(fake_config().as_bytes());
    let dump = format!("{parsed:?}");
    for secret in [
        fake_secret("pat"),
        fake_secret("sp"),
        "fake-client".to_owned(),
    ] {
        assert!(
            !dump.contains(&secret),
            "the parse kept a value it does not need"
        );
    }
}

#[test]
fn w_03a_parse_follows_the_cli_ini_rules() {
    let text = "\u{feff}host = https://example-default.cloud.databricks.com\n\
                auth_type: databricks-cli\n\
                # a comment\n\
                [ spaced ]\r\n\
                HOST = example-s.cloud.databricks.com/some/path?o=1\r\n\
                Auth_Type = \"databricks-cli\"\r\n\
                [dup]\n\
                host = https://example-d.cloud.databricks.com\n\
                [spaced]\n\
                auth_type = pat\n";
    let parsed = parse_config(text.as_bytes());
    // Keys before the first header belong to DEFAULT, as in the CLI's INI parser.
    assert_eq!(parsed[0].name, "DEFAULT");
    assert_eq!(
        parsed[0].host,
        "https://example-default.cloud.databricks.com"
    );
    assert_eq!(parsed[0].auth_type, U2M_AUTH_TYPE);
    // A repeated section merges into the first one; the later key wins.
    let spaced = parsed.iter().find(|p| p.name == "spaced").unwrap();
    assert_eq!(spaced.host, "https://example-s.cloud.databricks.com");
    assert_eq!(spaced.auth_type, "pat");
    assert_eq!(parsed.len(), 3);
}

#[test]
fn w_03a_parse_of_no_text_or_bad_bytes_gives_no_profiles() {
    assert!(parse_config(b"").is_empty());
    assert!(parse_config(b"\xff\xfe[not]\nhost").is_empty());
}

// ---------- W-03a, W-03b: only databricks-cli profiles are supported ----------

#[test]
fn w_03a_only_databricks_cli_profiles_are_supported() {
    let s = service(Some(fake_config()), secure_cli());
    let list = s.list(false).unwrap();
    assert_eq!(
        list.len(),
        3,
        "unsupported profiles are listed too, as not supported"
    );
    assert!(by_name(&list, "fake-u2m").supported);
    assert_eq!(by_name(&list, "fake-u2m").auth_type, U2M_AUTH_TYPE);
    assert!(!by_name(&list, "fake-pat").supported);
    assert!(!by_name(&list, "fake-sp").supported);
    assert_eq!(by_name(&list, "fake-sp").auth_type, "oauth-m2m");
}

#[test]
fn w_03b_a_pat_or_service_principal_profile_has_no_workspace_url() {
    let s = service(Some(fake_config()), secure_cli());
    assert_eq!(s.workspace_url("fake-u2m").unwrap(), HOST_A);
    for name in ["fake-pat", "fake-sp", "fake-unknown"] {
        let e = s.workspace_url(name).unwrap_err();
        assert_eq!(e.code(), "profile_not_supported", "{name}: {e:?}");
    }
}

#[test]
fn w_03b_storage_is_checked_only_for_databricks_cli_profiles() {
    let cli = secure_cli();
    let s = service(Some(fake_config()), cli.clone());
    s.list(false).unwrap();
    let calls = cli.calls();
    assert_eq!(
        calls,
        vec![describe_args("fake-u2m").map(str::to_owned).to_vec()]
    );
}

// ---------- W-10a, W-10b: plaintext token storage blocks the profile ----------

#[test]
fn w_10a_plaintext_token_storage_blocks_the_profile() {
    let cli = FakeCli::new(&[("fake-u2m", Describe::Mode("plaintext"))]);
    let s = service(Some(fake_config()), cli);
    let list = s.list(false).unwrap();
    assert!(!by_name(&list, "fake-u2m").supported);
    assert_eq!(by_name(&list, "fake-u2m").auth_type, U2M_AUTH_TYPE);
    assert_eq!(
        s.workspace_url("fake-u2m").unwrap_err().code(),
        "profile_not_supported"
    );
}

#[test]
fn w_10a_a_describe_without_a_mode_or_a_failed_describe_blocks_the_profile() {
    for d in [Describe::NoMode, Describe::Fails, Describe::Mode("other")] {
        let s = service(Some(fake_config()), FakeCli::new(&[("fake-u2m", d)]));
        assert!(!by_name(&s.list(false).unwrap(), "fake-u2m").supported);
    }
}

#[test]
fn w_10a_with_r5_not_built_the_storage_check_does_not_block() {
    // TODO(#16): R5 is the stub on main. No token can flow through it either.
    let s = service(Some(fake_config()), Arc::new(StubCliRunner));
    let list = s.list(false).unwrap();
    assert!(by_name(&list, "fake-u2m").supported);
    assert!(!by_name(&list, "fake-pat").supported);
}

#[test]
fn w_10b_the_profile_stays_unusable_until_the_storage_changes() {
    let cli = FakeCli::new(&[("fake-u2m", Describe::Mode("plaintext"))]);
    let s = service(Some(fake_config()), cli.clone());
    assert!(!by_name(&s.list(false).unwrap(), "fake-u2m").supported);
    cli.set("fake-u2m", Describe::Mode("secure"));
    // The cached list does not change by itself.
    assert!(!by_name(&s.list(false).unwrap(), "fake-u2m").supported);
    // A refresh checks the storage again.
    assert!(by_name(&s.list(true).unwrap(), "fake-u2m").supported);
}

// ---------- never picks a profile ----------

#[test]
fn w_03a_one_profile_is_listed_and_never_picked() {
    let text = format!("[fake-only]\nhost = {HOST_A}\nauth_type = databricks-cli\n");
    let cli = FakeCli::new(&[("fake-only", Describe::Mode("secure"))]);
    let s = service(Some(text), cli);
    let reply = s.call("profiles_list", json!({})).unwrap();
    assert_eq!(
        reply,
        json!([{ "name": "fake-only", "host": HOST_A, "authType": U2M_AUTH_TYPE, "supported": true }])
    );
}

#[test]
fn w_03a_no_config_file_lists_no_profiles() {
    let s = service(None, secure_cli());
    assert_eq!(s.list(false).unwrap(), Vec::<Profile>::new());
}

// ---------- the profiles_list reply through the hardened gateway ----------

#[test]
fn w_03a_profiles_list_through_the_gateway_from_brain_and_hub() {
    let s = service(Some(fake_config()), secure_cli());
    let gw = hardened_gateway_with(NoLog, s);
    for label in ["brain", "hub"] {
        let reply = gw
            .dispatch(label, "profiles_list", json!({ "refresh": true }))
            .unwrap();
        let list: Vec<Profile> = serde_json::from_value(reply).unwrap();
        assert_eq!(list.len(), 3);
        assert!(by_name(&list, "fake-u2m").supported);
    }
    let e = gw
        .dispatch("mog-a1", "profiles_list", json!({}))
        .unwrap_err();
    assert_eq!(e.code(), "window_not_allowed");
}

#[test]
fn w_01a_profiles_list_reply_holds_no_secret_value() {
    let s = service(Some(fake_config()), secure_cli());
    let reply = s.call("profiles_list", json!({})).unwrap().to_string();
    for secret in [
        fake_secret("pat"),
        fake_secret("sp"),
        "fake-client".to_owned(),
    ] {
        assert!(!reply.contains(&secret));
    }
}

#[test]
fn w_03a_other_commands_are_not_this_component() {
    let s = service(Some(fake_config()), secure_cli());
    let e = s
        .call("profile_status", json!({ "profile": "fake-u2m" }))
        .unwrap_err();
    assert_eq!(e.code(), "not_implemented");
}

// ---------- the config file: path, read, never written ----------

fn temp_file(tag: &str, text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "bottle-t7-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fake-databrickscfg");
    std::fs::write(&path, text).unwrap();
    path
}

fn remove(path: &Path) {
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn w_08_listing_leaves_the_config_file_unchanged() {
    let text = fake_config();
    let path = temp_file("w08", &text);
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    let s = ProfileService::new(Arc::new(ConfigFile::new(Some(path.clone()))), secure_cli());
    assert_eq!(s.list(true).unwrap().len(), 3);
    s.workspace_url("fake-u2m").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before
    );
    remove(&path);
}

#[test]
fn w_03a_config_file_missing_gives_no_profiles() {
    let path = temp_file("missing", "");
    let gone = path.with_file_name("not-there");
    assert!(ConfigFile::new(Some(gone)).profiles().unwrap().is_empty());
    assert!(ConfigFile::new(None).profiles().unwrap().is_empty());
    remove(&path);
}

#[test]
fn w_01a_config_path_is_the_cli_home_file() {
    let env = |k: &str| -> Option<OsString> {
        match k {
            "HOME" => Some("/fake/home".into()),
            "USERPROFILE" => Some("C:\\fake\\home".into()),
            // Bottle reads only the file the CLI reads with a clean env.
            "DATABRICKS_CONFIG_FILE" => Some("/fake/elsewhere".into()),
            _ => None,
        }
    };
    let home = if cfg!(windows) {
        "C:\\fake\\home"
    } else {
        "/fake/home"
    };
    assert_eq!(
        config_path(env),
        Some(Path::new(home).join(".databrickscfg"))
    );
    assert_eq!(config_path(|_| None), None);
}

// ---------- W-01a, W-08: source scans ----------

fn src(rel: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Every `.rs` file under `dir`, with its text.
fn rust_files(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((path.clone(), std::fs::read_to_string(&path).unwrap()));
        }
    }
}

/// The non-test part of a source file (before its first `#[cfg(test)]`).
fn non_test(text: &str) -> &str {
    text.split("#[cfg(test)]").next().unwrap_or(text)
}

#[test]
fn w_01a_no_other_credential_source_in_the_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    rust_files(&root.join("../../src-tauri/src"), &mut files);
    assert!(files.len() > 20, "the scan found the sources");
    // Credential sources other than the CLI: the CLI token cache, token or secret environment
    // variables, and the config file outside T7.
    let banned = [
        "token-cache.json",
        "DATABRICKS_TOKEN",
        "DATABRICKS_CLIENT_SECRET",
        "DATABRICKS_PASSWORD",
        "DATABRICKS_CONFIG_FILE",
    ];
    for (path, text) in &files {
        let code = non_test(text);
        for b in banned {
            assert!(
                !code.contains(&format!("\"{b}")),
                "{}: reads the credential source {b}",
                path.display()
            );
        }
        let is_t7 = path.ends_with("profile_service.rs");
        if !is_t7 {
            assert!(
                !code.contains("\".databrickscfg\""),
                "{}: only T7 opens the CLI config file",
                path.display()
            );
        }
    }
}

#[test]
fn w_08_profile_service_never_writes_a_file() {
    let code = src("src/profile_service.rs");
    let code = non_test(&code);
    for w in [
        "fs::write",
        "File::create",
        "OpenOptions",
        "set_permissions",
        "fs::rename",
        "remove_file",
        ".write(",
        "write_all",
    ] {
        assert!(!code.contains(w), "T7 must not write: found {w}");
    }
    // It reads only the keys it needs.
    assert!(code.contains("\"host\"") && code.contains("\"auth_type\""));
    for key in ["\"token\"", "\"client_secret\"", "\"password\""] {
        assert!(!code.contains(key), "T7 must not look up {key}");
    }
}

#[test]
fn w_03b_a_profile_added_after_the_first_list_is_read_again_once() {
    let config = FakeConfig::new(Some(String::new()));
    let cli = FakeCli::new(&[("fake-new", Describe::Mode("secure"))]);
    let s = ProfileService::new(config.clone(), cli.clone());
    assert!(s.list(false).unwrap().is_empty());
    *config.0.lock().unwrap() = Some(format!(
        "[fake-new]\nhost = {HOST_A}\nauth_type = databricks-cli\n"
    ));
    // An unknown name reads the file again once; a known name uses the last list.
    assert_eq!(s.workspace_url("fake-new").unwrap(), HOST_A);
    assert_eq!(s.workspace_url("fake-new").unwrap(), HOST_A);
    assert_eq!(
        cli.calls().len(),
        1,
        "one storage check for the new profile"
    );
}
