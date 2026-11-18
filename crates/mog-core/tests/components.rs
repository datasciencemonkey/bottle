//! Contract tests for the 17 Rust core components of section 4.3 (R1-R14, R16, R17; R15 is in
//! `mog-core-ffi`). Each stub must report `NotImplemented` with its own component ID until its
//! feature issue lands; R2 CaptureService has landed and is tested in `capture_service.rs`;
//! R16 AgentHost has landed and is tested in `agent_host.rs` (#112).
//! ARC-6: only R6 TokenBroker names the token type.

use std::path::{Path, PathBuf};

use mog_core::error::{CoreError, CoreResult};

fn not_implemented<T: std::fmt::Debug>(result: CoreResult<T>, component: &'static str) {
    match result {
        Err(CoreError::NotImplemented { component: got }) => assert_eq!(got, component),
        other => panic!("expected NotImplemented for {component}, got {other:?}"),
    }
}

#[test]
fn r1_hotkey_service_stub_reports_not_implemented() {
    use mog_core::hotkey::{HotkeyService, StubHotkeyService};
    let s = StubHotkeyService;
    not_implemented(s.start(), "R1");
    not_implemented(s.stop(), "R1");
    not_implemented(s.secure_input_active(), "R1");
}

#[test]
fn r3_session_guard_stub_reports_not_implemented() {
    use mog_core::session_guard::{SessionGuard, StubSessionGuard};
    let s = StubSessionGuard::default();
    not_implemented(s.arm("session"), "R3");
    not_implemented(s.mic_release_state(), "R3");
}

#[test]
fn r3_mic_release_has_three_states() {
    use mog_core::session_guard::MicRelease;
    let all = [
        MicRelease::Active,
        MicRelease::Released,
        MicRelease::Unknown,
    ];
    assert_eq!(all.len(), 3);
}

#[test]
fn r4_stt_engine_without_an_installed_model_is_unavailable() {
    use mog_core::error::CoreError;
    use mog_core::stt::SttEngine;
    use mog_core::stt_engine::{LocalStt, NoModelLoader, NoTranscriptSink};
    use std::sync::Arc;
    // #46: R4 is built. With no installed model (or a build without `local-stt`) it reports a
    // content-free code, not a stub error.
    let s = LocalStt::new(Arc::new(NoModelLoader), Arc::new(NoTranscriptSink), |_| {});
    assert_eq!(
        s.load_model(),
        Err(CoreError::Unavailable {
            component: "R4",
            reason: "stt_model_missing"
        })
    );
    assert_eq!(
        s.transcribe("segment"),
        Err(CoreError::Unavailable {
            component: "R4",
            reason: "stt_segment_unknown"
        })
    );
}

#[test]
fn r5_cli_runner_stub_reports_not_implemented() {
    use mog_core::cli_runner::{CliRunner, StubCliRunner};
    let s = StubCliRunner;
    not_implemented(s.resolve(), "R5");
    not_implemented(s.run_json(&["auth", "profiles"]), "R5");
}

#[test]
fn r6_token_broker_stub_reports_not_implemented() {
    use mog_core::token_broker::{StubTokenBroker, TokenBroker};
    let s = StubTokenBroker;
    let mut called = false;
    not_implemented(
        s.with_token("p", "h", &mut |_bytes: &[u8]| called = true),
        "R6",
    );
    assert!(!called, "the stub must not call the closure");
    not_implemented(
        s.invalidate("p", mog_core::token_broker::TokenLease::default()),
        "R6",
    );
    not_implemented(s.workspace_id("p"), "R6");
}

#[test]
fn r6_secret_token_debug_is_redacted() {
    use mog_core::token_broker::SecretToken;
    let raw = format!("dapi{}", "0123456789abcdef0123456789abcdef");
    let token = SecretToken::new(raw.clone());
    let shown = format!("{token:?}");
    assert!(!shown.contains(&raw), "Debug leaks the token: {shown}");
    assert!(
        !shown.contains("dapi"),
        "Debug leaks the token prefix: {shown}"
    );
    assert!(
        shown.contains("redacted"),
        "Debug must say it is redacted: {shown}"
    );
}

#[test]
fn r6_secret_token_wipe_overwrites_with_zeros() {
    use mog_core::token_broker::SecretToken;
    let mut token = SecretToken::new(["ey", "JhbGciOiJSUzI1NiJ9", ".payload.sig"].concat());
    assert!(!token.is_zeroed());
    assert_eq!(token.len(), 32);
    token.wipe();
    assert!(token.is_zeroed());
}

#[test]
fn r7_http_transport_stub_reports_not_implemented() {
    use mog_core::http_transport::{HttpTransport, StubHttpTransport};
    let s = StubHttpTransport;
    let tags: mog_core::types::Tags =
        serde_json::from_value(serde_json::json!({"app": "bottle", "mog": "a1", "op": "genie"}))
            .unwrap();
    not_implemented(
        s.request("p", "scim.me", &Default::default(), None, &tags),
        "R7",
    );
    not_implemented(s.is_online(), "R7");
}

#[test]
fn r8_mcp_transport_stub_reports_not_implemented() {
    use mog_core::mcp_transport::{McpTransport, StubMcpTransport};
    let s = StubMcpTransport;
    let tags: mog_core::types::Tags =
        serde_json::from_value(serde_json::json!({"app": "bottle", "mog": "a1", "op": "genie"}))
            .unwrap();
    not_implemented(
        s.call_tool(mog_core::mcp_transport::McpCall::new(
            "p",
            "tool",
            Default::default(),
            &tags,
        )),
        "R8",
    );
    not_implemented(s.list_tools("p", &tags), "R8");
}

#[cfg(feature = "lakebase")]
#[test]
fn r9_lakebase_pool_stub_reports_not_implemented() {
    use mog_core::lakebase_pool::{LakebasePool, StubLakebasePool};
    let s = StubLakebasePool;
    not_implemented(s.connect(), "R9");
    not_implemented(s.exec("mem.pull", serde_json::json!({})), "R9");
}

#[test]
fn r10_local_store_stub_reports_not_implemented() {
    use mog_core::local_store::{LocalStore, StubLocalStore};
    let s = StubLocalStore;
    not_implemented(s.open(), "R10");
    not_implemented(s.exec("stmt", serde_json::json!({})), "R10");
}

#[test]
fn r11_window_host_stub_reports_not_implemented() {
    use mog_core::window_host::{StubWindowHost, WindowHost};
    let s = StubWindowHost;
    not_implemented(s.open_mog_window("demo", None), "R11");
    not_implemented(s.close_mog_window("demo"), "R11");
    not_implemented(s.show_card("demo"), "R11");
    // #285: the gateway routes `window_card_hide` to R11 (ADR 0040).
    not_implemented(s.hide_card("demo"), "R11");
}

#[test]
fn r12_ipc_gateway_dispatch_reports_not_implemented() {
    use mog_core::ipc_gateway::{IpcGateway, StubIpcGateway};
    let s = StubIpcGateway;
    for (window, command) in [
        ("brain", "cli_status"),
        ("hub", "policy_get"),
        ("mog-1", "x"),
    ] {
        not_implemented(s.dispatch(window, command, serde_json::json!({})), "R12");
    }
}

#[test]
fn r13_policy_store_stub_reports_not_implemented() {
    use mog_core::policy_store::{PolicyStore, StubPolicyStore};
    let s = StubPolicyStore;
    not_implemented(s.get("OtlpTable"), "R13");
    not_implemented(s.telemetry_allowed(), "R13");
}

#[test]
fn r14_wipe_service_stub_reports_not_implemented() {
    use mog_core::wipe::{StubWipeService, WipeScope, WipeService};
    let s = StubWipeService;
    for scope in [
        WipeScope::OlderThan30Days,
        WipeScope::OlderThan90Days,
        WipeScope::Everything,
    ] {
        not_implemented(s.wipe(scope), "R14");
    }
    not_implemented(s.run_security_expiries(), "R14");
}

#[test]
fn r17_bottle_mcp_server_serves_on_loopback_and_stops() {
    // #113 replaced the R17 stub: the server starts on 127.0.0.1 for one MOG session and stops.
    use mog_core::bottle_mcp::{
        BottleMcpDeps, BottleMcpServer, UnavailableLocalTools, UnavailableRelay,
    };
    use std::sync::Arc;
    let deps = BottleMcpDeps::new(Arc::new(UnavailableLocalTools), Arc::new(UnavailableRelay));
    let s = BottleMcpServer::start("mog-1", &deps).expect("starts on loopback");
    assert!(s.url().starts_with("http://127.0.0.1:"), "{}", s.url());
    assert!(s.authorization().starts_with("Bearer "));
    s.stop();
}

#[test]
fn r17_bottle_mcp_lists_the_seven_row_tools() {
    use mog_core::bottle_mcp::TOOLS;
    assert_eq!(
        TOOLS,
        [
            "transcript.search",
            "memory.recall",
            "memory.write",
            "show_blurb",
            "set_mood",
            "propose_page",
            "ask_user"
        ]
    );
}

#[test]
fn r12_core_error_has_a_stable_code_and_names_the_component() {
    let e = CoreError::NotImplemented { component: "R12" };
    assert_eq!(e.code(), "not_implemented");
    assert!(e.to_string().contains("R12"));
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read source dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn arc_6_only_token_broker_names_the_token_type() {
    // The name comes from the type itself, so a rename keeps this test honest.
    let full = std::any::type_name::<mog_core::token_broker::SecretToken>();
    let name = full.rsplit("::").next().expect("type name");
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../..");
    let mut files = Vec::new();
    for dir in [
        "crates/mog-core/src",
        "crates/mog-core-ffi/src",
        "src-tauri/src",
    ] {
        rust_files(&root.join(dir), &mut files);
    }
    assert!(files.len() >= 17, "scan found too few files: {files:?}");
    let owner = manifest
        .join("src/token_broker.rs")
        .canonicalize()
        .expect("token_broker.rs");
    let offenders: Vec<_> = files
        .iter()
        .filter(|f| f.canonicalize().expect("canonical path") != owner)
        .filter(|f| {
            std::fs::read_to_string(f)
                .expect("read file")
                .contains(name)
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "only token_broker.rs may name {name}: {offenders:?}"
    );
}

#[test]
fn arc_6_cli_runner_run_json_refuses_auth_token() {
    use mog_core::cli_runner::{CliRunner, StubCliRunner};
    let s = StubCliRunner;
    // Only R6 TokenBroker may run `databricks auth token`; R5 must refuse it for every caller.
    for args in [
        &["auth", "token", "--profile", "p"][..],
        &["--debug", "auth", "token"][..],
        &["AUTH", "Token"][..],
        &["auth", "--profile", "p", "token"][..],
        &["auth", "-p", "p", "token"][..],
        &["auth", "--debug", "token"][..],
        &["auth", "--profile=p", "token"][..],
        &[
            "--output=json",
            "auth",
            "--profile=p",
            "token",
            "--force-refresh",
        ][..],
    ] {
        match s.run_json(args) {
            Err(e) => assert_eq!(e.code(), "token_command_refused", "args {args:?}"),
            Ok(v) => panic!("run_json returned {v:?} for {args:?}"),
        }
    }
    // Other commands still reach the stub.
    not_implemented(s.run_json(&["auth", "profiles"]), "R5");
}
