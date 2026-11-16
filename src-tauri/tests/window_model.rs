//! #8: the window model. MOG-08a (view capabilities), ARC-3 (R11 WindowHost lifecycle and
//! the cap of 8 MOGs) and ARC-4 (each window label gets exactly one capability).

use bottle_lib::window_host::{OpenOutcome, WindowError, WindowLedger};
use bottle_lib::windows::{WindowKind, MAX_ACTIVE_MOGS};
use mog_core::ipc::{WindowClass, COMMANDS};
use serde_json::Value;
use std::path::PathBuf;

// ---------- MB-10b / MB-10c (#71): the native window titles ----------

#[test]
fn mb_10b_hub_title_bar_shows_labs_label() {
    assert_eq!(
        WindowKind::Hub.spec().title,
        "Bottle (Databricks Labs / experimental)"
    );
}

#[test]
fn mb_10c_window_titles_use_mog_only_as_a_noun() {
    let kinds = [
        WindowKind::Brain,
        WindowKind::Hub,
        WindowKind::mog("a").unwrap(),
        WindowKind::card("a").unwrap(),
    ];
    for kind in kinds {
        let title = kind.spec().title;
        for word in title.split(|c: char| !c.is_alphanumeric()) {
            if word.eq_ignore_ascii_case("mog") || word.eq_ignore_ascii_case("mogs") {
                assert!(word.starts_with("MOG"), "{title}: write MOG, not {word}");
            }
        }
    }
}

fn capabilities() -> Vec<Value> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    let mut caps: Vec<Value> = std::fs::read_dir(dir)
        .expect("capabilities dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap())
        .collect();
    caps.sort_by_key(|c| c["identifier"].as_str().unwrap_or_default().to_owned());
    caps
}

fn capability(id: &str) -> Value {
    capabilities()
        .into_iter()
        .find(|c| c["identifier"] == id)
        .unwrap_or_else(|| panic!("capability {id}"))
}

/// The app-command permissions of a capability (no `core:` entries).
fn app_commands(cap: &Value) -> Vec<String> {
    cap["permissions"]
        .as_array()
        .expect("permissions")
        .iter()
        .map(|p| p.as_str().expect("string permission").to_owned())
        .filter(|p| !p.starts_with("core:"))
        .collect()
}

fn allow(command: &str) -> String {
    format!("allow-{}", command.replace('_', "-"))
}

// ---------- MOG-08a / AT-17: view capabilities ----------

#[test]
fn mog_08a_view_caps_only_view_commands() {
    for id in ["mog", "card"] {
        let cmds = app_commands(&capability(id));
        assert!(!cmds.is_empty(), "{id} needs view_emit");
        for p in &cmds {
            assert!(
                p.starts_with("allow-view-"),
                "{id} capability holds a non-view command: {p}"
            );
        }
    }
}

#[test]
fn mog_08a_hub_holds_only_view_commands_and_the_4_5_hub_rows() {
    // 4.5 gives the hub (V1) the profile, config, policy and hotkey rows (ADR 0008).
    let allowed: Vec<String> = COMMANDS
        .iter()
        .filter(|c| c.windows.contains(&WindowClass::Hub))
        .map(|c| allow(c.name))
        .collect();
    for p in app_commands(&capability("hub")) {
        assert!(
            p.starts_with("allow-view-") || allowed.contains(&p),
            "hub holds {p}"
        );
    }
    for brain_only in [
        "mcp_call",
        "dbx_http",
        "capture_start",
        "insert_text",
        "model_invoke",
    ] {
        assert!(!app_commands(&capability("hub")).contains(&allow(brain_only)));
    }
}

// ---------- ARC-4: each label made at run time matches exactly one capability ----------

fn matching_capabilities(label: &str) -> Vec<String> {
    capabilities()
        .iter()
        .filter(|c| {
            c["windows"].as_array().unwrap().iter().any(|w| {
                glob::Pattern::new(w.as_str().unwrap())
                    .expect("glob")
                    .matches(label)
            })
        })
        .map(|c| c["identifier"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn arc_4_every_window_label_matches_exactly_one_capability() {
    let id = "7b0c1f3e-0000-4000-8000-000000000001";
    for (kind, cap) in [
        (WindowKind::Brain, "brain"),
        (WindowKind::Hub, "hub"),
        (WindowKind::mog(id).unwrap(), "mog"),
        (WindowKind::card(id).unwrap(), "card"),
    ] {
        assert_eq!(matching_capabilities(&kind.label()), [cap], "{kind:?}");
    }
    // A label that no kind makes gets no capability, so no command at all.
    for stray in ["main", "brain-2", "hubx", "settings"] {
        assert!(matching_capabilities(stray).is_empty(), "{stray}");
    }
}

// ---------- ARC-3 / R11: window lifecycle in the WindowHost ----------

fn id(n: usize) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}

#[test]
fn arc_3_window_host_opens_mog_windows_up_to_the_cap() {
    let mut ledger = WindowLedger::default();
    for n in 0..MAX_ACTIVE_MOGS {
        assert_eq!(
            ledger.open_mog(&id(n)),
            Ok(OpenOutcome::Opened(WindowKind::Mog(id(n))))
        );
    }
    assert_eq!(ledger.active_mogs(), 8);
    assert_eq!(
        ledger.open_mog(&id(99)),
        Err(WindowError::CapReached { cap: 8 })
    );
    assert_eq!(ledger.active_mogs(), 8);
}

#[test]
fn arc_3_window_host_reopen_is_idempotent_and_close_frees_a_slot() {
    let mut ledger = WindowLedger::new(2);
    assert!(ledger.open_mog("a").is_ok());
    assert_eq!(ledger.open_mog("a"), Ok(OpenOutcome::AlreadyOpen));
    assert!(ledger.open_mog("b").is_ok());
    assert_eq!(
        ledger.open_mog("c"),
        Err(WindowError::CapReached { cap: 2 })
    );
    assert!(ledger.close_mog("a"));
    assert!(!ledger.close_mog("a"), "a second close is a no-op");
    assert!(ledger.open_mog("c").is_ok());
}

#[test]
fn arc_3_window_host_rejects_ids_that_are_not_label_safe() {
    let mut ledger = WindowLedger::default();
    for bad in ["", "a/b", "a b", "x:y", &"x".repeat(65)] {
        assert_eq!(ledger.open_mog(bad), Err(WindowError::InvalidId), "{bad:?}");
        assert_eq!(
            ledger.show_card(bad),
            Err(WindowError::InvalidId),
            "{bad:?}"
        );
    }
    assert_eq!(ledger.active_mogs(), 0);
}

#[test]
fn arc_3_card_opens_only_for_an_open_mog_and_closes_with_it() {
    let mut ledger = WindowLedger::default();
    assert_eq!(ledger.show_card("m1"), Err(WindowError::UnknownMog));
    ledger.open_mog("m1").unwrap();
    assert_eq!(ledger.show_card("m1"), Ok(WindowKind::Card("m1".into())));
    assert!(ledger.has_card("m1"));
    ledger.close_mog("m1");
    assert!(!ledger.has_card("m1"), "closing a MOG drops its card");
}

#[test]
fn arc_3_window_errors_name_no_id() {
    // Errors cross IPC as fixed codes (ADR 0040); they never echo the caller's argument.
    let bad = "dapi0123456789abcdef/secret";
    let mut ledger = WindowLedger::new(1);
    let mut errors = vec![
        ledger.open_mog(bad).unwrap_err(),
        ledger.show_card(bad).unwrap_err(),
        ledger.show_card("zz-unknown-mog-zz").unwrap_err(),
    ];
    ledger.open_mog("m1").unwrap();
    errors.push(ledger.open_mog("zz-second-mog-zz").unwrap_err());
    for e in errors {
        let text = format!("{e} {e:?} {}", e.code());
        for leaked in ["dapi0123", "secret", "zz-unknown", "zz-second", "m1"] {
            assert!(!text.contains(leaked), "{text}");
        }
    }
}

#[test]
fn arc_3_ledger_follows_windows_destroyed_outside_the_host() {
    let mut ledger = WindowLedger::new(1);
    ledger.open_mog("m1").unwrap();
    ledger.show_card("m1").unwrap();
    ledger.window_destroyed("card-m1");
    assert!(!ledger.has_card("m1"), "a destroyed card is no longer open");
    assert_eq!(ledger.active_mogs(), 1);
    ledger.window_destroyed("mog-m1");
    assert_eq!(ledger.active_mogs(), 0, "a destroyed MOG frees its slot");
    for other in ["brain", "hub", "mog-", "nope"] {
        ledger.window_destroyed(other);
    }
    assert!(ledger.open_mog("m2").is_ok());
}

// ---------- ARC-3 / R11: TauriWindowHost on the Tauri mock runtime ----------

mod host {
    use bottle_lib::window_host::{TauriWindowHost, WindowError};
    use tauri::test::{mock_app, MockRuntime};
    use tauri::{App, Manager};

    fn setup() -> (App<MockRuntime>, TauriWindowHost<MockRuntime>) {
        let app = mock_app();
        let host = TauriWindowHost::new(app.handle().clone());
        (app, host)
    }

    fn exists(app: &App<MockRuntime>, label: &str) -> bool {
        app.get_webview_window(label).is_some()
    }

    #[test]
    fn arc_3_host_makes_and_closes_mog_and_card_windows() {
        let (app, host) = setup();
        host.open_mog_window("m1").unwrap();
        assert!(exists(&app, "mog-m1"));
        assert!(
            !exists(&app, "card-m1"),
            "the card is made on the first show"
        );
        host.show_card("m1").unwrap();
        assert!(exists(&app, "card-m1"));
        host.hide_card("m1").unwrap();
        assert!(
            exists(&app, "card-m1"),
            "hide keeps the card for the reaper (#78)"
        );
        host.close_mog_window("m1").unwrap();
        // The mock runtime drops the manager entry only when its event loop runs, so the
        // test checks the ledger: the slot is free and a second close is refused.
        assert_eq!(host.active_mogs(), 0);
        assert_eq!(host.close_mog_window("m1"), Err(WindowError::UnknownMog));
    }

    #[test]
    fn arc_3_host_enforces_the_cap_of_8() {
        let (app, host) = setup();
        for n in 0..8 {
            host.open_mog_window(&format!("m{n}")).unwrap();
        }
        assert_eq!(
            host.open_mog_window("m9"),
            Err(WindowError::CapReached { cap: 8 })
        );
        assert!(!exists(&app, "mog-m9"));
    }

    #[test]
    fn arc_3_host_reopen_is_idempotent() {
        let (app, host) = setup();
        host.open_mog_window("m1").unwrap();
        host.open_mog_window("m1").unwrap();
        assert!(exists(&app, "mog-m1"));
        assert_eq!(host.active_mogs(), 1, "a second open takes no second slot");
    }

    #[test]
    fn arc_3_host_frees_the_slot_of_a_destroyed_window() {
        let (app, host) = setup();
        for n in 0..8 {
            host.open_mog_window(&format!("m{n}")).unwrap();
        }
        app.get_webview_window("mog-m0").unwrap().destroy().unwrap();
        // lib.rs calls this from `WindowEvent::Destroyed`.
        host.window_destroyed("mog-m0");
        assert_eq!(host.active_mogs(), 7);
        host.open_mog_window("m8").unwrap();
        assert!(exists(&app, "mog-m8"));
    }

    #[test]
    fn arc_3_host_rejects_bad_ids_and_unknown_mogs() {
        let (_app, host) = setup();
        assert_eq!(host.open_mog_window("a/b"), Err(WindowError::InvalidId));
        assert_eq!(host.show_card("m1"), Err(WindowError::UnknownMog));
        assert_eq!(host.hide_card("m1"), Err(WindowError::UnknownMog));
    }
}

#[test]
fn arc_3_app_feeds_destroyed_windows_to_the_host() {
    let lib = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
        .expect("lib.rs");
    assert!(lib.contains("on_window_event"), "no window event handler");
    assert!(lib.contains("WindowEvent::Destroyed") && lib.contains("host.window_destroyed("));
    assert!(lib.contains("app.manage(TauriWindowHost::new("));
}
