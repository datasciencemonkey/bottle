//! AT-01 "Create two MOGs" (#230), section 15, and the window part of AT-20 (UX-19a).
//!
//! The AT script: create two MOGs with different chords, home profiles and colours; restart;
//! drag both; remove a display. Pass: both configs survive, chords stay distinct, no secret is
//! in either config, no focus steal, both MOGs stay visible.
//!
//! What ships and runs here: the R11 window model (#8, #11, #78): two MOG windows with their own
//! labels, each a transparent never-key panel; the card is the only key window; a drag of
//! either MOG never reaches the page as a click; the 8-MOG cap. The chord check uses the R1
//! conflict check of #24 (MB-14, core part). The AppKit read-back of a real panel is
//! `tests/macos_panel_appkit.rs` (it runs on every macOS `cargo test`).
//!
//! Pending, each `#[ignore]` with its feature issue (they never count as a pass): MOG
//! definitions and restart (#26), per-display positions and display removal (#82),
//! click-through and the topmost watchdog (#81), the Windows window styles (#80), the hub
//! conflict UI (#77). The on-screen run is a person's step: docs/tests/AT-MOGS-TALKKEY-A11Y.md.

use bottle_lib::mog_drag::{DragGesture, MouseInput, Verdict, DRAG_THRESHOLD_PT};
use bottle_lib::window_host::{OpenOutcome, WindowError, WindowLedger};
use bottle_lib::windows::{PanelLevel, WindowKind, MAX_ACTIVE_MOGS};
use mog_core::hotkey_bindings::{
    check_conflicts, default_bindings, hotkeys_set, Binding, ConflictKind, HotkeyAction,
    KeyboardEnv, Os,
};
use mog_core::ipc::JsonObject;
use serde_json::{json, Value};

const MOG_A: &str = "7b0c1f3e-0000-4000-8000-00000000000a";
const MOG_B: &str = "7b0c1f3e-0000-4000-8000-00000000000b";

fn mog_ids(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| format!("7b0c1f3e-0000-4000-8000-{i:012}"))
        .collect()
}

/// One press-move-release on a MOG window. Returns the verdicts of the events.
fn drag(dx: f64, dy: f64) -> Vec<Verdict> {
    let mut g = DragGesture::default();
    let mut out = vec![g.on(MouseInput::Down { x: 100.0, y: 100.0 })];
    for step in 1..=4 {
        let f = f64::from(step) / 4.0;
        out.push(g.on(MouseInput::Dragged {
            x: 100.0 + dx * f,
            y: 100.0 + dy * f,
        }));
    }
    out.push(g.on(MouseInput::Up));
    out
}

// ---------- Two MOGs: two windows, no focus steal (MOG-01a, MOG-01b, MOG-02) ----------

#[test]
fn at_01_mog_01a_mog_01b_mog_02_two_mogs_open_as_two_transparent_never_key_panels() {
    let mut ledger = WindowLedger::default();
    let mut labels = Vec::new();
    for id in [MOG_A, MOG_B] {
        let OpenOutcome::Opened(kind) = ledger.open_mog(id).expect("open") else {
            panic!("{id} was open already");
        };
        let spec = kind.spec();
        assert!(spec.transparent && !spec.decorations && spec.always_on_top, "{spec:?}");
        assert!(!spec.focusable, "MOG-01b: a MOG never takes focus");
        let panel = kind.panel().expect("a MOG is an NSPanel on macOS");
        assert_eq!(panel.level, PanelLevel::Status);
        assert!(!panel.can_become_key && !panel.can_become_main && panel.nonactivating);
        assert!(panel.join_all_spaces && panel.stationary && panel.full_screen_auxiliary);
        assert!(panel.transparent);
        labels.push(spec.label);
    }
    assert_eq!(labels, vec![format!("mog-{MOG_A}"), format!("mog-{MOG_B}")]);
    assert_eq!(ledger.active_mogs(), 2);
    // Opening a MOG again is a no-op, so a restart that replays the list makes no third window.
    assert_eq!(ledger.open_mog(MOG_A), Ok(OpenOutcome::AlreadyOpen));
    assert_eq!(ledger.active_mogs(), 2);
}

#[test]
fn at_01_mog_03a_at_20_ux_19a_the_card_is_the_only_key_window() {
    let mut ledger = WindowLedger::default();
    ledger.open_mog(MOG_A).expect("open");
    let card = ledger.show_card(MOG_A).expect("card");
    let panel = card.panel().expect("the card is a panel on macOS");
    assert!(panel.can_become_key, "MOG-03a: typing and the IME work in the card");
    assert!(panel.nonactivating, "MOG-03b: the frontmost app does not change");
    assert_eq!(panel.level, PanelLevel::Floating);

    // UX-19a: of the windows that float over other apps, only the card takes the keyboard.
    let floating = [
        WindowKind::Brain,
        WindowKind::mog(MOG_A).expect("id"),
        WindowKind::mog(MOG_B).expect("id"),
        card.clone(),
    ];
    let key_capable: Vec<_> = floating
        .iter()
        .filter(|k| k.spec().focusable || k.panel().is_some_and(|p| p.can_become_key))
        .map(|k| k.label())
        .collect();
    assert_eq!(key_capable, vec![card.label()]);
}

// ---------- Drag both (MOG-01c, UX-27a, UX-27b) ----------

#[test]
fn at_01_mog_01c_ux_27a_ux_27b_dragging_each_mog_starts_a_native_drag_and_no_click() {
    for _mog in [MOG_A, MOG_B] {
        let v = drag(40.0, -25.0);
        assert_eq!(v[0], Verdict::Pass, "the press reaches the page (hover, poke)");
        let start = v.iter().position(|x| *x == Verdict::StartDrag).expect("a drag starts");
        // After the drag starts, the page gets nothing: no mouse-up, so no click, no capture.
        assert!(v[start + 1..].iter().all(|x| *x == Verdict::Swallow), "{v:?}");
    }
    // A move below 4 px is a click for the page (MOG-12a rules, #83).
    let small = DRAG_THRESHOLD_PT - 0.5;
    assert!(drag(small, 0.0).iter().all(|x| *x == Verdict::Pass));
}

// ---------- Chords stay distinct (MB-14, core part) ----------

#[test]
fn at_01_mb_14_a_second_binding_that_duplicates_an_active_one_is_a_conflict() {
    let os = Os::MacOs;
    let current = default_bindings(os);
    let session = current.get(HotkeyAction::Session).expect("session key");
    let mut input = JsonObject::new();
    input.insert(
        HotkeyAction::OpenCard.key().to_owned(),
        Value::String(session.format(os)),
    );
    let out = hotkeys_set(os, &current, &input, &KeyboardEnv::default()).expect("hotkeys_set");
    let dup: Vec<_> = out
        .conflicts
        .iter()
        .filter(|c| c.kind == ConflictKind::Duplicate)
        .collect();
    assert_eq!(dup.len(), 1, "{:?}", out.conflicts);
    assert_eq!(dup[0].action, HotkeyAction::OpenCard);
    assert_eq!(dup[0].binding, session);
    assert_eq!(
        out.result_json(os)["conflicts"][0]["kind"],
        json!("duplicate")
    );
    // Distinct chords have no conflict.
    assert!(check_conflicts(os, &current, &KeyboardEnv::default()).is_empty());
}

#[test]
#[ignore = "pending TODO(#77): the hub does not save a binding with a conflict yet"]
fn at_01_mb_14_the_hub_does_not_save_the_duplicate_binding() {
    let os = Os::Windows;
    let current = default_bindings(os);
    let panic = current.get(HotkeyAction::Panic).expect("panic key");
    let mut input = JsonObject::new();
    input.insert("session".to_owned(), Value::String(panic.format(os)));
    let out = hotkeys_set(os, &current, &input, &KeyboardEnv::default()).expect("hotkeys_set");
    assert!(out.conflicts.iter().any(|c| c.kind == ConflictKind::Duplicate));
    let kept: Option<Binding> = current.get(HotkeyAction::Session);
    todo!(
        "TODO(#77): give this outcome to the hub save; assert app.hotkeys.session stays {:?} \
         and the hub shows the conflict",
        kept.map(|b| b.format(os))
    );
}

// ---------- The running cap (MB-03b, window part) ----------

#[test]
fn at_01_the_9th_mog_window_is_refused_at_the_cap_of_8() {
    assert_eq!(MAX_ACTIVE_MOGS, 8);
    let mut ledger = WindowLedger::default();
    let ids = mog_ids(MAX_ACTIVE_MOGS + 1);
    for id in &ids[..MAX_ACTIVE_MOGS] {
        assert!(matches!(ledger.open_mog(id), Ok(OpenOutcome::Opened(_))));
    }
    assert_eq!(
        ledger.open_mog(&ids[MAX_ACTIVE_MOGS]),
        Err(WindowError::CapReached { cap: 8 })
    );
    // Stop one, then the 9th starts.
    assert!(ledger.close_mog(&ids[0]));
    assert!(matches!(
        ledger.open_mog(&ids[MAX_ACTIVE_MOGS]),
        Ok(OpenOutcome::Opened(_))
    ));
}

// ---------- Pending: displays, click-through, Windows ----------
// MOG definitions and restart (MB-01, MB-03a, MB-03b text) are TS: src/acceptance/at-01-mogs.test.ts.
// These features have no interface yet, so each test ends in `todo!` with the assertion to add.

#[test]
#[ignore = "pending TODO(#82): per-display positions and display removal are not built (MOG-21 too)"]
fn at_01_mog_15a_mog_15b_mog_21_positions_survive_a_restart_and_a_removed_display() {
    let mut ledger = WindowLedger::default();
    ledger.open_mog(MOG_A).expect("open");
    ledger.open_mog(MOG_B).expect("open");
    todo!(
        "TODO(#82): save {{display UUID, normalised position}} per MOG, restart, remove the \
         display; assert both MOGs are on the primary display and visible; a new MOG with no \
         free slot opens at the right edge of the cursor display, 16 px apart"
    );
}

#[test]
#[ignore = "pending TODO(#81): click-through outside the sprite and the topmost watchdog are not built"]
fn at_01_mog_16a_mog_16b_mog_17_clicks_pass_through_outside_the_mascot() {
    let kind = WindowKind::mog(MOG_A).expect("id");
    assert!(kind.spec().transparent);
    todo!(
        "TODO(#81): apply the hit shape of {}; assert a click outside it reaches the app \
         below, the shape is applied again after a DPI change and the z-order is asserted \
         every 2 s with no activation",
        kind.label()
    );
}

#[test]
#[ignore = "pending TODO(#80): WS_EX_NOACTIVATE, the tool window and one WebView2 environment are not built"]
fn at_01_mog_04_windows_mog_windows_never_activate() {
    let spec = WindowKind::mog(MOG_A).expect("id").spec();
    assert!(!spec.focusable && spec.skip_taskbar && spec.always_on_top);
    todo!(
        "TODO(#80): on Windows, read back WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST, \
         SW_SHOWNOACTIVATE, MA_NOACTIVATE and one shared WebView2 environment for {}",
        spec.label
    );
}
