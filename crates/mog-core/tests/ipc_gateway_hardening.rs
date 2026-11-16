//! #36: the hardened R12 IpcGateway (SEC-01, SEC-02a, SEC-02b, SEC-03, AT-17).
//!
//! - SEC-02a: each command only for its window labels, in the gateway and the capability files.
//! - SEC-02b: every payload is checked against a `deny_unknown_fields` schema.
//! - SEC-03 / SEC-01: replies, error codes and gateway log lines hold no rule-7 secret, also
//!   under a seeded, deterministic fuzz run (AT-17).
//!
//! Every fake secret is built at run time from pieces, so secret scanners do not flag the repo.
//! No test reads a real profile or a real token.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc::{self, IpcError, WindowClass, COMMANDS};
use mog_core::ipc_gateway::{
    hardened_gateway, hardened_gateway_with, ipc_error_code, CheckedGateway, GatewayComponents,
    GatewayLog, IpcGateway, ScrubbingGateway, ERROR_CODES,
};
use mog_core::token_guard::{contains_secret, json_contains_secret, REDACTED};
use serde_json::{json, Map, Value};

// ---------- fakes and fixtures ----------

fn hex32() -> String {
    "0123456789abcdef".repeat(2)
}

/// One fake for each rule-7 pattern (4.6), built at run time.
fn fakes() -> Vec<String> {
    vec![
        format!("{}{}.{}.sig", "ey", "JhbGciOiJub25lIn0", "eyJzdWIiOiJ4In0"),
        format!("{}{}", "dap", "i") + &hex32(),
        format!("{}{}", "do", "se") + "a1b2c3d4e5f6",
        format!("{}{}", "AK", "IA") + "ABCDEFGHIJ234567",
        format!("{}{}", "gh", "p_") + "FakeFakeFakeFakeFakeFakeFakeFake0000",
        format!("{}{} PRIVATE KEY-----\nMIIfake", "-----", "BEGIN"),
        format!("{}{}", "pass", "word=") + "hunter2",
    ]
}

/// A log sink that keeps every line.
#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<String>>>);

impl GatewayLog for Recorder {
    fn record(&self, line: &str) {
        self.0.lock().unwrap().push(line.to_owned());
    }
}

impl Recorder {
    fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

fn tags() -> Value {
    json!({"app": "bottle", "mog": "a1", "op": "genie"})
}

/// A valid payload for every command, and a window that may send it.
fn sample(command: &str) -> (&'static str, Value) {
    let t = tags();
    let args = match command {
        "cli_status" | "capture_status" | "capture_stop_all" | "mem_remote_status"
        | "config_list" | "policy_get" | "profiles_list" => json!({}),
        "cli_install" => json!({"consent": true}),
        "profile_status" => json!({"profile": "p"}),
        "profile_login" => {
            json!({"workspaceUrl": "https://x.cloud.databricks.com", "profile": "p"})
        }
        "profile_select" => json!({"mogId": "a1", "profile": "p", "scope": "question"}),
        "capture_start" => json!({"mogId": "a1", "mode": "ptt"}),
        "capture_stop" | "capture_pause" | "capture_resume" => {
            json!({"mogId": "a1", "leaseId": "l1"})
        }
        "capture_subscribe" => {
            json!({"onLevel": "__CHANNEL__:1", "onPartial": "__CHANNEL__:2"})
        }
        "session_attest" => json!({"mogId": "a1", "kind": "only_me", "noticeVersion": 1}),
        "stt_transcribe" => {
            json!({"segmentId": "s1", "provider": "local_parakeet", "tags": t})
        }
        "insert_text" => json!({"mogId": "a1", "text": "hello"}),
        "agent_start" => json!({"mogId": "a1", "harness": "claude"}),
        "agent_stop" | "mog_set_active" | "window_mog_close" | "window_card_show"
        | "window_card_hide" => json!({"mogId": "a1"}),
        "agent_send" => json!({"mogId": "a1", "turn": {}}),
        "agent_tool_result" => json!({"mogId": "a1", "callId": "c1", "result": {"ok": true}}),
        "model_invoke" => json!({"role": "router", "profile": "p", "request": {}, "tags": t}),
        "mcp_call" => json!({
            "profile": "p", "server": "genie_one", "tool": "ask", "args": {},
            "opId": "o1", "tags": t
        }),
        "dbx_http" => json!({"profile": "p", "routeId": "scim.me", "params": {}, "tags": t}),
        "mem_local_exec" | "mem_remote_exec" => json!({"stmtId": "s", "params": {}}),
        "config_read" | "config_delete" => json!({"id": "a1"}),
        "config_write" => json!({"id": "a1", "json": {}}),
        "hotkeys_set" => json!({"bindings": {}}),
        "permission_request" => return ("hub", json!({"permission": "accessibility"})),
        "window_mog_open" => json!({
            "mogId": "a1", "geometry": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0}
        }),
        "view_emit" => return ("mog-a1", json!({"mogId": "a1", "kind": "click"})),
        "audit_append" => json!({"kind": "k", "mogId": "a1", "opId": "o1", "sizes": {"chars": 1}}),
        "telemetry_span" => json!({"name": "n", "startMs": 1, "endMs": 2}),
        "brain_subscribe" => json!({"onEvent": "__CHANNEL__:3"}),
        other => panic!("no sample for {other}"),
    };
    ("brain", args)
}

/// A careless component: it echoes the command and its arguments, plus a fake secret, in its
/// reply, and counts its calls.
#[derive(Clone, Default)]
struct Leaky(Arc<Mutex<usize>>);

impl GatewayComponents for Leaky {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        *self.0.lock().unwrap() += 1;
        Ok(json!({"command": command, "args": args, "upstream": fakes()[1]}))
    }
}

impl Leaky {
    fn calls(&self) -> usize {
        *self.0.lock().unwrap()
    }
}

fn code_of(r: &CoreResult<Value>) -> &'static str {
    match r {
        Ok(_) => "ok",
        Err(e) => e.code(),
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// ---------- SEC-02a: per-window capabilities ----------

#[test]
fn sec_02a_view_window_cannot_call_mcp_call() {
    let gw = hardened_gateway(Recorder::default());
    let (_, args) = sample("mcp_call");
    for label in ["hub", "mog-a1", "card-a1"] {
        let r = gw.dispatch(label, "mcp_call", args.clone());
        assert_eq!(code_of(&r), "window_not_allowed", "{label}");
    }
    assert_eq!(
        code_of(&gw.dispatch("brain", "mcp_call", args)),
        "not_implemented"
    );
}

#[test]
fn sec_02a_every_command_is_refused_outside_its_window_classes() {
    let gw = hardened_gateway(Recorder::default());
    let labels = [
        ("brain", WindowClass::Brain),
        ("hub", WindowClass::Hub),
        ("mog-a1", WindowClass::Mog),
        ("card-a1", WindowClass::Card),
    ];
    for c in COMMANDS {
        let (_, args) = sample(c.name);
        for (label, class) in labels {
            let code = code_of(&gw.dispatch(label, c.name, args.clone()));
            if c.windows.contains(&class) {
                assert_ne!(code, "window_not_allowed", "{} from {label}", c.name);
            } else {
                assert_eq!(code, "window_not_allowed", "{} from {label}", c.name);
            }
        }
        // Labels outside ARC-3 (for example the C ABI's `ffi` before #176 decides) get nothing.
        for label in ["ffi", "main", "", "mog-", "mog-a b", "brain2"] {
            let code = code_of(&gw.dispatch(label, c.name, args.clone()));
            assert_eq!(code, "window_not_allowed", "{} from {label:?}", c.name);
        }
    }
}

#[test]
fn sec_02a_view_capability_files_do_not_grant_brain_commands() {
    for (file, class) in [
        ("hub", WindowClass::Hub),
        ("mog", WindowClass::Mog),
        ("card", WindowClass::Card),
    ] {
        let path = repo_root().join(format!("src-tauri/capabilities/{file}.json"));
        let cap: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let perms: Vec<&str> = cap["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap())
            .collect();
        assert!(!perms.contains(&"allow-mcp-call"), "{file} grants mcp_call");
        for c in COMMANDS.iter().filter(|c| !c.windows.contains(&class)) {
            let allow = format!("allow-{}", c.name.replace('_', "-"));
            assert!(!perms.contains(&allow.as_str()), "{file} grants {}", c.name);
        }
    }
}

// ---------- SEC-02b: deny_unknown_fields schemas ----------

#[test]
fn sec_02b_every_sample_payload_passes_the_schema() {
    let gw = hardened_gateway(Recorder::default());
    for c in COMMANDS {
        let (label, args) = sample(c.name);
        let code = code_of(&gw.dispatch(label, c.name, args));
        assert_eq!(code, "not_implemented", "{} sample", c.name);
    }
}

#[test]
fn sec_02b_payload_with_an_extra_field_is_rejected() {
    let gw = hardened_gateway(Recorder::default());
    for c in COMMANDS {
        let (label, mut args) = sample(c.name);
        args.as_object_mut()
            .unwrap()
            .insert("extra".into(), json!(1));
        let code = code_of(&gw.dispatch(label, c.name, args));
        assert_eq!(code, "invalid_args", "{} accepted an extra field", c.name);
    }
}

#[test]
fn sec_02b_extra_field_in_a_nested_struct_is_rejected() {
    let gw = hardened_gateway(Recorder::default());
    let cases = [
        ("mcp_call", "tags"),
        ("stt_transcribe", "tags"),
        ("window_mog_open", "geometry"),
    ];
    for (cmd, field) in cases {
        let (label, mut args) = sample(cmd);
        args[field]
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), json!(1));
        let code = code_of(&gw.dispatch(label, cmd, args));
        assert_eq!(code, "invalid_args", "{cmd}.{field}");
    }
    // The spec policy values (#259): any other value is now rejected as `invalid_args`.
    let mut policy = Map::new();
    for (f, v) in [
        ("checkInAtMin", 55),
        ("softCapMin", 60),
        ("extendMin", 30),
        ("silenceNudgeMin", 5),
        ("silenceStopMin", 15),
        ("hardCapMin", 240),
        ("lockStopMin", 10),
        ("holdMaxSec", 120),
        ("handsFreeMaxMin", 5),
    ] {
        policy.insert(f.into(), json!(v));
    }
    let ok = json!({"mogId": "a1", "mode": "session", "policy": policy.clone()});
    assert_eq!(
        code_of(&gw.dispatch("brain", "capture_start", ok)),
        "not_implemented"
    );
    policy.insert("extra".into(), json!(1));
    let bad = json!({"mogId": "a1", "mode": "session", "policy": policy});
    assert_eq!(
        code_of(&gw.dispatch("brain", "capture_start", bad)),
        "invalid_args"
    );
}

#[test]
fn sec_02b_every_object_schema_denies_unknown_fields() {
    fn walk(v: &Value, cmd: &str, path: &str) {
        match v {
            Value::Object(m) => {
                if m.contains_key("properties") {
                    assert_eq!(
                        m.get("additionalProperties"),
                        Some(&json!(false)),
                        "{cmd}: {path} allows unknown fields"
                    );
                }
                for (k, child) in m {
                    walk(child, cmd, &format!("{path}/{k}"));
                }
            }
            Value::Array(a) => a.iter().for_each(|c| walk(c, cmd, path)),
            _ => {}
        }
    }
    let path = repo_root().join("src-tauri/schemas/ipc-commands.json");
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let args = doc["arguments"].as_object().unwrap();
    assert_eq!(args.len(), COMMANDS.len());
    for (cmd, schema) in args {
        walk(schema, cmd, "");
    }
}

// ---------- SEC-03 / SEC-01: content-free errors and logs ----------

#[test]
fn sec_03_hardened_gateway_scrubs_component_replies() {
    let (_, mut args) = sample("insert_text");
    args["text"] = json!(fakes()[0]);
    // Without the scrub wrapper the component's reply leaks: the wrapper is what scrubs.
    let raw = CheckedGateway::with_components(Recorder::default(), Leaky::default())
        .dispatch("brain", "insert_text", args.clone())
        .unwrap();
    assert!(json_contains_secret(&raw), "{raw}");
    let reply = hardened_gateway_with(Recorder::default(), Leaky::default())
        .dispatch("brain", "insert_text", args)
        .unwrap();
    assert!(!json_contains_secret(&reply), "{reply}");
    assert_eq!(reply["upstream"], json!(REDACTED));
    assert_eq!(reply["command"], json!("insert_text"));
}

#[test]
fn sec_02a_sec_02b_refused_calls_never_reach_a_component() {
    let c = Leaky::default();
    let gw = hardened_gateway_with(Recorder::default(), c.clone());
    let (_, args) = sample("mcp_call");
    for label in ["hub", "mog-a1", "card-a1", "ffi"] {
        assert_eq!(
            code_of(&gw.dispatch(label, "mcp_call", args.clone())),
            "window_not_allowed"
        );
    }
    let mut extra = args.clone();
    extra["extra"] = json!(1);
    assert_eq!(
        code_of(&gw.dispatch("brain", "mcp_call", extra)),
        "invalid_args"
    );
    assert_eq!(
        code_of(&gw.dispatch("brain", "nope", json!({}))),
        "unknown_command"
    );
    assert_eq!(c.calls(), 0, "a refused call reached the component");
    assert_eq!(code_of(&gw.dispatch("brain", "mcp_call", args)), "ok");
    assert_eq!(c.calls(), 1);
}

#[test]
fn sec_03_unknown_command_and_label_are_not_echoed() {
    let log = Recorder::default();
    let gw = hardened_gateway(log.clone());
    for secret in fakes() {
        let r = gw.dispatch("brain", &secret, json!({}));
        assert_eq!(code_of(&r), "unknown_command");
        let r = gw.dispatch(&secret, "cli_status", json!({}));
        assert_eq!(code_of(&r), "window_not_allowed");
        // Some fakes are valid view IDs (`[A-Za-z0-9-]`); a brain command is refused for all.
        let r = gw.dispatch(&format!("mog-{secret}"), "cli_status", json!({}));
        assert_eq!(code_of(&r), "window_not_allowed");
    }
    let lines = log.lines();
    assert_eq!(lines.len(), 3 * fakes().len(), "one log line per call");
    for line in lines {
        assert!(!contains_secret(&line), "log leaked: {line}");
    }
}

#[test]
fn sec_03_each_ipc_error_maps_to_a_fixed_code() {
    let secret = fakes()[1].clone();
    let s = || secret.clone();
    let all = [
        IpcError::UnknownCommand,
        IpcError::WindowNotAllowed { command: s() },
        IpcError::InvalidArgs {
            command: s(),
            reason: s(),
        },
        IpcError::NotImplemented { command: s() },
        IpcError::HostFailed { command: s() },
        IpcError::UnknownLease { command: s() },
        IpcError::MicBusy { command: s() },
        IpcError::ConsentRequired { command: s() },
        IpcError::InvalidMode { command: s() },
        IpcError::MogCapReached { command: s() },
        IpcError::UnknownMog { command: s() },
        IpcError::WindowClosing { command: s() },
        IpcError::ConfigStorageFailed { command: s() },
    ];
    assert_eq!(all.len(), ERROR_CODES.len(), "one code per variant");
    for e in &all {
        let code = ipc_error_code(e);
        // The code is the serde tag of the variant, never a field value.
        assert_eq!(serde_json::to_value(e).unwrap()["error"], json!(code));
        assert!(ERROR_CODES.contains(&code), "{code}");
        let core = CoreError::IpcRefused { code };
        let shown = format!("{core} {core:?} {}", core.code());
        assert!(!contains_secret(&shown), "{shown}");
    }
}

#[test]
fn sec_03_gateway_log_line_is_content_free() {
    let log = Recorder::default();
    let gw = CheckedGateway::new(log.clone());
    let secret = fakes()[0].clone();
    let (_, mut args) = sample("insert_text");
    args["text"] = json!(secret);
    let _ = gw.dispatch("brain", "insert_text", args);
    let _ = gw.dispatch("mog-a1", "mcp_call", json!({"tool": secret}));
    let _ = gw.dispatch("card-zz", &secret, json!({}));
    let (label, view) = sample("view_emit");
    let _ = gw.dispatch(label, "view_emit", view);
    let with = CheckedGateway::with_components(log.clone(), Leaky::default());
    let (_, ok) = sample("cli_status");
    let _ = with.dispatch("hub", "cli_status", ok);
    assert_eq!(
        log.lines(),
        vec![
            "ipc command=insert_text window=brain outcome=not_implemented",
            "ipc command=mcp_call window=mog outcome=window_not_allowed",
            "ipc command=unknown window=card outcome=unknown_command",
            "ipc command=view_emit window=mog outcome=not_implemented",
            "ipc command=cli_status window=hub outcome=ok",
        ]
    );
}

#[test]
fn sec_03_production_code_has_no_direct_print() {
    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    for dir in [
        "crates/mog-core/src",
        "crates/mog-core-ffi/src",
        "src-tauri/src",
    ] {
        rust_files(&repo_root().join(dir), &mut files);
    }
    assert!(files.len() > 5);
    for path in files {
        // `testkit` is test-fixture code: its skip macro prints an env variable name only.
        if path.ends_with("testkit.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        // Skip only the trailing unit-test module, not code after another `#[cfg(test)]` item.
        let code = text
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap_or_default();
        for (n, line) in code.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for banned in ["println!(", "eprintln!(", "print!(", "eprint!(", "dbg!("] {
                assert!(
                    !line.contains(banned),
                    "{}:{}: {banned} bypasses the scrubbed gateway log (SEC-03)",
                    path.display(),
                    n + 1
                );
            }
        }
    }
}

// ---------- SEC-01 / SEC-03 / AT-17: seeded fuzz ----------

/// splitmix64: a small seeded PRNG, so the fuzz run is the same on every CI run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
    fn text(&mut self, secrets: &[String]) -> String {
        match self.below(4) {
            0 => self.pick(secrets).clone(),
            1 => format!("x {} y", self.pick(secrets)),
            _ => (0..self.below(12))
                .map(|_| (b'a' + self.below(26) as u8) as char)
                .collect(),
        }
    }
    fn json(&mut self, secrets: &[String], depth: u32) -> Value {
        match self.below(if depth == 0 { 4 } else { 6 }) {
            0 => Value::Null,
            1 => json!(self.below(1000)),
            2 | 3 => json!(self.text(secrets)),
            4 => Value::Array(
                (0..self.below(3))
                    .map(|_| self.json(secrets, depth - 1))
                    .collect(),
            ),
            _ => {
                let mut m = Map::new();
                for _ in 0..self.below(4) {
                    m.insert(self.text(secrets), self.json(secrets, depth - 1));
                }
                Value::Object(m)
            }
        }
    }
}

/// Put a fake secret somewhere in a valid payload: a string value, a new key or a whole value.
fn mutate(rng: &mut Rng, secrets: &[String], args: &mut Value) {
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    let keys: Vec<String> = obj.keys().cloned().collect();
    match rng.below(3) {
        0 if !keys.is_empty() => {
            let k = rng.pick(&keys).clone();
            obj.insert(k, json!(rng.pick(secrets).clone()));
        }
        1 => {
            obj.insert(rng.pick(secrets).clone(), json!(1));
        }
        _ => {
            if let Some(k) = keys.first() {
                obj.insert(k.clone(), rng.json(secrets, 2));
            }
        }
    }
}

struct FuzzStats {
    cases: usize,
    leaks: Vec<String>,
    reached_component: usize,
}

/// Send `cases` seeded calls to `gw` and scan every reply, error code and log line.
fn fuzz(gw: &dyn IpcGateway, log: &Recorder, seed: u64, cases: usize) -> FuzzStats {
    let secrets = fakes();
    let mut rng = Rng(seed);
    let mut labels: Vec<String> = ["brain", "hub", "mog-a1", "card-a1", "ffi", "main"]
        .map(String::from)
        .to_vec();
    labels.extend(secrets.iter().map(|s| format!("mog-{s}")));
    labels.extend(secrets.iter().cloned());
    let mut stats = FuzzStats {
        cases,
        leaks: Vec::new(),
        reached_component: 0,
    };
    for i in 0..cases {
        let command = match rng.below(5) {
            0 => rng.pick(&secrets).clone(),
            1 => rng.text(&secrets),
            _ => rng.pick(COMMANDS).name.to_owned(),
        };
        let (good_label, mut args) = if ipc::command(&command).is_some() {
            sample(&command)
        } else {
            ("brain", json!({}))
        };
        let label = if rng.below(2) == 0 {
            good_label.to_owned()
        } else {
            rng.pick(&labels).clone()
        };
        match rng.below(3) {
            0 => {}
            1 => mutate(&mut rng, &secrets, &mut args),
            _ => args = rng.json(&secrets, 3),
        }
        let r = gw.dispatch(&label, &command, args);
        match &r {
            Ok(v) if json_contains_secret(v) => stats.leaks.push(format!("case {i}: reply {v}")),
            Err(e) if contains_secret(e.code()) || contains_secret(&e.to_string()) => {
                stats.leaks.push(format!("case {i}: error {e}"))
            }
            _ => {}
        }
        if matches!(code_of(&r), "not_implemented" | "ok") {
            stats.reached_component += 1;
        }
    }
    for line in log.lines() {
        if contains_secret(&line) {
            stats.leaks.push(format!("log: {line}"));
        }
    }
    stats
}

#[test]
fn sec_01_at_17_ipc_fuzz_finds_no_secret() {
    let log = Recorder::default();
    // The component echoes every accepted payload with a secret, as a careless upstream would.
    let gw = hardened_gateway_with(log.clone(), Leaky::default());
    for seed in [1, 36, 0x5EC0] {
        let stats = fuzz(&gw, &log, seed, 3000);
        assert!(stats.leaks.is_empty(), "seed {seed}: {:#?}", stats.leaks);
        // The run must reach the components, not only the early checks.
        assert!(stats.reached_component > stats.cases / 20, "seed {seed}");
    }
    assert!(log.lines().len() >= 9000, "one log line per call");
}

#[test]
fn sec_01_at_17_fuzz_harness_catches_a_leaky_gateway() {
    // A component that echoes its arguments, as a careless upstream reply would.
    struct Echo;
    impl IpcGateway for Echo {
        fn dispatch(&self, _w: &str, command: &str, args: Value) -> CoreResult<Value> {
            Ok(json!({"command": command, "args": args}))
        }
    }
    let log = Recorder::default();
    let raw = fuzz(&Echo, &log, 36, 500);
    assert!(!raw.leaks.is_empty(), "the harness must see a leak");
    let scrubbed = fuzz(&ScrubbingGateway::new(Echo), &log, 36, 500);
    assert!(scrubbed.leaks.is_empty(), "{:#?}", scrubbed.leaks);
}

#[test]
fn sec_01_at_17_tauri_reachable_ipc_errors_hold_no_secret() {
    // The Tauri host calls `ipc::handle` with a command from `COMMANDS` and a label that the
    // app set, and sends the `IpcError` as JSON. Fuzz the payload on that path.
    let secrets = fakes();
    let mut rng = Rng(223);
    for _ in 0..3000 {
        let c = rng.pick(COMMANDS);
        let (label, mut args) = sample(c.name);
        let label = if rng.below(2) == 0 {
            label
        } else {
            *rng.pick(&["brain", "hub", "mog-a1", "card-a1"])
        };
        if rng.below(2) == 0 {
            mutate(&mut rng, &secrets, &mut args);
        } else {
            args = rng.json(&secrets, 3);
        }
        if let Err(e) = ipc::handle(label, c.name, args) {
            let shown = format!("{e} {e:?} {}", serde_json::to_string(&e).unwrap());
            assert!(!contains_secret(&shown), "{}: {shown}", c.name);
        }
    }
}
