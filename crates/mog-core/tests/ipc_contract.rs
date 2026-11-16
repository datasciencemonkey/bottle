//! IPC contract tests for #223: the command table of 4.5 (ARC-4, ARC-5, ARC-6).

use mog_core::ipc::{self, events, IpcError, Phase, WindowClass, COMMANDS};
use serde_json::json;

/// Every command of the 4.5 table, with the slash pairs split, plus `brain_subscribe`.
const SPEC_COMMANDS: &[(&str, &[WindowClass])] = {
    use WindowClass::{Brain as B, Card as C, Hub as H, Mog as M};
    &[
        ("cli_status", &[B, H]),
        ("cli_install", &[B, H]),
        ("profiles_list", &[B, H]),
        ("profile_status", &[B, H]),
        ("profile_login", &[B, H]),
        ("profile_select", &[B, H]),
        ("capture_start", &[B]),
        ("capture_stop", &[B]),
        ("capture_pause", &[B]),
        ("capture_status", &[B]),
        ("capture_stop_all", &[B]),
        ("capture_subscribe", &[B]),
        ("session_attest", &[B]),
        ("stt_transcribe", &[B]),
        ("insert_text", &[B]),
        ("agent_start", &[B]),
        ("agent_stop", &[B]),
        ("agent_send", &[B]),
        ("agent_tool_result", &[B]),
        ("model_invoke", &[B]),
        ("mcp_call", &[B]),
        ("dbx_http", &[B]),
        ("mem_local_exec", &[B]),
        ("mem_remote_exec", &[B]),
        ("mem_remote_status", &[B, H]),
        ("config_list", &[B, H]),
        ("config_read", &[B, H]),
        ("config_write", &[B, H]),
        ("config_delete", &[B, H]),
        ("policy_get", &[B, H]),
        ("hotkeys_set", &[B, H]),
        ("mog_set_active", &[B]),
        ("window_mog_open", &[B]),
        ("window_mog_close", &[B]),
        ("window_card_show", &[B]),
        ("window_card_hide", &[B]),
        ("view_emit", &[H, M, C]),
        ("audit_append", &[B]),
        ("telemetry_span", &[B]),
        // Not in 4.5: the brain's private Channel (ADR 0003).
        ("brain_subscribe", &[B]),
    ]
};

#[test]
fn arc_4_every_4_5_command_has_a_row_and_window_list() {
    let names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
    let spec: Vec<&str> = SPEC_COMMANDS.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, spec, "command table must equal 4.5 in order");
    for (name, windows) in SPEC_COMMANDS {
        let row = ipc::command(name).expect("row");
        assert_eq!(row.windows, *windows, "windows of {name}");
    }
}

#[test]
fn arc_4_every_command_has_a_json_schema() {
    let schemas = ipc::schemas();
    for row in COMMANDS {
        let schema = schemas
            .get(row.name)
            .unwrap_or_else(|| panic!("no schema for {}", row.name));
        assert_eq!(
            schema["additionalProperties"],
            json!(false),
            "{} must deny unknown fields",
            row.name
        );
    }
}

#[test]
fn arc_4_v1_commands_are_marked_v1() {
    for name in ["mem_remote_exec", "mem_remote_status"] {
        assert_eq!(ipc::command(name).unwrap().phase, Phase::V1);
    }
    assert_eq!(ipc::command("cli_status").unwrap().phase, Phase::MvpD);
}

#[test]
fn arc_4_wrong_window_label_is_rejected() {
    // A view cannot call a brain command.
    for label in ["hub", "mog-a1", "card-a1"] {
        let err = ipc::handle(label, "mcp_call", json!({})).unwrap_err();
        assert!(
            matches!(err, IpcError::WindowNotAllowed { .. }),
            "{label}: {err:?}"
        );
    }
    // The brain cannot fake view input.
    let err = ipc::handle(
        "brain",
        "view_emit",
        json!({"mogId": "a1", "kind": "click"}),
    )
    .unwrap_err();
    assert!(matches!(err, IpcError::WindowNotAllowed { .. }));
    // Unknown labels and unknown commands are rejected.
    assert!(matches!(
        ipc::handle("main", "cli_status", json!({})).unwrap_err(),
        IpcError::WindowNotAllowed { .. }
    ));
    assert!(matches!(
        ipc::handle("brain", "shell_exec", json!({})).unwrap_err(),
        IpcError::UnknownCommand { .. }
    ));
}

#[test]
fn arc_4_unknown_fields_are_rejected() {
    let err = ipc::handle(
        "brain",
        "profile_status",
        json!({"profile": "bottle-dev", "token": "x"}),
    )
    .unwrap_err();
    assert!(matches!(err, IpcError::InvalidArgs { .. }), "{err:?}");
    let err = ipc::handle("brain", "cli_status", json!({"extra": 1})).unwrap_err();
    assert!(matches!(err, IpcError::InvalidArgs { .. }), "{err:?}");
}

#[test]
fn arc_4_valid_calls_reach_the_stub() {
    let err = ipc::handle("brain", "profile_status", json!({"profile": "bottle-dev"})).unwrap_err();
    assert!(
        matches!(&err, IpcError::NotImplemented { command } if command == "profile_status"),
        "{err:?}"
    );
    let err = ipc::handle("hub", "config_list", json!({})).unwrap_err();
    assert!(matches!(err, IpcError::NotImplemented { .. }));
}

#[test]
fn arc_4_cli_install_needs_consent_true() {
    let err = ipc::handle("hub", "cli_install", json!({"consent": false})).unwrap_err();
    assert!(matches!(err, IpcError::InvalidArgs { .. }), "{err:?}");
    let err = ipc::handle("hub", "cli_install", json!({"consent": true})).unwrap_err();
    assert!(matches!(err, IpcError::NotImplemented { .. }));
}

#[test]
fn arc_4_view_emit_mog_id_must_match_the_sending_window() {
    let ok = ipc::handle(
        "mog-a1",
        "view_emit",
        json!({"mogId": "a1", "kind": "poked"}),
    )
    .expect("own MOG id is accepted");
    assert_eq!(
        ok,
        ipc::Handled::RelayToBrain {
            from: "mog-a1".into()
        }
    );
    let err = ipc::handle(
        "mog-a1",
        "view_emit",
        json!({"mogId": "b2", "kind": "poked"}),
    )
    .unwrap_err();
    assert!(matches!(err, IpcError::InvalidArgs { .. }), "{err:?}");
    let err = ipc::handle(
        "card-a1",
        "view_emit",
        json!({"mogId": "a1", "kind": "wave"}),
    )
    .unwrap_err();
    assert!(
        matches!(err, IpcError::InvalidArgs { .. }),
        "unknown kind: {err:?}"
    );
    // The hub may emit for any MOG.
    assert!(ipc::handle("hub", "view_emit", json!({"mogId": "b2", "kind": "click"})).is_ok());
}

#[test]
fn arc_4_dbx_http_takes_route_ids_only() {
    let base = json!({"profile": "p", "params": {}, "tags": {"app": "bottle", "mog": "a1", "op": "genie"}});
    let mut ok = base.clone();
    ok["routeId"] = json!("genie.get_message");
    assert!(matches!(
        ipc::handle("brain", "dbx_http", ok).unwrap_err(),
        IpcError::NotImplemented { .. }
    ));
    let mut bad = base;
    bad["routeId"] = json!("https://example.com/api");
    assert!(matches!(
        ipc::handle("brain", "dbx_http", bad).unwrap_err(),
        IpcError::InvalidArgs { .. }
    ));
}

#[test]
fn arc_5_capture_subscribe_takes_two_channels() {
    let schemas = ipc::schemas();
    let s = &schemas["capture_subscribe"];
    let required: Vec<&str> = s["required"]
        .as_array()
        .expect("required")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(required, ["onLevel", "onPartial"]);
}

#[test]
fn arc_5_event_names_are_valid_tauri_names() {
    let valid = |n: &str| {
        !n.is_empty()
            && n.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '/' | ':' | '_'))
    };
    let spec_names = [
        "hotkey.ptt",
        "hotkey.panic",
        "capture.state",
        "capture.segment",
        "session.stop_signal",
        "auth.status",
        "net.status",
        "mem.remote_status",
        "agent.tool_call",
        "agent.status",
        "mog.view_state",
        "card.update",
        "view.input",
    ];
    let names: Vec<&str> = events::ALL.iter().map(|e| e.name).collect();
    let specs: Vec<&str> = events::ALL.iter().map(|e| e.spec_name).collect();
    assert_eq!(specs, spec_names);
    for e in events::ALL {
        assert!(valid(e.name), "{} is not a valid Tauri event name", e.name);
        assert_eq!(e.name, e.spec_name.replace('.', ":"));
    }
    assert_eq!(names.len(), spec_names.len());
}

#[test]
fn arc_5_core_events_go_to_the_brain_and_view_events_come_from_it() {
    use events::Direction::*;
    for e in events::ALL {
        let expected = match e.spec_name {
            "mog.view_state" | "card.update" => BrainToView,
            _ => CoreToBrain,
        };
        assert_eq!(e.direction, expected, "{}", e.spec_name);
    }
}

#[test]
fn arc_6_no_command_takes_a_url_token_or_header() {
    let schemas = ipc::schemas();
    for (name, schema) in &schemas {
        let text = schema.to_string().to_lowercase();
        for banned in [
            "\"token\"",
            "\"authorization\"",
            "\"headers\"",
            "\"url\"",
            "\"host\"",
        ] {
            assert!(!text.contains(banned), "{name} has a {banned} field");
        }
        // 4.5 profile_login takes the workspace URL that `databricks auth login --host` needs.
        // It is the only URL-like field (4.6 rule 1 covers request targets).
        if *name != "profile_login" {
            assert!(!text.contains("workspaceurl"), "{name} takes a URL");
        }
    }
}

const SECRET: &str = concat!("dap", "i0123456789abcdef0123456789abcdef");

#[test]
fn arc_6_errors_never_echo_argument_values() {
    let tags = json!({"app": "bottle", "mog": "a1", "op": "genie"});
    let cases = [
        (
            "dbx_http",
            json!({"profile": "p", "routeId": format!("https://h/x?t={SECRET}"), "params": {}, "tags": tags}),
        ),
        (
            "session_attest",
            json!({"mogId": "a1", "kind": "only_me", "noticeVersion": SECRET}),
        ),
        ("profile_status", json!({"profile": "p", SECRET: 1})),
        ("capture_start", json!({"mogId": "a1", "mode": SECRET})),
        (
            "capture_subscribe",
            json!({"onLevel": SECRET, "onPartial": "__CHANNEL__:2"}),
        ),
    ];
    for (cmd, args) in cases {
        let err = ipc::handle("brain", cmd, args).unwrap_err();
        assert!(
            matches!(err, IpcError::InvalidArgs { .. }),
            "{cmd}: {err:?}"
        );
        let shown = format!("{err} {err:?} {}", serde_json::to_string(&err).unwrap());
        assert!(!shown.contains(SECRET), "{cmd} echoed the value: {shown}");
        assert!(!shown.contains("https://"), "{cmd} echoed the URL: {shown}");
    }
}

#[test]
fn arc_5_channel_arguments_must_be_channel_references() {
    let ok = ipc::handle(
        "brain",
        "capture_subscribe",
        json!({"onLevel": "__CHANNEL__:1", "onPartial": "__CHANNEL__:2"}),
    )
    .unwrap_err();
    assert!(matches!(ok, IpcError::NotImplemented { .. }), "{ok:?}");
    for bad in ["1", "__CHANNEL__:", "__CHANNEL__:x", "channel"] {
        let err = ipc::handle(
            "brain",
            "capture_subscribe",
            json!({"onLevel": bad, "onPartial": "__CHANNEL__:2"}),
        )
        .unwrap_err();
        assert!(matches!(err, IpcError::InvalidArgs { .. }), "{bad}");
    }
}

#[test]
fn arc_5_only_the_brain_can_take_the_core_to_brain_channel() {
    let args = json!({"onEvent": "__CHANNEL__:7"});
    assert!(matches!(
        ipc::handle("brain", "brain_subscribe", args.clone()).unwrap(),
        ipc::Handled::SubscribeBrain { .. }
    ));
    for label in ["hub", "mog-a1", "card-a1"] {
        assert!(matches!(
            ipc::handle(label, "brain_subscribe", args.clone()).unwrap_err(),
            IpcError::WindowNotAllowed { .. }
        ));
    }
}
