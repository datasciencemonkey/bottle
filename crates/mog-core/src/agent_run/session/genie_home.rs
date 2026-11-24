//! GENIE_HOME: the disposable home of the one Genie Code server (plan Amendment 2, A3.1).
//!
//! Rust deletes it and writes it again before each start, and deletes it again at each end and
//! on Wipe. It holds `config.toml` (0600) and `answer.schema.json` (0600) in a 0700 folder. It
//! never holds a token, a secret, a question or an answer: the shim secret reaches the server
//! only through the process environment, and `config.toml` names the variables.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::appserver::SpawnSpec;
use crate::agent_run::pin::GENIE_PINNED;

/// The output schema of a turn: the spike's `schema.json`.
pub const ANSWER_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"required":["answer","sources"],"properties":{"answer":{"type":"string"},"sources":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["workspace","callId"],"properties":{"workspace":{"type":"string"},"callId":{"type":"string"}}}}}}"#;

/// The model alias of Genie Code in a workspace.
pub const MODEL: &str = "genie-code-cli-primary";
/// The path of the model gateway on the workspace host.
pub const GATEWAY_PATH: &str = "/ai-gateway/clients/genie-cli/openai/v1";
/// `tool_timeout_sec` of the bottle MCP server (owner default).
pub const TOOL_TIMEOUT_SEC: u32 = 600;
/// The prefix of the temp cwd folders.
pub const CWD_PREFIX: &str = "bottle-genie-cwd-";

/// The feature flags that must stay off (the 23 flags of the tested `home-e`).
pub const FEATURES_OFF: [&str; 23] = [
    "shell_tool",
    "unified_exec_tty",
    "shell_snapshot",
    "view_image",
    "multi_agent",
    "apps",
    "plugins",
    "remote_plugin",
    "browser_use",
    "browser_use_external",
    "in_app_browser",
    "computer_use",
    "image_generation",
    "goals",
    "hooks",
    "tool_suggest",
    "skill_search",
    "skill_mcp_dependency_install",
    "daemon_auto_start",
    "multi_agent_v2",
    "collaboration_modes",
    "request_permissions_tool",
    "default_mode_request_user_input",
];

/// What `config.toml` needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeSpec {
    /// The workspace host of the agent profile: a bare host or `https://<host>`.
    pub host: String,
    /// The absolute path of the R5-vetted Databricks CLI.
    pub cli_path: PathBuf,
    /// The agent profile (an argument of the auth command, written through the escaper).
    pub profile: String,
    /// The shim command, its arguments and the names (not the values) of its env vars.
    pub shim_command: PathBuf,
    pub shim_args: Vec<String>,
    pub shim_env_names: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HomeError {
    /// The host is not a bare DNS name over https.
    BadHost,
    /// A path or an argument is not valid text for the file.
    BadValue,
    Io,
}

impl From<io::Error> for HomeError {
    fn from(_: io::Error) -> Self {
        HomeError::Io
    }
}

/// A bare DNS host: ASCII labels, no number as last label. No port, path, query or user.
fn bare_host(host: &str) -> Option<String> {
    let t = host.trim();
    let bare = match t.get(..8) {
        Some(s) if s.eq_ignore_ascii_case("https://") => &t[8..],
        _ => t,
    };
    let bare = bare.trim_end_matches('/').to_ascii_lowercase();
    let last_numeric = bare.rsplit('.').next().is_some_and(|l| {
        l.bytes().all(|b| b.is_ascii_digit())
            || (l.starts_with("0x") && l[2..].bytes().all(|b| b.is_ascii_hexdigit()))
    });
    // The same egress rule as R7 and R8: a Databricks workspace domain, DNS labels, no account host.
    let ok = !bare.is_empty()
        && crate::http_transport::host_policy::is_workspace_host(&bare)
        && bare.len() <= 253
        && !last_numeric
        && bare.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    ok.then_some(bare)
}

/// A TOML basic string. Control characters and quotes are escaped, so a value cannot leave its
/// string.
pub fn toml_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                o.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn toml_list(items: &[String]) -> String {
    let v: Vec<String> = items.iter().map(|s| toml_str(s)).collect();
    format!("[{}]", v.join(", "))
}

fn path_text(p: &Path) -> Result<String, HomeError> {
    p.to_str().map(str::to_owned).ok_or(HomeError::BadValue)
}

/// The text of `config.toml`.
pub fn config_toml(spec: &HomeSpec) -> Result<String, HomeError> {
    let host = bare_host(&spec.host).ok_or(HomeError::BadHost)?;
    // The profile rule of R6: it is one argument, and it cannot look like a flag.
    if !crate::token_broker::valid_profile(&spec.profile) {
        return Err(HomeError::BadValue);
    }
    let cli = path_text(&spec.cli_path)?;
    if !spec.cli_path.is_absolute() {
        return Err(HomeError::BadValue);
    }
    let shim = path_text(&spec.shim_command)?;
    // The server runs in another folder: a relative shim command would start another file.
    if !spec.shim_command.is_absolute() {
        return Err(HomeError::BadValue);
    }
    let profile_arg = format!("--profile={}", spec.profile);
    let auth_args: Vec<String> = [
        "auth",
        "token",
        profile_arg.as_str(),
        "--timeout",
        "15s",
        "--output",
        "text",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let mut t = String::new();
    t.push_str("check_for_update_on_startup = false\n");
    t.push_str("model_provider = \"databricks\"\n");
    t.push_str(&format!("model = {}\n", toml_str(MODEL)));
    t.push_str("approval_policy = \"never\"\n");
    t.push_str("sandbox_mode = \"read-only\"\n");
    t.push_str("web_search = \"disabled\"\n");
    t.push_str("\n[memories]\ngenerate_memories = false\nuse_memories = false\n");
    t.push_str("\n[model_providers.databricks]\nname = \"Unity Gateway\"\n");
    t.push_str(&format!(
        "base_url = {}\n",
        toml_str(&format!("https://{host}{GATEWAY_PATH}"))
    ));
    t.push_str("wire_api = \"responses\"\n");
    t.push_str("\n[model_providers.databricks.auth]\n");
    t.push_str(&format!("command = {}\n", toml_str(&cli)));
    t.push_str(&format!("args = {}\n", toml_list(&auth_args)));
    t.push_str("timeout_ms = 60000\nrefresh_interval_ms = 900000\n");
    t.push_str("\n[features]\n");
    for f in FEATURES_OFF {
        t.push_str(&format!("{f} = false\n"));
    }
    t.push_str("\n[mcp_servers.bottle]\n");
    t.push_str(&format!("command = {}\n", toml_str(&shim)));
    t.push_str(&format!("args = {}\n", toml_list(&spec.shim_args)));
    t.push_str(&format!("env_vars = {}\n", toml_list(&spec.shim_env_names)));
    t.push_str("enabled_tools = [\"ask_genie\"]\nrequired = true\nstartup_timeout_sec = 30\n");
    t.push_str(&format!("tool_timeout_sec = {TOOL_TIMEOUT_SEC}\n"));
    t.push_str("default_tools_approval_mode = \"approve\"\n");
    t.push_str("\n[mcp_servers.bottle.tools.ask_genie]\napproval_mode = \"approve\"\n");
    Ok(t)
}

/// Delete GENIE_HOME. A missing folder is fine. A symlink is removed, not followed.
pub fn delete_home(home: &Path) -> io::Result<()> {
    match fs::symlink_metadata(home) {
        Ok(m) if m.file_type().is_symlink() => fs::remove_file(home),
        Ok(_) => fs::remove_dir_all(home),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn make_private_dir(p: &Path) -> io::Result<()> {
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(p)
}

fn write_private(p: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(p)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Delete GENIE_HOME and write it again: a 0700 folder with `config.toml` and
/// `answer.schema.json`, both 0600.
pub fn write_home(home: &Path, spec: &HomeSpec) -> Result<(), HomeError> {
    let config = config_toml(spec)?;
    delete_home(home)?;
    if let Some(parent) = home.parent() {
        make_private_dir(parent)?;
    }
    make_private_dir(home)?;
    write_private(&home.join("config.toml"), config.as_bytes())?;
    write_private(&home.join("answer.schema.json"), ANSWER_SCHEMA.as_bytes())?;
    Ok(())
}

/// A new private temp cwd for the thread: `<root>/bottle-genie-cwd-<16 hex>`, 0700.
pub fn make_cwd(root: &Path) -> io::Result<PathBuf> {
    let mut n = [0u8; 8];
    getrandom::fill(&mut n).map_err(|_| io::Error::other("no random"))?;
    let hex: String = n.iter().map(|b| format!("{b:02x}")).collect();
    let p = root.join(format!("{CWD_PREFIX}{hex}"));
    make_private_dir(&p)?;
    Ok(p)
}

/// Delete the temp cwd folders that an earlier run left in `root`.
pub fn sweep_cwds(root: &Path) {
    let Ok(rd) = fs::read_dir(root) else { return };
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().starts_with(CWD_PREFIX) {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

/// The environment of the server: a fixed allow-list, each variable only if it is set. The CLI
/// config file comes from `HOME` (a credential-source variable is never passed on; W-01a). `PATH` is
/// `/usr/bin:/bin` plus the folder of the CLI. `shim_env` holds the values of the shim variables.
pub fn server_env(
    var: &dyn Fn(&str) -> Option<String>,
    home: &Path,
    cli_dir: Option<&Path>,
    shim_env: &[(String, String)],
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = Vec::new();
    for name in [
        "HOME",
        "TMPDIR",
        "LANG",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "NO_PROXY",
        "ALL_PROXY",
        "SSL_CERT_FILE",
    ] {
        if let Some(v) = var(name) {
            env.push((name.to_owned(), v));
        }
    }
    env.push(("GENIE_HOME".to_owned(), home.to_string_lossy().into_owned()));
    let mut path = String::from("/usr/bin:/bin");
    if let Some(d) = cli_dir {
        path.push(':');
        path.push_str(&d.to_string_lossy());
    }
    env.push(("PATH".to_owned(), path));
    env.extend(shim_env.iter().cloned());
    env
}

/// The whole spawn spec of the server.
pub fn spawn_spec(genie: &Path, env: Vec<(String, String)>, cwd: Option<PathBuf>) -> SpawnSpec {
    SpawnSpec {
        program: genie.to_path_buf(),
        args: super::appserver::server_args(),
        env,
        cwd,
    }
}

/// The version that the server must report in `thread/start`.
pub fn expected_cli_version() -> &'static str {
    GENIE_PINNED
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> HomeSpec {
        HomeSpec {
            host: "https://Example-WS.cloud.databricks.com/".into(),
            cli_path: PathBuf::from("/opt/bottle/bin/databricks"),
            profile: "sales".into(),
            shim_command: PathBuf::from("/Applications/Bottle.app/Contents/MacOS/bottle"),
            shim_args: vec!["--agent-mcp-shim".into()],
            shim_env_names: vec![
                "BOTTLE_AGENT_SOCK".into(),
                "BOTTLE_AGENT_SESSION".into(),
                "BOTTLE_AGENT_SECRET".into(),
            ],
        }
    }

    #[test]
    fn config_toml_matches_the_golden_file() {
        let got = config_toml(&spec()).unwrap();
        assert_eq!(got, include_str!("golden_config.toml"));
    }

    #[test]
    fn every_lockdown_flag_is_off_and_one_mcp_server_offers_one_tool() {
        let got = config_toml(&spec()).unwrap();
        for f in FEATURES_OFF {
            assert!(got.contains(&format!("{f} = false\n")), "{f}");
        }
        assert_eq!(got.matches("[mcp_servers.").count(), 2); // the server and its one tool
        assert!(got.contains("enabled_tools = [\"ask_genie\"]"));
        assert!(got.contains("approval_policy = \"never\""));
        assert!(got.contains("sandbox_mode = \"read-only\""));
        assert!(got.contains("tool_timeout_sec = 600"));
        assert!(got.contains("use_memories = false"));
        assert!(!got.contains("dangerously") && !got.contains("approvals_reviewer"));
    }

    #[test]
    fn a_hostile_profile_or_host_cannot_leave_its_string() {
        let mut s = spec();
        s.profile = "a\"]\n[mcp_servers.evil]\ncommand=\"x".into();
        assert_eq!(config_toml(&s), Err(HomeError::BadValue));
        // The rule of R6: one safe argument, never a flag.
        for bad in ["a\"] b\\c", "--x", "-p", "a b", "", &"p".repeat(65)] {
            s.profile = bad.into();
            assert_eq!(config_toml(&s), Err(HomeError::BadValue), "{bad}");
        }
        s.profile = "sales_1.eu-west".into();
        let t = config_toml(&s).unwrap();
        assert!(t.contains("\"--profile=sales_1.eu-west\""));
        assert_eq!(t.matches("[mcp_servers.").count(), 2);
        for bad in [
            "evil.com/x",
            "evil.com:8443",
            "a b.com",
            "127.0.0.1",
            "",
            "u@evil.com",
            "evil.example.com",
            "accounts.cloud.databricks.com",
            "cloud.databricks.com",
        ] {
            let mut s = spec();
            s.host = bad.into();
            assert_eq!(config_toml(&s), Err(HomeError::BadHost), "{bad}");
        }
        let mut s = spec();
        s.cli_path = PathBuf::from("relative/databricks");
        assert_eq!(config_toml(&s), Err(HomeError::BadValue));
        let mut s = spec();
        s.shim_command = PathBuf::from("bottle");
        assert_eq!(config_toml(&s), Err(HomeError::BadValue));
    }

    #[test]
    fn the_config_holds_no_token_and_no_secret_value() {
        let got = config_toml(&spec()).unwrap();
        assert!(!got.contains("dapi") && !got.contains("Bearer"));
        // Only the names of the shim variables, never a value.
        assert!(got.contains("BOTTLE_AGENT_SECRET"));
    }

    #[cfg(unix)]
    #[test]
    fn write_home_makes_private_files_and_replaces_old_content() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempdir();
        let home = tmp.join("genie-home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("state_5.sqlite"), b"OLD").unwrap();
        write_home(&home, &spec()).unwrap();
        assert!(!home.join("state_5.sqlite").exists());
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&home), 0o700);
        assert_eq!(mode(&home.join("config.toml")), 0o600);
        assert_eq!(mode(&home.join("answer.schema.json")), 0o600);
        let schema: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(home.join("answer.schema.json")).unwrap())
                .unwrap();
        assert_eq!(schema["required"], serde_json::json!(["answer", "sources"]));
        delete_home(&home).unwrap();
        assert!(!home.exists());
        delete_home(&home).unwrap();
        let _ = fs::remove_dir_all(tmp);
    }

    #[cfg(unix)]
    #[test]
    fn delete_home_removes_a_symlink_and_not_its_target() {
        let tmp = tempdir();
        let target = tmp.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("keep"), b"x").unwrap();
        let link = tmp.join("genie-home");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        delete_home(&link).unwrap();
        assert!(!link.exists());
        assert!(target.join("keep").exists());
        let _ = fs::remove_dir_all(tmp);
    }

    #[test]
    fn the_server_env_is_an_allow_list() {
        let vars = |n: &str| match n {
            "HOME" => Some("/Users/x".to_owned()),
            "AWS_SECRET_ACCESS_KEY" => Some("nope".to_owned()),
            "HTTPS_PROXY" => Some("http://127.0.0.1:9".to_owned()),
            "DATABRICKS_TOKEN" => Some("nope".to_owned()),
            _ => None,
        };
        let env = server_env(
            &vars,
            Path::new("/h/genie-home"),
            Some(Path::new("/opt/cli")),
            &[("BOTTLE_AGENT_SECRET".into(), "ab".into())],
        );
        let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            names,
            [
                "HOME",
                "HTTPS_PROXY",
                "GENIE_HOME",
                "PATH",
                "BOTTLE_AGENT_SECRET"
            ]
        );
        assert_eq!(
            env.iter().find(|(k, _)| k == "PATH").unwrap().1,
            "/usr/bin:/bin:/opt/cli"
        );
    }

    #[cfg(unix)]
    fn tempdir() -> PathBuf {
        let mut n = [0u8; 6];
        getrandom::fill(&mut n).unwrap();
        let p = std::env::temp_dir().join(format!(
            "r1-home-{}",
            n.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
}
