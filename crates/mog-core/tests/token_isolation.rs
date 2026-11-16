//! Contract tests for the token isolation rules of section 4.6 (#224).
//!
//! Requirement IDs: ARC-6, ARC-7, MB-02, AT-17, COL-78. Each 4.6 rule has at least one test
//! named `arc_6_rule_<n>_...` or `arc_7_rule_<n>_...`.
//!
//! Every fake secret is built at run time from pieces, so secret scanners do not flag the repo.
//! No test reads a real profile or a real token.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mog_core::egress_policy::{self, TokenEgressPolicy};
use mog_core::ipc::{self, WindowClass, COMMANDS};
use mog_core::ipc_gateway::{IpcGateway, ScrubbingGateway};
use mog_core::loopback::{
    LoopbackEndpoint, LoopbackHandler, LoopbackRequest, LoopbackResponse, LoopbackSession,
    NotImplementedHandler, MAX_CONNECTIONS,
};
use mog_core::mcp_transport::forwardable_tool_args;
use mog_core::token_guard::{
    check_config_json, contains_secret, json_contains_secret, scrub, scrub_json,
    strip_profile_keys, ConfigSecret, PATTERNS, REDACTED,
};
use serde_json::{json, Value};

// ---------- fake secrets, built at run time ----------

fn hex32() -> String {
    "0123456789abcdef".repeat(2)
}

/// One fake for each of the 7 patterns of 4.6 rule 7, in the order of `PATTERNS`.
fn fakes() -> Vec<String> {
    vec![
        format!("{}{}.{}.sig", "ey", "JhbGciOiJub25lIn0", "eyJzdWIiOiJ4In0"),
        format!("{}{}", "dap", "i") + &hex32(),
        format!("{}{}", "do", "se") + "a1b2c3d4e5f6",
        format!("{}{}", "AK", "IA") + "ABCDEFGHIJ234567",
        format!("{}{}", "gh", "p_") + "FakeFakeFakeFakeFakeFakeFakeFake0000",
        format!(
            "{}{} PRIVATE KEY-----\nMIIfake\n{}{} PRIVATE KEY-----",
            "-----", "BEGIN", "-----", "END"
        ),
        format!("{}{}", "pass", "word=") + "hunter2",
    ]
}

fn pat() -> String {
    format!("{}{}", "dap", "i") + &hex32()
}

// ---------- rule 7: the scrubber ----------

#[test]
fn arc_6_rule_7_lists_exactly_the_seven_patterns() {
    assert_eq!(PATTERNS.len(), 7);
    assert_eq!(fakes().len(), PATTERNS.len());
}

#[test]
fn arc_6_rule_7_detects_each_pattern() {
    for (fake, pattern) in fakes().iter().zip(PATTERNS) {
        assert!(contains_secret(fake), "{pattern} not detected");
        let embedded = format!("before {fake} after");
        assert!(
            contains_secret(&embedded),
            "{pattern} not detected inside text"
        );
    }
}

#[test]
fn arc_6_rule_7_scrub_removes_each_pattern() {
    for (fake, pattern) in fakes().iter().zip(PATTERNS) {
        let text = format!("error: upstream said {fake} (retry)");
        let out = scrub(&text);
        assert!(!contains_secret(&out), "{pattern} survived: {out}");
        assert!(out.contains(REDACTED), "{pattern}: no marker in {out}");
        assert!(out.starts_with("error: upstream said "), "{pattern}: {out}");
    }
}

#[test]
fn arc_6_rule_7_scrub_removes_the_whole_secret() {
    let token = pat();
    let out = scrub(&format!("t={token}-2 end"));
    assert!(!out.contains(&hex32()[4..]), "{out}");
    let pw = scrub(&format!("{}{}hunter2&next=1", "pass", "word="));
    assert!(!pw.contains("hunter2"), "{pw}");
    assert!(pw.contains("next=1"), "{pw}");
    let pem = scrub(&format!("a {} b", fakes()[5]));
    assert!(!pem.contains("MIIfake"), "{pem}");
    assert!(pem.starts_with("a ") && pem.ends_with(" b"), "{pem}");
}

#[test]
fn arc_6_rule_7_scrub_keeps_clean_text() {
    for clean in [
        "",
        "Revenue by region for Q3",
        "dapi is short",        // dapi without 32 hex
        "AKIA123",              // too short
        "eyJ with no dot",      // no segment and dot
        "path=/x and pat=none", // no secret pattern
    ] {
        assert!(!contains_secret(clean), "false positive: {clean}");
        assert_eq!(scrub(clean), clean);
    }
}

#[test]
fn arc_6_rule_7_scrub_is_linear_on_hostile_input() {
    // A run of `eyJ` with no dot made the first scrubber quadratic (review of #308).
    for unit in ["eyJ", "AKIA", "dapi", "-----BEGIN ", "ghp_", "dose"] {
        let text = unit.repeat(400_000 / unit.len());
        let t = std::time::Instant::now();
        let _ = scrub(&text);
        let _ = contains_secret(&text);
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "{unit}: {:?}",
            t.elapsed()
        );
    }
}

#[test]
fn arc_6_rule_7_scrub_json_keeps_values_of_colliding_keys() {
    let a = pat();
    let b = fakes()[3].clone();
    let out = scrub_json(&json!({ a: 1, b: 2 }));
    let m = out.as_object().unwrap();
    assert_eq!(m.len(), 2, "{out}");
    let mut vals: Vec<i64> = m.values().map(|v| v.as_i64().unwrap()).collect();
    vals.sort_unstable();
    assert_eq!(vals, [1, 2]);
    assert!(!json_contains_secret(&out));
}

#[test]
fn arc_6_rule_7_scrub_handles_multibyte_text() {
    let text = format!("é ü {} 漢字", pat());
    let out = scrub(&text);
    assert!(out.starts_with("é ü ") && out.ends_with(" 漢字"), "{out}");
    assert!(!contains_secret(&out));
}

#[test]
fn arc_6_rule_7_scrub_json_cleans_keys_and_values() {
    let token = pat();
    let v = json!({
        "ok": "plain",
        "nested": {"list": [1, token.clone(), {"deep": fakes()[0]}]},
        token.clone(): true,
    });
    assert!(json_contains_secret(&v));
    let out = scrub_json(&v);
    assert!(!json_contains_secret(&out), "{out}");
    assert!(!out.to_string().contains(&token));
    assert_eq!(out["ok"], "plain");
    assert_eq!(out["nested"]["list"][0], 1);
}

#[test]
fn arc_6_rule_7_ipc_gateway_replies_are_scrubbed() {
    struct Leaky;
    impl IpcGateway for Leaky {
        fn dispatch(
            &self,
            _window_label: &str,
            _command: &str,
            _args: Value,
        ) -> mog_core::error::CoreResult<Value> {
            Ok(json!({"text": format!("upstream echoed {}", pat())}))
        }
    }
    let reply = ScrubbingGateway::new(Leaky)
        .dispatch("brain", "mcp_call", json!({}))
        .unwrap();
    assert!(!json_contains_secret(&reply), "{reply}");
}

#[test]
fn arc_6_rule_7_core_errors_carry_no_secret() {
    use mog_core::error::CoreError;
    for e in [
        CoreError::NotImplemented { component: "R12" },
        CoreError::TokenCommandRefused { component: "R5" },
        CoreError::Unavailable {
            component: "R16",
            reason: mog_core::loopback::UNAVAILABLE,
        },
    ] {
        let shown = format!("{e} {e:?} {}", e.code());
        assert!(!contains_secret(&shown), "{shown}");
    }
}

// ---------- rule 5: mcp_call strips profile keys ----------

#[test]
fn arc_6_rule_5_mcp_call_strips_profile_keys() {
    let mut args: BTreeMap<String, Value> = serde_json::from_value(json!({
        "question": "sales by region",
        "profile": "other-workspace",
        "Profile": "x",
        "options": {"profile": "nested", "keep": 1},
        "list": [{"PROFILE": "y", "keep": 2}],
    }))
    .unwrap();
    let removed = strip_profile_keys(&mut args);
    assert_eq!(removed, 4);
    let text = serde_json::to_string(&args).unwrap().to_lowercase();
    assert!(!text.contains("\"profile\""), "{text}");
    assert_eq!(args["question"], "sales by region");
    assert_eq!(args["options"]["keep"], 1);
    assert_eq!(args["list"][0]["keep"], 2);
}

#[test]
fn arc_6_rule_5_mcp_transport_forwards_only_stripped_args() {
    let args: BTreeMap<String, Value> =
        serde_json::from_value(json!({"profile": "p2", "q": "x"})).unwrap();
    let out = forwardable_tool_args(args);
    assert!(!out.contains_key("profile"));
    assert_eq!(out["q"], "x");
}

// ---------- MB-02: config_write rejects secrets ----------

fn obj(v: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(v).unwrap()
}

#[test]
fn mb_02_config_rejects_token_like_values() {
    for fake in fakes() {
        let cfg = obj(json!({"name": "Sales MOG", "notes": [ {"text": fake} ]}));
        assert_eq!(check_config_json(&cfg), Err(ConfigSecret::TokenLikeValue));
    }
}

#[test]
fn mb_02_config_rejects_secret_named_keys() {
    for key in [
        "token",
        "Token",
        "accessToken",
        "api_token",
        "password",
        "db-password",
        "secret",
        "clientSecret",
        "pat",
        "PAT",
        "github_pat",
        "APIToken",
        "accesstoken",
        "clientsecret",
        "password1",
        "authToken2",
        "SECRETKey",
        "PATValue",
        "db_passwd",
    ] {
        let cfg = obj(json!({"name": "m", "auth": {key: "x"}}));
        assert_eq!(
            check_config_json(&cfg),
            Err(ConfigSecret::SecretNamedKey),
            "{key}"
        );
    }
}

#[test]
fn mb_02_config_accepts_a_clean_mog_config() {
    let cfg = obj(json!({
        "name": "Sales MOG",
        "profile": "work",
        "path": "/tmp/x",
        "maxTokens": 512,
        "patterns": ["a"],
        "homeDomain": "sales",
        "genieSpaces": [{"id": "01ef", "title": "Revenue"}],
    }));
    assert_eq!(check_config_json(&cfg), Ok(()));
}

#[test]
fn mb_02_config_rejection_codes_are_content_free() {
    for e in [ConfigSecret::SecretNamedKey, ConfigSecret::TokenLikeValue] {
        assert!(!e.code().is_empty());
        assert!(!contains_secret(&format!("{e} {e:?} {}", e.code())));
    }
}

// ---------- rules 2, 3, 6: the egress seam (implementation is #207) ----------

/// Allows everything. A policy that does this must fail the contract.
struct AllowAll;
impl TokenEgressPolicy for AllowAll {
    fn may_send_token(&self, _profile_host: &str, _url: &str) -> bool {
        true
    }
    fn may_follow_redirect_with_authorization(&self, _p: &str, _from: &str, _to: &str) -> bool {
        true
    }
    fn reply_for_ts(&self, _content_type: Option<&str>, body: &[u8]) -> Option<Value> {
        Some(Value::String(String::from_utf8_lossy(body).into_owned()))
    }
    fn may_retry_with_token_after_non_json(&self) -> bool {
        true
    }
}

/// Refuses everything. A policy that does this must fail the contract too.
struct DenyAll;
impl TokenEgressPolicy for DenyAll {
    fn may_send_token(&self, _profile_host: &str, _url: &str) -> bool {
        false
    }
    fn may_follow_redirect_with_authorization(&self, _p: &str, _from: &str, _to: &str) -> bool {
        false
    }
    fn reply_for_ts(&self, _content_type: Option<&str>, _body: &[u8]) -> Option<Value> {
        None
    }
    fn may_retry_with_token_after_non_json(&self) -> bool {
        false
    }
}

fn rules_hit(violations: &[egress_policy::Violation]) -> Vec<u8> {
    let mut rules: Vec<u8> = violations.iter().map(|v| v.rule).collect();
    rules.dedup();
    rules
}

#[test]
fn arc_6_rule_2_contract_fails_a_policy_that_sends_tokens_anywhere() {
    let v = egress_policy::contract_violations(&AllowAll);
    assert!(rules_hit(&v).contains(&2), "{v:?}");
    for host_case in [
        "http://",
        ":8443",
        "accounts.",
        "attacker",
        "cloud.databricks.com.",
    ] {
        assert!(
            v.iter().any(|x| x.rule == 2 && x.case.contains(host_case)),
            "no rule-2 case for {host_case}: {v:?}"
        );
    }
}

#[test]
fn arc_6_rule_2_contract_fails_a_policy_that_blocks_the_profile_host() {
    let v = egress_policy::contract_violations(&DenyAll);
    assert!(v.iter().any(|x| x.rule == 2), "{v:?}");
}

#[test]
fn arc_6_rule_3_contract_fails_a_policy_that_follows_cross_host_redirects() {
    let v = egress_policy::contract_violations(&AllowAll);
    assert!(rules_hit(&v).contains(&3), "{v:?}");
    let v = egress_policy::contract_violations(&DenyAll);
    assert!(
        v.iter().any(|x| x.rule == 3),
        "same-host case missing: {v:?}"
    );
}

#[test]
fn arc_6_rule_6_contract_fails_a_policy_that_echoes_pages() {
    let v = egress_policy::contract_violations(&AllowAll);
    assert!(
        v.iter().filter(|x| x.rule == 6).count() >= 3,
        "html, wrong content type and retry cases: {v:?}"
    );
    let v = egress_policy::contract_violations(&DenyAll);
    assert!(
        v.iter().any(|x| x.rule == 6),
        "JSON pass-through case missing: {v:?}"
    );
}

#[test]
fn arc_6_egress_contract_covers_rules_2_3_and_6() {
    assert_eq!(egress_policy::CONTRACT_RULES, [2, 3, 6]);
    // TODO(#207): run `egress_policy::contract_violations` against `http_policy::TokenPolicy`
    // through a small adapter once #256 merges, and assert that it returns no violation.
}

// ---------- rule 1: no command takes a URL ----------

#[test]
fn arc_6_rule_1_no_command_takes_a_url_argument() {
    fn walk(v: &Value, cmd: &str, path: &str) {
        if let Value::Object(m) = v {
            if let Some(Value::Object(props)) = m.get("properties") {
                for (k, sub) in props {
                    let lk = k.to_lowercase();
                    let urlish = lk.ends_with("url") || lk.ends_with("uri") || lk == "endpoint";
                    // 4.5 lists `profile_login {workspaceUrl}` for `databricks auth login
                    // --host`; it is not a request target. Rule 1 names only the v1
                    // `custom_agent` URL, so this difference is recorded in ADR 0013 and #224.
                    let allowed = cmd == "profile_login" && k == "workspaceUrl";
                    assert!(!urlish || allowed, "{cmd} takes a URL field {path}{k}");
                    walk(sub, cmd, &format!("{path}{k}."));
                }
            }
            for sub in m.values() {
                walk(sub, cmd, path);
            }
        }
    }
    for (cmd, schema) in ipc::schemas() {
        walk(&schema, cmd, "");
    }
    let tags = json!({"app": "bottle", "mog": "a1", "op": "genie"});
    let err = ipc::handle(
        "brain",
        "dbx_http",
        json!({"profile": "p", "routeId": "https://evil.example/x", "params": {}, "tags": tags}),
    );
    assert!(err.is_err());
}

// ---------- rule 4: CSP and Rive ----------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn directive<'a>(csp: &'a str, name: &str) -> Vec<&'a str> {
    csp.split(';')
        .map(str::trim)
        .find_map(|d| d.strip_prefix(name))
        .map(|rest| rest.split_whitespace().collect())
        .unwrap_or_default()
}

#[test]
fn arc_6_rule_4_csp_blocks_external_connect_src() {
    let conf: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("src-tauri/tauri.conf.json")).unwrap(),
    )
    .unwrap();
    let csp = conf["app"]["security"]["csp"]
        .as_str()
        .expect("a CSP string");
    let connect = directive(csp, "connect-src");
    assert!(!connect.is_empty(), "connect-src must be explicit: {csp}");
    for src in connect {
        assert!(
            src == "ipc:" || src == "http://ipc.localhost" || src == "'self'",
            "external connect-src {src}"
        );
    }
}

#[test]
fn arc_6_rule_4_rive_asset_cdn_is_never_enabled() {
    // Rive loads assets from its CDN unless `enableRiveAssetCDN` is false, so every file that
    // uses the Rive runtime must set it to false. No Rive code exists in `src/` yet (#3, #11).
    fn scan(dir: &Path, hits: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                scan(&p, hits);
            } else if matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("ts" | "tsx" | "js" | "jsx")
            ) {
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
                let uses_rive = ["@rive-app", "newRive(", "useRive(", "RiveCanvas"]
                    .iter()
                    .any(|m| compact.contains(m));
                let off = compact.contains("enableRiveAssetCDN:false");
                let on = compact.contains("enableRiveAssetCDN:true")
                    || compact.contains("enableRiveAssetCDN=true");
                if on || (uses_rive && !off) {
                    hits.push(p);
                }
            }
        }
    }
    let mut hits = Vec::new();
    scan(&repo_root().join("src"), &mut hits);
    assert!(
        hits.is_empty(),
        "Rive asset CDN is not set to false in {hits:?}"
    );
}

// ---------- rule 8 (v1): Lakebase ----------

#[test]
fn arc_6_rule_8_lakebase_is_off_in_the_default_build() {
    // TODO(#193): test that the Lakebase credential goes only to the endpoint host that the
    // same home profile returns, when R9 LakebasePool lands (v1).
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let default = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("default"))
        .expect("a default feature list");
    assert!(
        !default.contains("lakebase"),
        "R9 is v1 and off by default: {default}"
    );
}

// ---------- AT-17: no IPC return carries a token ----------

fn label_for(class: WindowClass) -> &'static str {
    match class {
        WindowClass::Brain => "brain",
        WindowClass::Hub => "hub",
        WindowClass::Mog => "mog-a1",
        WindowClass::Card => "card-a1",
    }
}

fn secret_args(secret: &str) -> Vec<Value> {
    let tags = json!({"app": "bottle", "mog": "a1", "op": secret});
    vec![
        json!({ secret: 1 }),
        json!({
            "profile": secret, "mogId": secret, "id": secret, "json": {"token": secret},
            "tool": secret, "args": {"profile": secret}, "opId": secret, "tags": tags,
            "routeId": secret, "params": {"q": secret}, "body": secret, "text": secret,
            "stmtId": secret, "workspaceUrl": format!("https://x/?t={secret}"),
            "segmentId": secret, "kind": secret, "payload": secret, "callId": secret,
            "result": secret, "turn": {"t": secret}, "request": {"r": secret},
            "consentToken": secret, "leaseId": secret, "onEvent": secret,
        }),
        json!(secret),
        json!([secret]),
    ]
}

#[test]
fn at_17_no_ipc_return_contains_a_token_pattern() {
    for secret in fakes() {
        for spec in COMMANDS {
            for &class in spec.windows {
                for args in secret_args(&secret) {
                    let shown = match ipc::handle(label_for(class), spec.name, args) {
                        Ok(h) => format!("{h:?}"),
                        Err(e) => format!("{e} {e:?} {}", serde_json::to_string(&e).unwrap()),
                    };
                    assert!(
                        !contains_secret(&shown),
                        "{} from {class:?} leaked: {shown}",
                        spec.name
                    );
                }
            }
        }
    }
}

// ---------- rule 9 / ARC-7 / COL-78: loopback endpoints ----------

fn http(addr: SocketAddr, host: &str, auth: Option<&str>, body: &str) -> (u16, String) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut req = format!(
        "POST /mcp HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(a) = auth {
        req.push_str(&format!("Authorization: {a}\r\n"));
    }
    req.push_str("Connection: close\r\n\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let status = out
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    (status, out)
}

struct Counting(AtomicUsize);
impl LoopbackHandler for Counting {
    fn handle(&self, req: &LoopbackRequest) -> LoopbackResponse {
        self.0.fetch_add(1, Ordering::SeqCst);
        LoopbackResponse {
            status: 200,
            body: json!({"endpoint": req.endpoint.name(), "leak": pat()}),
        }
    }
}

fn host_of(addr: SocketAddr) -> String {
    format!("127.0.0.1:{}", addr.port())
}

#[test]
fn arc_7_rule_9_session_has_three_loopback_endpoints() {
    let s = LoopbackSession::open(Arc::new(NotImplementedHandler)).unwrap();
    assert_eq!(LoopbackEndpoint::ALL.len(), 3);
    let names: Vec<&str> = LoopbackEndpoint::ALL.iter().map(|e| e.name()).collect();
    assert_eq!(names, ["bottle-mcp", "genie-one-mcp", "model-proxy"]);
    let mut ports = Vec::new();
    for e in LoopbackEndpoint::ALL {
        let a = s.addr(e);
        assert!(a.ip().is_loopback(), "{e:?} bound to {a}");
        ports.push(a.port());
    }
    ports.sort_unstable();
    ports.dedup();
    assert_eq!(ports.len(), 3);
}

#[test]
fn arc_7_rule_9_loopback_rejects_calls_without_the_secret() {
    let handler = Arc::new(Counting(AtomicUsize::new(0)));
    let s = LoopbackSession::open(handler.clone()).unwrap();
    for e in LoopbackEndpoint::ALL {
        let a = s.addr(e);
        let (status, _) = http(a, &host_of(a), None, "{}");
        assert_eq!(status, 401, "{e:?} without a secret");
        let (status, _) = http(a, &host_of(a), Some("Bearer wrong"), "{}");
        assert_eq!(status, 401, "{e:?} with a wrong secret");
        let (status, _) = http(a, &host_of(a), Some("Basic Zm9vOmJhcg=="), "{}");
        assert_eq!(status, 401, "{e:?} with another scheme");
    }
    assert_eq!(handler.0.load(Ordering::SeqCst), 0);
}

#[test]
fn arc_7_rule_9_loopback_accepts_the_session_secret() {
    let handler = Arc::new(Counting(AtomicUsize::new(0)));
    let s = LoopbackSession::open(handler.clone()).unwrap();
    let cfg = s.harness_config();
    for e in LoopbackEndpoint::ALL {
        let a = s.addr(e);
        let (status, out) = http(a, &host_of(a), Some(cfg.authorization()), "{}");
        assert_eq!(status, 200, "{e:?}: {out}");
        assert!(out.contains(e.name()), "{out}");
        assert!(!contains_secret(&out), "the reply was not scrubbed: {out}");
    }
    assert_eq!(handler.0.load(Ordering::SeqCst), 3);
}

#[test]
fn arc_7_rule_9_default_handler_is_not_implemented() {
    let s = LoopbackSession::open(Arc::new(NotImplementedHandler)).unwrap();
    let a = s.addr(LoopbackEndpoint::BottleMcp);
    let (status, _) = http(
        a,
        &host_of(a),
        Some(s.harness_config().authorization()),
        "{}",
    );
    assert_eq!(status, 501);
}

#[test]
fn arc_7_rule_9_loopback_rejects_a_foreign_host_header() {
    let handler = Arc::new(Counting(AtomicUsize::new(0)));
    let s = LoopbackSession::open(handler.clone()).unwrap();
    let a = s.addr(LoopbackEndpoint::ModelProxy);
    let auth = s.harness_config().authorization().to_owned();
    let (status, _) = http(a, "evil.example", Some(&auth), "{}");
    assert_eq!(status, 403, "DNS rebinding guard");
    let (status, _) = http(a, &format!("localhost:{}", a.port()), Some(&auth), "{}");
    assert_eq!(status, 200);
    assert_eq!(handler.0.load(Ordering::SeqCst), 1);
}

#[test]
fn arc_7_rule_9_each_session_has_its_own_secret() {
    let a = LoopbackSession::open(Arc::new(NotImplementedHandler)).unwrap();
    let b = LoopbackSession::open(Arc::new(NotImplementedHandler)).unwrap();
    let auth_a = a.harness_config().authorization().to_owned();
    assert_ne!(auth_a, b.harness_config().authorization());
    let secret = auth_a.strip_prefix("Bearer ").expect("bearer scheme");
    assert!(secret.len() >= 64, "at least 256 bits as hex");
    let addr = b.addr(LoopbackEndpoint::GenieOneMcp);
    let (status, _) = http(addr, &host_of(addr), Some(&auth_a), "{}");
    assert_eq!(status, 401, "session A's secret opens session B");
}

#[test]
fn arc_7_col_78_harness_config_carries_no_token() {
    let s = LoopbackSession::open(Arc::new(NotImplementedHandler)).unwrap();
    let cfg = s.harness_config();
    assert_eq!(cfg.endpoints().len(), 3);
    for (e, url) in cfg.endpoints() {
        assert!(url.starts_with("http://127.0.0.1:"), "{e:?}: {url}");
    }
    let shown = format!("{cfg:?}");
    assert!(
        !shown.contains(cfg.authorization()),
        "Debug shows the secret"
    );
    assert!(!contains_secret(cfg.authorization()));
    for (_, url) in cfg.endpoints() {
        assert!(!contains_secret(url));
    }
}

#[test]
fn arc_7_rule_9_closed_session_stops_listening() {
    let s = LoopbackSession::open(Arc::new(NotImplementedHandler)).unwrap();
    let a = s.addr(LoopbackEndpoint::BottleMcp);
    s.close();
    let ok = TcpStream::connect_timeout(&a, Duration::from_millis(300));
    if let Ok(mut c) = ok {
        // A late accept may still take the connection; it must not answer with data.
        c.set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let _ = c.write_all(b"GET / HTTP/1.1\r\n\r\n");
        let mut buf = [0u8; 16];
        assert!(matches!(c.read(&mut buf), Ok(0) | Err(_)));
    }
}

#[test]
fn arc_7_rule_9_open_connections_are_capped() {
    let s = LoopbackSession::open(Arc::new(NotImplementedHandler)).unwrap();
    let a = s.addr(LoopbackEndpoint::BottleMcp);
    let idle: Vec<TcpStream> = (0..MAX_CONNECTIONS + 16)
        .filter_map(|_| TcpStream::connect(a).ok())
        .collect();
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        s.open_connections() <= MAX_CONNECTIONS * LoopbackEndpoint::ALL.len(),
        "{} connections for {} clients",
        s.open_connections(),
        idle.len()
    );
}

#[test]
fn arc_7_rule_9_slow_requests_hit_the_deadline() {
    let s = LoopbackSession::open(Arc::new(NotImplementedHandler)).unwrap();
    let a = s.addr(LoopbackEndpoint::ModelProxy);
    let mut c = TcpStream::connect(a).unwrap();
    let mut writer = c.try_clone().unwrap();
    // One byte per second never finishes the head; each read alone is well under 5 s.
    let drip = std::thread::spawn(move || {
        for b in b"POST /mcp HTTP/1.1\r\nHost: x" {
            if writer.write_all(&[*b]).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
    c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let t = std::time::Instant::now();
    let mut out = String::new();
    let _ = c.read_to_string(&mut out);
    assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
    assert!(out.starts_with("HTTP/1.1 408"), "{out}");
    let _ = c.shutdown(std::net::Shutdown::Both);
    let _ = drip.join();
}

#[test]
fn arc_7_rule_9_close_ends_open_connections() {
    let handler = Arc::new(Counting(AtomicUsize::new(0)));
    let s = LoopbackSession::open(handler.clone()).unwrap();
    let a = s.addr(LoopbackEndpoint::GenieOneMcp);
    let auth = s.harness_config().authorization().to_owned();
    let mut c = TcpStream::connect(a).unwrap();
    // Half a request with the right secret: the handler must never run after close.
    let head = format!(
        "POST /mcp HTTP/1.1\r\nHost: {}\r\nAuthorization: {auth}\r\nContent-Length: 10\r\n\r\n{{",
        host_of(a)
    );
    c.write_all(head.as_bytes()).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let t = std::time::Instant::now();
    s.close();
    assert!(
        t.elapsed() < Duration::from_secs(2),
        "close waited {:?}",
        t.elapsed()
    );
    c.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut buf = [0u8; 64];
    assert!(matches!(c.read(&mut buf), Ok(0) | Err(_)));
    assert_eq!(handler.0.load(Ordering::SeqCst), 0);
}

struct Panicking;
impl LoopbackHandler for Panicking {
    fn handle(&self, _req: &LoopbackRequest) -> LoopbackResponse {
        panic!("handler bug");
    }
}

#[test]
fn arc_7_rule_9_a_panicking_handler_frees_its_connection() {
    let s = LoopbackSession::open(Arc::new(Panicking)).unwrap();
    let a = s.addr(LoopbackEndpoint::BottleMcp);
    let auth = s.harness_config().authorization().to_owned();
    for _ in 0..MAX_CONNECTIONS + 4 {
        let (status, _) = http(a, &host_of(a), Some(&auth), "{}");
        assert_eq!(status, 500);
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(s.open_connections(), 0);
}

#[test]
fn arc_7_rule_9_the_connection_cap_is_per_endpoint() {
    let s = LoopbackSession::open(Arc::new(NotImplementedHandler)).unwrap();
    let a = s.addr(LoopbackEndpoint::BottleMcp);
    let _idle: Vec<TcpStream> = (0..MAX_CONNECTIONS + 4)
        .filter_map(|_| TcpStream::connect(a).ok())
        .collect();
    std::thread::sleep(Duration::from_millis(300));
    let b = s.addr(LoopbackEndpoint::ModelProxy);
    let (status, _) = http(
        b,
        &host_of(b),
        Some(s.harness_config().authorization()),
        "{}",
    );
    assert_eq!(
        status, 501,
        "a full bottle-mcp endpoint must not lock out the model proxy"
    );
}
