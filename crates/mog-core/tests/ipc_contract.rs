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
        // #279 (ADR 0039): a separate row, not a `capture_pause` toggle.
        ("capture_resume", &[B]),
        ("capture_status", &[B]),
        ("capture_stop_all", &[B]),
        ("capture_subscribe", &[B]),
        ("session_attest", &[B]),
        // #430 (ADR 0060): T12 in the brain writes and reads the consent records.
        ("consent_notice_record", &[B]),
        ("consent_records_list", &[B]),
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
        // #312 (ADR 0039): the wizard and Settings ask for an OS permission.
        ("permission_request", &[H]),
        // #453 (CAP-22): the app-wide settings, like the `config_*` rows.
        ("app_setting_read", &[B, H]),
        ("app_setting_write", &[B, H]),
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
        IpcError::UnknownCommand
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
        // #297, #294 (ADR 0039): the talk-key status for the pill and the tooltip.
        "hotkey.status",
        "capture.state",
        "capture.segment",
        // #425 (ADR 0055): why a lease stopped (hold cap, hands-free cap, panic).
        "capture.stopped",
        "session.stop_signal",
        "auth.status",
        "net.status",
        "mem.remote_status",
        "agent.tool_call",
        "agent.status",
        "mog.view_state",
        "card.update",
        // #285 (ADR 0040): mic frames for the MOG level meter (UX-06a).
        "mog.level",
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
            "mog.view_state" | "card.update" | "mog.level" => BrainToView,
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

#[test]
fn cap_02a_stop_and_pause_need_the_lease_id() {
    for cmd in ["capture_stop", "capture_pause"] {
        // No lease: rejected.
        let err = ipc::handle("brain", cmd, json!({"mogId": "a1"})).unwrap_err();
        assert!(
            matches!(err, IpcError::InvalidArgs { .. }),
            "{cmd}: {err:?}"
        );
        // A malformed lease: rejected.
        for bad in [json!(""), json!(17), json!("a b"), json!("x".repeat(65))] {
            let err =
                ipc::handle("brain", cmd, json!({"mogId": "a1", "leaseId": bad})).unwrap_err();
            assert!(matches!(err, IpcError::InvalidArgs { .. }), "{cmd} {bad}");
        }
        // A well-formed lease reaches R2 (still a stub).
        let err = ipc::handle("brain", cmd, json!({"mogId": "a1", "leaseId": "17"})).unwrap_err();
        assert!(
            matches!(err, IpcError::NotImplemented { .. }),
            "{cmd}: {err:?}"
        );
    }
}

#[test]
fn cap_02a_an_unknown_lease_has_a_typed_error() {
    // The same wire code as R2 CaptureError::UnknownLease (PR #273).
    let err = IpcError::UnknownLease {
        command: "capture_stop".into(),
    };
    assert_eq!(
        serde_json::to_value(&err).unwrap(),
        json!({"error": "unknown_lease", "command": "capture_stop"})
    );
    assert!(!err.to_string().is_empty());
    for (e, code) in [
        (
            IpcError::MicBusy {
                command: "capture_start".into(),
            },
            "mic_busy",
        ),
        (
            IpcError::ConsentRequired {
                command: "capture_start".into(),
            },
            "consent_required",
        ),
        (
            IpcError::InvalidMode {
                command: "capture_pause".into(),
            },
            "invalid_mode",
        ),
    ] {
        assert_eq!(serde_json::to_value(&e).unwrap()["error"], code);
    }
}

#[test]
fn cap_02a_schema_states_the_lease_rule() {
    let schemas = ipc::schemas();
    let text = schemas["capture_stop"].to_string();
    assert!(text.contains(ipc::LEASE_PATTERN), "{text}");
}

// ---------- #285 / #333 (ADR 0040): content-free errors and the R11 codes ----------

#[test]
fn sec_03_window_not_allowed_names_no_window_label() {
    let label = format!("mog-{}", "a1b2c3");
    let err = ipc::handle(&label, "cli_status", json!({})).unwrap_err();
    let wire = serde_json::to_value(&err).unwrap();
    assert_eq!(
        wire,
        json!({"error": "window_not_allowed", "command": "cli_status"})
    );
    let shown = format!("{err} {err:?} {wire}");
    assert!(!shown.contains("a1b2c3"), "the label leaked: {shown}");
}

#[test]
fn sec_03_unknown_command_echoes_no_name() {
    let err = ipc::handle("brain", SECRET, json!({})).unwrap_err();
    assert_eq!(err, IpcError::UnknownCommand);
    let wire = serde_json::to_value(&err).unwrap();
    assert_eq!(wire, json!({"error": "unknown_command"}));
    let shown = format!("{err} {err:?} {wire}");
    assert!(!shown.contains(SECRET), "the name leaked: {shown}");
}

#[test]
fn arc_3_mb_03b_r11_refusals_have_their_own_codes() {
    for (e, code) in [
        (
            IpcError::MogCapReached {
                command: "window_mog_open".into(),
            },
            "mog_cap_reached",
        ),
        (
            IpcError::UnknownMog {
                command: "window_card_show".into(),
            },
            "unknown_mog",
        ),
        (
            IpcError::WindowClosing {
                command: "window_mog_open".into(),
            },
            "window_closing",
        ),
    ] {
        assert_eq!(serde_json::to_value(&e).unwrap()["error"], code);
        assert!(!e.to_string().is_empty());
    }
}

#[test]
fn sec_03_schema_document_lists_every_error_code() {
    let doc = ipc::schema_document();
    let codes: Vec<&str> = doc["errors"]
        .as_array()
        .expect("errors list")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(codes, mog_core::ipc_gateway::ERROR_CODES);
}

// ---------- #297, #294, #312, #279 (ADR 0039) ----------

#[test]
fn arc_5_cap_39c_hotkey_status_is_the_only_sticky_core_to_brain_event() {
    let e = events::ALL
        .iter()
        .find(|e| e.spec_name == "hotkey.status")
        .expect("hotkey.status is in the event list");
    assert_eq!(e.name, events::HOTKEY_STATUS);
    assert_eq!(e.name, "hotkey:status");
    assert_eq!(e.direction, events::Direction::CoreToBrain);
    assert_eq!(events::STICKY, [events::HOTKEY_STATUS]);
    assert!(events::is_sticky(events::HOTKEY_STATUS));
    assert!(!events::is_sticky(events::HOTKEY_PTT));
    assert!(!events::is_sticky(events::VIEW_INPUT));
}

#[test]
fn cap_02a_capture_resume_is_brain_only_and_needs_a_valid_lease_id() {
    let row = ipc::command("capture_resume").expect("a capture_resume row");
    assert_eq!(row.windows, [WindowClass::Brain]);
    assert_eq!(row.phase, Phase::MvpD);
    let good = json!({"mogId": "a1", "leaseId": "17"});
    for label in ["hub", "mog-a1", "card-a1"] {
        assert!(
            matches!(
                ipc::handle(label, "capture_resume", good.clone()).unwrap_err(),
                IpcError::WindowNotAllowed { .. }
            ),
            "{label}"
        );
    }
    for bad in [
        json!({"mogId": "a1"}),
        json!({"mogId": "a1", "leaseId": ""}),
        json!({"mogId": "a1", "leaseId": "a b"}),
        json!({"mogId": "a1", "leaseId": "x".repeat(65)}),
        json!({"mogId": "a1", "leaseId": "17", "resume": true}),
    ] {
        assert!(
            matches!(
                ipc::handle("brain", "capture_resume", bad.clone()).unwrap_err(),
                IpcError::InvalidArgs { .. }
            ),
            "{bad}"
        );
    }
    // Until the capture IPC wiring of #4, a valid call answers not_implemented.
    assert!(matches!(
        ipc::handle("brain", "capture_resume", good).unwrap_err(),
        IpcError::NotImplemented { .. }
    ));
    // The same argument schema as capture_pause.
    let schemas = ipc::schemas();
    assert_eq!(schemas["capture_resume"], schemas["capture_pause"]);
}

#[test]
fn cap_36_cap_37_permission_request_is_hub_only_and_takes_one_known_permission() {
    let row = ipc::command("permission_request").expect("a permission_request row");
    assert_eq!(row.windows, [WindowClass::Hub]);
    assert_eq!(row.phase, Phase::MvpD);
    for p in ["input_monitoring", "accessibility"] {
        let args = json!({ "permission": p });
        // The gateway checks pass; the host makes the user action and asks the OS.
        assert!(matches!(
            ipc::handle("hub", "permission_request", args.clone()).unwrap_err(),
            IpcError::NotImplemented { .. }
        ));
        for label in ["brain", "mog-a1", "card-a1"] {
            assert!(
                matches!(
                    ipc::handle(label, "permission_request", args.clone()).unwrap_err(),
                    IpcError::WindowNotAllowed { .. }
                ),
                "{label} {p}"
            );
        }
    }
    for bad in [
        json!({}),
        json!({"permission": "camera"}),
        json!({"permission": "inputMonitoring"}),
        json!({"permission": "accessibility", "prompt": false}),
    ] {
        assert!(
            matches!(
                ipc::handle("hub", "permission_request", bad.clone()).unwrap_err(),
                IpcError::InvalidArgs { .. }
            ),
            "{bad}"
        );
    }
}

// ---------- #430 (ADR 0060) ----------

#[test]
fn sec_32a_at_24_consent_notice_record_is_brain_only_and_takes_only_the_notice_version() {
    let row = ipc::command("consent_notice_record").expect("a consent_notice_record row");
    assert_eq!(row.windows, [WindowClass::Brain]);
    assert_eq!(row.phase, Phase::MvpD);
    let good = json!({"noticeVersion": 3});
    for label in ["hub", "mog-a1", "card-a1"] {
        assert!(
            matches!(
                ipc::handle(label, "consent_notice_record", good.clone()).unwrap_err(),
                IpcError::WindowNotAllowed { .. }
            ),
            "{label}"
        );
    }
    // SEC-32a: the Rust side fills the user, the device and the time. A caller cannot.
    for bad in [
        json!({}),
        json!({"noticeVersion": -1}),
        json!({"noticeVersion": "3"}),
        json!({"noticeVersion": 3, "user": "fake.user@example.invalid"}),
        json!({"noticeVersion": 3, "device": "d0d0d0d0-0000-4000-8000-00000000fa4e"}),
        json!({"noticeVersion": 3, "acceptedAt": 1}),
        json!({"noticeVersion": 3, "mogId": "a1"}),
    ] {
        assert!(
            matches!(
                ipc::handle("brain", "consent_notice_record", bad.clone()).unwrap_err(),
                IpcError::InvalidArgs { .. }
            ),
            "{bad}"
        );
    }
    // The gateway checks pass; the host routes it to the consent component.
    assert!(matches!(
        ipc::handle("brain", "consent_notice_record", good).unwrap_err(),
        IpcError::NotImplemented { .. }
    ));
    let schema = &ipc::schemas()["consent_notice_record"];
    assert_eq!(schema["required"], json!(["noticeVersion"]));
    assert_eq!(schema["additionalProperties"], json!(false));
}

#[test]
fn sec_32a_consent_records_list_is_brain_only_and_takes_no_args() {
    let row = ipc::command("consent_records_list").expect("a consent_records_list row");
    assert_eq!(row.windows, [WindowClass::Brain]);
    assert_eq!(row.phase, Phase::MvpD);
    for label in ["hub", "mog-a1", "card-a1"] {
        assert!(
            matches!(
                ipc::handle(label, "consent_records_list", json!({})).unwrap_err(),
                IpcError::WindowNotAllowed { .. }
            ),
            "{label}"
        );
    }
    assert!(matches!(
        ipc::handle("brain", "consent_records_list", json!({"mogId": "a1"})).unwrap_err(),
        IpcError::InvalidArgs { .. }
    ));
    assert!(matches!(
        ipc::handle("brain", "consent_records_list", json!({})).unwrap_err(),
        IpcError::NotImplemented { .. }
    ));
    let schemas = ipc::schemas();
    assert_eq!(schemas["consent_records_list"], schemas["cli_status"]);
}

#[test]
fn sec_03_consent_storage_failed_is_a_fixed_code() {
    assert!(mog_core::ipc_gateway::ERROR_CODES.contains(&"consent_storage_failed"));
}

// ---------- #453 (CAP-22) ----------

#[test]
fn cap_22_app_setting_rows_are_brain_and_hub_rows_and_take_a_known_key() {
    for name in ["app_setting_read", "app_setting_write"] {
        let row = ipc::command(name).expect("an app_setting row");
        assert_eq!(
            row.windows,
            [WindowClass::Brain, WindowClass::Hub],
            "{name}"
        );
        assert_eq!(row.phase, Phase::MvpD, "{name}");
    }
    let read = json!({ "key": "app.capture.preferBuiltInMic" });
    let write = json!({ "key": "app.capture.preferBuiltInMic", "value": true });
    for (name, args) in [("app_setting_read", &read), ("app_setting_write", &write)] {
        for label in ["brain", "hub"] {
            assert!(matches!(
                ipc::handle(label, name, args.clone()).unwrap_err(),
                IpcError::NotImplemented { .. }
            ));
        }
        for label in ["mog-a1", "card-a1"] {
            assert!(
                matches!(
                    ipc::handle(label, name, args.clone()).unwrap_err(),
                    IpcError::WindowNotAllowed { .. }
                ),
                "{label} {name}"
            );
        }
    }
    for (name, bad) in [
        ("app_setting_read", json!({})),
        ("app_setting_read", json!({"key": "app.capture.other"})),
        (
            "app_setting_read",
            json!({"key": "app.capture.preferBuiltInMic", "value": true}),
        ),
        (
            "app_setting_write",
            json!({"key": "app.capture.preferBuiltInMic"}),
        ),
        (
            "app_setting_write",
            json!({"key": "app.capture.preferBuiltInMic", "value": "on"}),
        ),
        (
            "app_setting_write",
            json!({"key": "app.capture.other", "value": true}),
        ),
        (
            "app_setting_write",
            json!({"key": "app.capture.preferBuiltInMic", "value": true, "device": "AirPods"}),
        ),
    ] {
        assert!(
            matches!(
                ipc::handle("hub", name, bad.clone()).unwrap_err(),
                IpcError::InvalidArgs { .. }
            ),
            "{name} {bad}"
        );
    }
}
