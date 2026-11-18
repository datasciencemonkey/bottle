//! #78: the card window. MOG-03a (key-capable non-activating panel at Floating level), MOG-03b
//! (the frontmost app stays), MOG-18a (made on the first show), MOG-18b (destroyed after 5 min
//! hidden), UX-19a (the only window that takes the keyboard) and UX-19b (Esc leaves no half
//! state). The AppKit read-back of a real card panel is in `tests/macos_panel_appkit.rs`; the
//! on-screen checks (AT-01, AT-20, AT-21) are `needs:mac` steps.

use bottle_lib::window_host::WindowLedger;
use bottle_lib::window_host::{CardState, NativeWindowStyle, TauriWindowHost, WindowError};
use bottle_lib::windows::{card_beside, PanelLevel, WindowKind, CARD_HIDDEN_TTL, CARD_REAP_TICK};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ID: &str = "7b0c1f3e-0000-4000-8000-000000000001";

fn card() -> WindowKind {
    WindowKind::card(ID).expect("valid id")
}

fn app_file(rel: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)).expect(rel)
}

/// The code of a Rust file without its `//` comment lines.
fn code_of(rel: &str) -> String {
    app_file(rel)
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------- MOG-03a: the card panel spec ----------

#[test]
fn mog_03a_card_is_a_key_capable_nonactivating_panel_at_floating_level() {
    let p = card().panel().expect("a card window is a panel");
    assert_eq!(p.level, PanelLevel::Floating);
    assert!(p.can_become_key, "typing and the IME need a key window");
    assert!(!p.can_become_main, "the card never becomes the main window");
    assert!(
        p.nonactivating,
        "a click on the card does not activate Bottle"
    );
    assert!(
        !p.hides_on_deactivate,
        "the card stays while another app is active"
    );
    assert!(
        p.full_screen_auxiliary,
        "the card shows over a fullscreen Space"
    );
    assert!(
        p.transparent,
        "#559: the card is transparent; the pop-up paints its own rounded shape"
    );
}

// ---------- MOG-03b: the frontmost app stays ----------

#[test]
fn mog_03b_card_is_built_hidden_and_unfocused_then_shown_by_the_native_style() {
    let spec = card().build_spec();
    assert!(card().spec().visible, "the card shows once it is open");
    assert!(!spec.visible, "built hidden, then made a panel, then shown");
    // `windows::build` focuses a window only if it is focusable and visible at build.
    assert!(
        !(spec.focusable && spec.visible),
        "the build never focuses the card"
    );
}

#[test]
fn mog_03b_macos_panel_code_never_activates_bottle() {
    let code = code_of("src/macos_panel.rs");
    for call in [
        "activateIgnoringOtherApps",
        "activateWithOptions",
        ".activate(",
        "set_focus",
        "makeKeyAndOrderFront",
    ] {
        assert!(!code.contains(call), "macos_panel.rs calls {call}");
    }
    assert!(code.contains("orderFrontRegardless"));
    assert!(code.contains("makeKeyWindow"), "the card panel becomes key");
}

#[derive(Default, Clone)]
struct Recorder(Arc<Mutex<Vec<String>>>);

impl Recorder {
    fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    fn count(&self, call: &str) -> usize {
        self.calls().iter().filter(|c| *c == call).count()
    }
}

impl<R: tauri::Runtime> NativeWindowStyle<R> for Recorder {
    fn style(&self, _: &tauri::WebviewWindow<R>, kind: &WindowKind) -> Result<(), WindowError> {
        self.0
            .lock()
            .unwrap()
            .push(format!("style {}", kind.label()));
        Ok(())
    }

    fn show(&self, _: &tauri::WebviewWindow<R>, kind: &WindowKind) -> Result<(), WindowError> {
        self.0
            .lock()
            .unwrap()
            .push(format!("show {}", kind.label()));
        Ok(())
    }
}

/// A clock the test moves by hand.
#[derive(Clone)]
struct FakeClock(Arc<Mutex<Instant>>);

impl FakeClock {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Instant::now())))
    }

    fn advance(&self, by: Duration) {
        *self.0.lock().unwrap() += by;
    }

    fn source(&self) -> Arc<dyn Fn() -> Instant + Send + Sync> {
        let c = self.0.clone();
        Arc::new(move || *c.lock().unwrap())
    }
}

type Host = TauriWindowHost<tauri::test::MockRuntime>;

fn host() -> (
    tauri::App<tauri::test::MockRuntime>,
    Host,
    Recorder,
    FakeClock,
) {
    let app = tauri::test::mock_app();
    let rec = Recorder::default();
    let clock = FakeClock::new();
    let host = TauriWindowHost::with_style(app.handle().clone(), Box::new(rec.clone()))
        .with_clock(clock.source());
    (app, host, rec, clock)
}

#[test]
fn mog_03b_host_shows_the_card_only_through_the_native_style() {
    let (_app, host, rec, _clock) = host();
    host.open_mog_window("m1").unwrap();
    host.show_card("m1").unwrap();
    assert_eq!(
        rec.calls(),
        [
            "style mog-m1",
            "show mog-m1",
            "style card-m1",
            "show card-m1"
        ]
    );
    // 20 opens: each one goes through the native show (`orderFrontRegardless`), never a
    // plain show that activates the app, and the panel step runs once.
    for _ in 0..20 {
        host.hide_card("m1").unwrap();
        host.show_card("m1").unwrap();
    }
    assert_eq!(rec.count("style card-m1"), 1);
    assert_eq!(rec.count("show card-m1"), 21);
}

// ---------- MOG-18a: the card WebView is made on the first open ----------

#[test]
fn mog_18a_no_card_window_exists_before_the_first_show() {
    use tauri::Manager;
    let (app, host, rec, _clock) = host();
    host.open_mog_window("m1").unwrap();
    assert!(app.get_webview_window("card-m1").is_none());
    assert_eq!(host.card_state("m1"), Some(CardState::Absent));
    assert_eq!(rec.count("style card-m1"), 0);
    host.show_card("m1").unwrap();
    assert!(app.get_webview_window("card-m1").is_some());
    assert_eq!(host.card_state("m1"), Some(CardState::Shown));
}

#[test]
fn mog_18a_a_release_start_makes_no_card() {
    assert!(bottle_lib::startup_windows(false)
        .iter()
        .all(|k| !matches!(k, WindowKind::Card(_))));
}

// ---------- MOG-18b: a card hidden for 5 min is destroyed ----------

#[test]
fn mog_18b_hidden_ttl_is_5_min_and_the_reaper_ticks_within_30_s() {
    assert_eq!(CARD_HIDDEN_TTL, Duration::from_secs(300));
    assert!(CARD_REAP_TICK > Duration::ZERO && CARD_REAP_TICK <= Duration::from_secs(30));
}

#[test]
fn mog_18b_ledger_reaps_a_card_hidden_for_5_min() {
    let t0 = Instant::now();
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    ledger.show_card("m1").unwrap();
    ledger.hide_card("m1", t0).unwrap();
    assert_eq!(ledger.card_state("m1"), Some(CardState::Hidden(t0)));
    let almost = t0 + CARD_HIDDEN_TTL - Duration::from_secs(1);
    assert!(
        ledger.take_due_cards(almost).is_empty(),
        "4:59 hidden stays"
    );
    assert_eq!(
        ledger.take_due_cards(t0 + CARD_HIDDEN_TTL),
        [WindowKind::Card("m1".into())]
    );
    assert_eq!(ledger.card_state("m1"), Some(CardState::Closing));
    assert!(
        ledger.take_due_cards(t0 + CARD_HIDDEN_TTL * 2).is_empty(),
        "a card is reaped once"
    );
}

#[test]
fn mog_18b_a_reshow_resets_the_hidden_timer() {
    let t0 = Instant::now();
    let min = Duration::from_secs(60);
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    ledger.show_card("m1").unwrap();
    ledger.hide_card("m1", t0).unwrap();
    ledger.show_card("m1").unwrap();
    ledger.hide_card("m1", t0 + 4 * min).unwrap();
    assert!(ledger.take_due_cards(t0 + 5 * min).is_empty());
    assert_eq!(ledger.take_due_cards(t0 + 9 * min).len(), 1);
}

#[test]
fn mog_18b_a_second_hide_keeps_the_first_hide_time() {
    let t0 = Instant::now();
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    ledger.show_card("m1").unwrap();
    ledger.hide_card("m1", t0).unwrap();
    ledger
        .hide_card("m1", t0 + Duration::from_secs(200))
        .unwrap();
    assert_eq!(ledger.take_due_cards(t0 + CARD_HIDDEN_TTL).len(), 1);
}

#[test]
fn mog_18b_a_shown_card_is_never_reaped() {
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    ledger.show_card("m1").unwrap();
    assert!(ledger
        .take_due_cards(Instant::now() + Duration::from_secs(3600))
        .is_empty());
}

#[test]
fn mog_18b_host_destroys_a_hidden_card_after_5_min_and_makes_it_again_on_show() {
    let (_app, host, rec, clock) = host();
    host.open_mog_window("m1").unwrap();
    host.open_mog_window("m2").unwrap();
    host.show_card("m1").unwrap();
    host.show_card("m2").unwrap();
    host.hide_card("m1").unwrap();
    clock.advance(CARD_HIDDEN_TTL - Duration::from_secs(1));
    assert_eq!(host.reap_hidden_cards(), 0);
    clock.advance(Duration::from_secs(1));
    assert_eq!(host.reap_hidden_cards(), 1, "only the hidden card goes");
    assert_eq!(host.card_state("m1"), Some(CardState::Closing));
    assert_eq!(host.card_state("m2"), Some(CardState::Shown));
    // A show while the old WebView is still closing must not adopt the dying window.
    assert_eq!(host.show_card("m1"), Err(WindowError::Closing));
    // lib.rs feeds `WindowEvent::Destroyed` to the host.
    host.window_destroyed("card-m1");
    assert_eq!(host.card_state("m1"), Some(CardState::Absent));
    assert_eq!(host.active_mogs(), 2, "reaping a card keeps its MOG");
    host.show_card("m1").unwrap();
    assert_eq!(rec.count("style card-m1"), 2, "a new card window is made");
}

#[test]
fn mog_18b_app_starts_the_card_reaper() {
    let lib = code_of("src/lib.rs");
    assert!(lib.contains("window_host::spawn_card_reaper("));
}

// ---------- UX-19a: only the card takes the keyboard ----------

#[test]
fn ux_19a_only_the_card_panel_can_become_key() {
    let kinds = [
        WindowKind::Brain,
        WindowKind::Hub,
        WindowKind::mog(ID).unwrap(),
        card(),
    ];
    let key: Vec<_> = kinds
        .iter()
        .filter(|k| k.panel().is_some_and(|p| p.can_become_key))
        .collect();
    assert_eq!(key, [&card()]);
    for k in [WindowKind::Brain, WindowKind::mog(ID).unwrap()] {
        assert!(!k.spec().focusable, "{k:?} must not take focus");
    }
    // The hub is the normal app window that the user opens from the Dock; it is not a
    // floating surface (ADR 0022).
}

// ---------- UX-19b: Esc leaves no half state ----------

#[test]
fn ux_19b_ledger_hide_is_all_or_nothing() {
    let t0 = Instant::now();
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    assert_eq!(ledger.hide_card("m1", t0), Err(WindowError::UnknownMog));
    assert_eq!(ledger.card_state("m1"), Some(CardState::Absent));
    assert_eq!(ledger.hide_card("zz", t0), Err(WindowError::UnknownMog));
    ledger.show_card("m1").unwrap();
    ledger.hide_card("m1", t0).unwrap();
    ledger.take_due_cards(t0 + CARD_HIDDEN_TTL);
    // A hide of a card that is closing already is done: it changes nothing.
    assert_eq!(
        ledger.hide_card("m1", t0),
        Ok(WindowKind::Card("m1".into()))
    );
    assert_eq!(ledger.card_state("m1"), Some(CardState::Closing));
}

#[test]
fn ux_19b_host_hide_of_a_card_that_never_opened_changes_nothing() {
    let (_app, host, _rec, _clock) = host();
    host.open_mog_window("m1").unwrap();
    assert_eq!(host.hide_card("m1"), Err(WindowError::UnknownMog));
    assert_eq!(host.card_state("m1"), Some(CardState::Absent));
    host.show_card("m1").unwrap();
    host.hide_card("m1").unwrap();
    assert!(matches!(host.card_state("m1"), Some(CardState::Hidden(_))));
}

// ---------- ARC-3: the two ledger gaps from the PR #292 review ----------

#[test]
fn arc_3_a_late_destroyed_event_does_not_free_a_reopened_mog() {
    let (_app, host, _rec, _clock) = host();
    host.open_mog_window("m1").unwrap();
    host.close_mog_window("m1").unwrap();
    // The old window is still closing: a reopen would adopt it, so it waits.
    assert_eq!(host.open_mog_window("m1"), Err(WindowError::Closing));
    host.window_destroyed("mog-m1");
    host.open_mog_window("m1").unwrap();
    assert_eq!(host.active_mogs(), 1);
    // The expected event was used up: the new window keeps its slot.
    assert_eq!(host.card_state("m1"), Some(CardState::Absent));
}

#[test]
fn arc_3_ledger_ignores_the_destroyed_event_it_expects() {
    let mut ledger = WindowLedger::new(2);
    ledger.open_mog("m1").unwrap();
    ledger.close_mog("m1");
    ledger.expect_destroyed("mog-m1");
    assert_eq!(ledger.open_mog("m1"), Err(WindowError::Closing));
    assert!(ledger.window_destroyed("mog-m1").is_empty());
    ledger.open_mog("m1").unwrap();
    // A real destroy of the new window still frees its slot.
    ledger.window_destroyed("mog-m1");
    assert_eq!(ledger.active_mogs(), 0);
}

#[test]
fn arc_3_a_mog_destroyed_outside_the_host_takes_its_card() {
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    ledger.open_mog("m2").unwrap();
    ledger.show_card("m1").unwrap();
    assert_eq!(
        ledger.window_destroyed("mog-m1"),
        [WindowKind::Card("m1".into())],
        "the orphan card must be destroyed"
    );
    assert!(
        ledger.window_destroyed("mog-m2").is_empty(),
        "m2 had no card"
    );
}

#[test]
fn arc_3_host_destroys_the_card_of_a_mog_destroyed_outside_it() {
    let (app, host, _rec, _clock) = host();
    use tauri::Manager;
    host.open_mog_window("m1").unwrap();
    host.show_card("m1").unwrap();
    app.get_webview_window("mog-m1").unwrap().destroy().unwrap();
    host.window_destroyed("mog-m1");
    assert_eq!(host.active_mogs(), 0);
    // The host destroyed `card-m1` and waits for its event before the label is used again.
    host.open_mog_window("m1").unwrap();
    assert_eq!(host.show_card("m1"), Err(WindowError::Closing));
    host.window_destroyed("card-m1");
    host.show_card("m1").unwrap();
}

#[test]
fn arc_3_closing_error_has_a_stable_code_and_names_no_id() {
    let e = WindowError::Closing;
    assert_eq!(e.code(), "window_closing");
    let text = format!("{e} {e:?}");
    assert!(!text.contains("m1") && !text.contains("mog-"), "{text}");
}

// ---------- PR #338 review: closing races and one critical section per card operation ----------

#[test]
fn arc_3_two_expectations_for_one_window_leave_the_label_reusable() {
    // The reaper and a MOG close can both destroy the same card; one window gives one event.
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    ledger.show_card("m1").unwrap();
    ledger.expect_destroyed("card-m1");
    ledger.expect_destroyed("card-m1");
    ledger.window_destroyed("card-m1");
    assert_eq!(ledger.show_card("m1"), Ok(WindowKind::Card("m1".into())));
}

#[test]
fn arc_3_begin_closing_records_only_a_live_window_once() {
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    ledger.show_card("m1").unwrap();
    // The Destroyed event came first (Tauri drops the window from its manager before the
    // handler runs): no live window, so nothing is expected and the label stays free.
    ledger.window_destroyed("card-m1");
    assert!(!ledger.begin_closing("card-m1", false));
    assert_eq!(ledger.show_card("m1"), Ok(WindowKind::Card("m1".into())));
    // A live window is recorded once; a second destroy of it is skipped.
    assert!(ledger.begin_closing("card-m1", true));
    assert!(!ledger.begin_closing("card-m1", true));
    // A destroy that failed takes the record back.
    ledger.cancel_closing("card-m1");
    assert_eq!(ledger.show_card("m1"), Ok(WindowKind::Card("m1".into())));
}

#[test]
fn ux_19b_a_card_window_gone_before_its_event_waits_for_the_event() {
    // The window left the manager but its Destroyed event is not handled yet: a show must not
    // make a new window that the late event then marks absent.
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    ledger.show_card("m1").unwrap();
    ledger.card_window_missing("m1");
    assert_eq!(ledger.card_state("m1"), Some(CardState::Absent));
    assert_eq!(ledger.show_card("m1"), Err(WindowError::Closing));
    ledger.window_destroyed("card-m1");
    assert_eq!(ledger.show_card("m1"), Ok(WindowKind::Card("m1".into())));
}

/// Records shows and hides; the card show waits, so another thread can run in between.
#[derive(Default, Clone)]
struct SlowShow(Arc<Mutex<Vec<String>>>);

impl<R: tauri::Runtime> NativeWindowStyle<R> for SlowShow {
    fn style(&self, _: &tauri::WebviewWindow<R>, _: &WindowKind) -> Result<(), WindowError> {
        Ok(())
    }

    fn show(&self, _: &tauri::WebviewWindow<R>, kind: &WindowKind) -> Result<(), WindowError> {
        if matches!(kind, WindowKind::Card(_)) {
            std::thread::sleep(Duration::from_millis(150));
        }
        self.0
            .lock()
            .unwrap()
            .push(format!("show {}", kind.label()));
        Ok(())
    }

    fn hide(&self, _: &tauri::WebviewWindow<R>, kind: &WindowKind) -> Result<(), WindowError> {
        self.0
            .lock()
            .unwrap()
            .push(format!("hide {}", kind.label()));
        Ok(())
    }
}

#[test]
fn ux_19b_a_hide_during_a_show_leaves_the_ledger_matching_the_window() {
    let app = tauri::test::mock_app();
    let rec = SlowShow::default();
    let host = Arc::new(TauriWindowHost::with_style(
        app.handle().clone(),
        Box::new(rec.clone()),
    ));
    host.open_mog_window("m1").unwrap();
    host.show_card("m1").unwrap();
    host.hide_card("m1").unwrap();
    let shower = {
        let host = host.clone();
        std::thread::spawn(move || host.show_card("m1"))
    };
    std::thread::sleep(Duration::from_millis(40));
    let hid = host.hide_card("m1");
    shower.join().unwrap().unwrap();
    hid.unwrap();
    let last = rec
        .0
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|c| c.ends_with("card-m1"))
        .cloned()
        .unwrap();
    let state = host.card_state("m1").unwrap();
    match last.as_str() {
        "show card-m1" => assert_eq!(state, CardState::Shown, "the card shows"),
        _ => assert!(
            matches!(state, CardState::Hidden(_)),
            "{last} but {state:?}"
        ),
    }
}

/// Shows a MOG, and fails every card show after the first.
#[derive(Default)]
struct FailSecondCardShow(Mutex<usize>);

impl<R: tauri::Runtime> NativeWindowStyle<R> for FailSecondCardShow {
    fn style(&self, _: &tauri::WebviewWindow<R>, _: &WindowKind) -> Result<(), WindowError> {
        Ok(())
    }

    fn show(&self, _: &tauri::WebviewWindow<R>, kind: &WindowKind) -> Result<(), WindowError> {
        if matches!(kind, WindowKind::Card(_)) {
            let mut n = self.0.lock().unwrap();
            *n += 1;
            if *n > 1 {
                return Err(WindowError::HostFailed);
            }
        }
        Ok(())
    }
}

#[test]
fn mog_18b_a_failed_show_of_a_hidden_card_keeps_it_hidden_for_the_reaper() {
    let app = tauri::test::mock_app();
    let clock = FakeClock::new();
    let host = TauriWindowHost::with_style(
        app.handle().clone(),
        Box::new(FailSecondCardShow::default()),
    )
    .with_clock(clock.source());
    host.open_mog_window("m1").unwrap();
    host.show_card("m1").unwrap();
    host.hide_card("m1").unwrap();
    let hidden = host.card_state("m1");
    assert_eq!(host.show_card("m1"), Err(WindowError::HostFailed));
    assert_eq!(host.card_state("m1"), hidden, "the card did not show");
    clock.advance(CARD_HIDDEN_TTL);
    assert_eq!(host.reap_hidden_cards(), 1);
}

// ---------- PR #338 review round 2 ----------

use bottle_lib::window_host::ShowPlan;

#[test]
fn ux_19b_begin_show_reads_the_window_and_writes_the_state_in_one_step() {
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    assert_eq!(ledger.begin_show("m1", false), Ok(ShowPlan::Make));
    ledger.hide_card("m1", Instant::now()).unwrap();
    let hidden = ledger.card_state("m1").unwrap();
    assert_eq!(ledger.begin_show("m1", true), Ok(ShowPlan::Reuse(hidden)));
    assert_eq!(ledger.card_state("m1"), Some(CardState::Shown));
}

#[test]
fn ux_19b_an_outside_close_handled_before_a_show_lets_the_show_make_a_new_card() {
    // Round 2, MEDIUM A: the Destroyed event of an outside close came first. The show sees
    // `Absent` and no window in the same step, so it makes a new card and records nothing.
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    ledger.show_card("m1").unwrap();
    ledger.hide_card("m1", Instant::now()).unwrap();
    ledger.window_destroyed("card-m1");
    assert_eq!(ledger.begin_show("m1", false), Ok(ShowPlan::Make));
    assert!(!ledger.is_closing(&WindowKind::Card("m1".into())));
}

#[test]
fn ux_19b_a_card_gone_before_its_event_refuses_show_and_hide_until_the_event() {
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    ledger.show_card("m1").unwrap();
    assert_eq!(ledger.begin_show("m1", false), Err(WindowError::Closing));
    assert_eq!(ledger.card_state("m1"), Some(CardState::Absent));
    assert_eq!(ledger.begin_hide("m1", false), Err(WindowError::UnknownMog));
    ledger.window_destroyed("card-m1");
    assert_eq!(ledger.begin_show("m1", false), Ok(ShowPlan::Make));
}

#[test]
fn ux_19b_begin_hide_needs_a_live_card_window() {
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    assert_eq!(ledger.begin_hide("m1", false), Err(WindowError::UnknownMog));
    assert_eq!(ledger.begin_hide("zz", false), Err(WindowError::UnknownMog));
    ledger.show_card("m1").unwrap();
    assert_eq!(ledger.begin_hide("m1", true), Ok(true));
    ledger.hide_card("m1", Instant::now()).unwrap();
    ledger.take_due_cards(Instant::now() + CARD_HIDDEN_TTL);
    assert_eq!(
        ledger.begin_hide("m1", true),
        Ok(false),
        "a closing card needs no hide"
    );
}

#[test]
fn arc_3_a_mog_window_gone_before_its_event_is_not_made_again() {
    // Round 2, LOW C: the late event must not free the slot of a second window.
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    ledger.mog_window_missing("m1");
    assert_eq!(ledger.active_mogs(), 0);
    assert_eq!(ledger.open_mog("m1"), Err(WindowError::Closing));
    ledger.window_destroyed("mog-m1");
    assert!(ledger.open_mog("m1").is_ok());
    assert_eq!(ledger.active_mogs(), 1);
}

#[test]
fn arc_3_app_seals_the_main_thread_after_setup_and_before_the_reaper() {
    let lib = code_of("src/lib.rs");
    let seal = lib
        .find(".seal_main_thread()")
        .expect("setup seals the main thread");
    let reaper = lib.find("window_host::spawn_card_reaper(").unwrap();
    let windows = lib.find("for kind in startup_windows(demo)").unwrap();
    assert!(
        windows < seal && seal < reaper,
        "startup windows, seal, then the reaper"
    );
}

// ---------- PR #338 review round 3 ----------

#[test]
fn arc_3_sealed_host_refuses_operations_on_the_sealing_thread_on_every_os() {
    // Setup seals on the main thread; the check compares thread IDs, so it holds on every OS.
    let (_app, host, _rec, _clock) = host();
    host.open_mog_window("m1").unwrap();
    host.seal_main_thread();
    assert_eq!(host.show_card("m1"), Err(WindowError::MainThread));
    assert_eq!(host.open_mog_window("m2"), Err(WindowError::MainThread));
    assert_eq!(host.reap_hidden_cards(), 0);
    let host = Arc::new(host);
    let other = host.clone();
    std::thread::spawn(move || other.show_card("m1"))
        .join()
        .unwrap()
        .unwrap();
    assert_eq!(host.card_state("m1"), Some(CardState::Shown));
}

#[test]
fn mog_18b_a_failed_show_does_not_write_hidden_over_a_card_closed_meanwhile() {
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    ledger.show_card("m1").unwrap();
    let t0 = Instant::now();
    ledger.hide_card("m1", t0).unwrap();
    assert_eq!(
        ledger.begin_show("m1", true),
        Ok(ShowPlan::Reuse(CardState::Hidden(t0)))
    );
    // An outside close is handled before the native show fails.
    ledger.window_destroyed("card-m1");
    ledger.restore_hidden("m1", t0);
    assert_eq!(ledger.card_state("m1"), Some(CardState::Absent));
    assert_eq!(ledger.begin_show("m1", false), Ok(ShowPlan::Make));
}

#[test]
fn arc_3_a_mog_gone_before_its_event_hands_its_card_to_be_destroyed() {
    let mut ledger = WindowLedger::default();
    ledger.open_mog("m1").unwrap();
    ledger.show_card("m1").unwrap();
    assert_eq!(
        ledger.mog_window_missing("m1"),
        Some(WindowKind::Card("m1".into()))
    );
    ledger.open_mog("m2").unwrap();
    assert_eq!(ledger.mog_window_missing("m2"), None, "m2 had no card");
}

// The host branch of `open_mog_window` that calls `mog_window_missing` needs a MOG window that
// left Tauri's manager while the ledger still has it. The mock runtime drops a destroyed window
// only inside `App::run`, which never returns, so a mock test cannot reach that branch. The
// ledger tests above pin its decisions; the device check (#336) runs the real event loop.

// ---------- #559: the pop-up window beside its MOG ----------

#[test]
fn issue_559_card_window_is_a_small_transparent_window_with_no_native_shadow() {
    let spec = card().spec();
    assert!(spec.transparent, "only the pop-up pixels paint");
    assert!(
        !spec.shadow,
        "the pop-up draws its own shadow; a native one halos the window"
    );
    assert!(
        spec.width <= 360 && spec.height <= 96,
        "{}x{}",
        spec.width,
        spec.height
    );
}

fn g(x: f64, y: f64, width: f64, height: f64) -> mog_core::ipc::Geometry {
    mog_core::ipc::Geometry {
        x,
        y,
        width,
        height,
    }
}

#[test]
fn issue_559_card_sits_beside_the_mog_on_the_right_centred_on_it() {
    let spec = card().spec();
    let mog = g(100.0, 200.0, 160.0, 160.0);
    let c = card_beside(&mog, Some(&g(0.0, 0.0, 1440.0, 900.0)));
    assert_eq!(
        (c.width, c.height),
        (f64::from(spec.width), f64::from(spec.height))
    );
    assert!(
        c.x >= mog.x + mog.width - 24.0 && c.x <= mog.x + mog.width,
        "x {}",
        c.x
    );
    assert_eq!(c.y, mog.y + (mog.height - c.height) / 2.0);
}

#[test]
fn issue_559_card_goes_left_of_the_mog_at_the_right_edge_and_stays_on_screen() {
    let screen = g(0.0, 0.0, 1440.0, 900.0);
    let mog = g(1300.0, 860.0, 160.0, 160.0);
    let c = card_beside(&mog, Some(&screen));
    assert!(c.x + c.width <= mog.x + 24.0, "left of the MOG: x {}", c.x);
    assert!(
        c.y + c.height <= screen.y + screen.height,
        "on screen: y {}",
        c.y
    );
    let no_screen = card_beside(&mog, None);
    assert!(
        no_screen.x >= mog.x + mog.width - 24.0,
        "no screen: the right side"
    );
}
