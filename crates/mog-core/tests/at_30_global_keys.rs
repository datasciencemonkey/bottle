//! AT-30 "Global-key defaults and Secure Input" (#230), section 15.
//!
//! The AT script: 1,000 HID cycles, Secure Keyboard Entry during a hold, AltGr and Copilot
//! layouts, the panic key without Input Monitoring. This file runs the script against the
//! shipped R1 code (#6 macOS, #7 Windows, #24 default keys) with the #229 simulated clock and
//! the `testkit::hotkey` doubles. No test here touches the OS, a key or a prompt.
//!
//! What this file does not do (docs/tests/AT-MOGS-TALKKEY-A11Y.md):
//! - The 1,000-cycle HID numbers (NFR-08a, NFR-08b) come from the #171 bench:
//!   `cargo test -p mog-core --test latency_bench at_30` (docs/tests/LATENCY-BENCH.md, Part A).
//!   This file does not copy that test.
//! - The device runs skip with the reason unless a person sets the fixture.
//! - The panic key (MB-05) waits for #66, wizard step 7 (UX-37a, UX-37b) for #43.

use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use mog_core::clock::{MonotonicClock, SimClock, SystemClock};
use mog_core::hotkey_bindings::{
    self, default_bindings, first_run_check, Binding, ConflictKind, DictationShortcut, Fix,
    HotkeyAction, KeyboardEnv, Os, TalkBinding, VoiceApp,
};
use mog_core::hotkey_core::{
    check_ptt_permission, request_insertion_permission, request_ptt_permission, HotkeyEngine,
    HotkeyOutput, KeyEdge, Permission, PermissionStatus, PhysicalState, PttPhase, TalkKey,
    TapSignal, UserAction, SECURE_INPUT_DEADLINE, TALK_KEY_PAUSED_PILL, TAP_FAILURE_LIMIT,
    WATCHDOG_TICK,
};
use mog_core::hotkey_macos::{
    classify_flags_changed, talk_key_code, FlagsEvent, CG_EVENT_FLAGS_CHANGED, CG_EVENT_KEY_DOWN,
    CG_EVENT_TAP_OPTION_LISTEN_ONLY, OTHER_KEY_TAP_SPEC, TAP_SPEC,
};
use mog_core::hotkey_windows::{
    forward_to_engine, vk, Elevation, ElevationGate, HookCore, HookFilter, HookInstaller,
    HookSignal, HookSupervisor, KeyEvent, LatencyRecorder, TalkKey as WinTalkKey, Verdict,
    CALLBACK_BUDGET, REINSTALL_INTERVAL,
};
use mog_core::keyboard_env::{
    read_keyboard_env, CopilotKeyFlag, EnvReads, MemoryCopilotKeyFlag, WithCopilotFlag,
};
use mog_core::latency_bench::import::{
    parse_sp03_secure_input, SecureInputSamples, SecureInputVerdict,
};
use mog_core::latency_bench::KeySampleTool;
use mog_core::testkit::hotkey::{
    FakePermissions, RecordingSink, RecordingTapControl, ScriptedProbe, TapCall,
};
use mog_core::testkit::{self, Fixture};

const CYCLES: usize = 1_000;

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

fn ptt(phase: PttPhase) -> HotkeyOutput {
    HotkeyOutput::Ptt(phase)
}

/// An engine on a simulated clock, its sink, the probe and the tap control. The first tick
/// records the baseline of the watchdog.
fn engine() -> (
    HotkeyEngine,
    Arc<RecordingSink>,
    SimClock,
    ScriptedProbe,
    RecordingTapControl,
) {
    let clock = SimClock::new();
    let sink = RecordingSink::new();
    let mut e = HotkeyEngine::with_clock(sink.clone(), Arc::new(clock.clone()));
    let probe = ScriptedProbe::new();
    let ctl = RecordingTapControl::new();
    e.tick(clock.now(), &probe, &ctl);
    ctl.take();
    (e, sink, clock, probe, ctl)
}

fn tick(e: &mut HotkeyEngine, clock: &SimClock, probe: &ScriptedProbe, ctl: &RecordingTapControl) {
    clock.advance(WATCHDOG_TICK);
    e.tick(clock.now(), probe, ctl);
}

// ---------- 1,000 cycles: every key-up reaches R1 (CAP-35a, CAP-35b) ----------

#[test]
fn at_30_cap_35a_the_build_has_only_listen_only_taps() {
    for spec in [TAP_SPEC, OTHER_KEY_TAP_SPEC] {
        assert_eq!(spec.options, CG_EVENT_TAP_OPTION_LISTEN_ONLY, "{spec:?}");
    }
    // The PTT tap asks for FlagsChanged only (the talk key is modifier-only), the
    // other-key tap for KeyDown only. Neither asks for KeyUp, so no typed key is seen.
    assert_eq!(TAP_SPEC.events_of_interest, 1 << CG_EVENT_FLAGS_CHANGED);
    assert_eq!(
        OTHER_KEY_TAP_SPEC.events_of_interest,
        1 << CG_EVENT_KEY_DOWN
    );
}

#[test]
fn at_30_cap_35b_macos_1000_simulated_hold_cycles_give_1000_key_ups() {
    let (mut e, sink, clock, _probe, _ctl) = engine();
    let (code, mask) = talk_key_code(TalkKey::RightOption);
    for _ in 0..CYCLES {
        for (flags, hold) in [(mask, ms(300)), (0, ms(100))] {
            match classify_flags_changed(TalkKey::RightOption, code, flags) {
                FlagsEvent::Talk(edge) => e.on_talk_key(edge),
                other => panic!("the talk key decoded as {other:?}"),
            }
            clock.advance(hold);
        }
    }
    let out = sink.take();
    let downs = out.iter().filter(|o| **o == ptt(PttPhase::Down)).count();
    let ups = out.iter().filter(|o| **o == ptt(PttPhase::Up)).count();
    assert_eq!((downs, ups, out.len()), (CYCLES, CYCLES, 2 * CYCLES));
    assert!(!e.machine().is_held());
}

#[test]
fn at_30_cap_35b_windows_1000_simulated_hold_cycles_give_1000_key_ups() {
    let clock = SimClock::new();
    let sink = RecordingSink::new();
    let mut e = HotkeyEngine::with_clock(sink.clone(), Arc::new(clock.clone()));
    let mut filter = HookFilter::new(WinTalkKey::RightAlt);
    for _ in 0..CYCLES {
        for (ev, hold) in [
            (KeyEvent::down(vk::RMENU), ms(300)),
            (KeyEvent::up(vk::RMENU), ms(100)),
        ] {
            let (verdict, signal) = filter.on_key(ev, clock.now());
            assert_eq!(
                verdict,
                Verdict::Swallow,
                "CAP-41: the talk key never reaches the app"
            );
            let signal = signal.expect("each talk-key edge gives a signal");
            assert_eq!(forward_to_engine(signal, &mut e), None);
            clock.advance(hold);
        }
    }
    let out = sink.take();
    let ups = out.iter().filter(|o| **o == ptt(PttPhase::Up)).count();
    assert_eq!((ups, out.len()), (CYCLES, 2 * CYCLES));
}

// ---------- Secure Keyboard Entry during a hold (CAP-39a, CAP-39b, CAP-39c) ----------

#[test]
fn at_30_cap_39a_secure_input_ends_an_active_ptt_within_250_ms() {
    let (mut e, sink, clock, probe, ctl) = engine();
    e.on_talk_key(KeyEdge::Down);
    clock.advance(ms(600));
    assert_eq!(sink.take(), vec![ptt(PttPhase::Down)]);

    // Worst case: Secure Input starts just after a tick, so R1 sees it on the next tick.
    probe.set_secure_input(true);
    probe.set_physical(PhysicalState {
        talk_key_down: true,
        chord_down: false,
    });
    let started = clock.now();
    tick(&mut e, &clock, &probe, &ctl);
    let out = sink.take();
    assert_eq!(out.first(), Some(&ptt(PttPhase::Cancel)), "{out:?}");
    assert!(clock.now() - started <= SECURE_INPUT_DEADLINE);
    assert!(WATCHDOG_TICK <= SECURE_INPUT_DEADLINE);
    assert!(!e.machine().is_held());

    // The key-up that comes later is not a second end.
    e.on_talk_key(KeyEdge::Up);
    assert_eq!(sink.take(), vec![]);
}

#[test]
fn at_30_cap_39b_the_carbon_chord_drives_ptt_while_secure_input_is_on() {
    let (mut e, sink, clock, probe, ctl) = engine();
    probe.set_secure_input(true);
    tick(&mut e, &clock, &probe, &ctl);
    assert!(ctl.take().contains(&TapCall::SetChord(true)));
    e.on_chord_registered(true, clock.now());
    sink.take();

    // The talk key is ignored; the chord press and release work.
    e.on_talk_key(KeyEdge::Down);
    e.on_talk_key(KeyEdge::Up);
    assert_eq!(sink.take(), vec![]);
    e.on_chord(KeyEdge::Down);
    clock.advance(ms(800));
    e.on_chord(KeyEdge::Up);
    assert_eq!(sink.take(), vec![ptt(PttPhase::Down), ptt(PttPhase::Up)]);

    // Secure Input ends: the chord goes away and the talk key works again.
    probe.set_secure_input(false);
    tick(&mut e, &clock, &probe, &ctl);
    assert!(ctl.take().contains(&TapCall::SetChord(false)));
    assert_eq!(sink.take(), vec![HotkeyOutput::TalkKeyRestored]);
    e.on_talk_key(KeyEdge::Down);
    assert_eq!(sink.take(), vec![ptt(PttPhase::Down)]);
}

#[test]
fn at_30_cap_39c_secure_input_shows_the_talk_key_paused_pill() {
    let (mut e, sink, clock, probe, ctl) = engine();
    probe.set_secure_input(true);
    tick(&mut e, &clock, &probe, &ctl);
    let out = sink.take();
    assert_eq!(out, vec![HotkeyOutput::TalkKeyPaused]);
    assert_eq!(
        out[0].pill_text(),
        Some("Talk key paused: another app protects typing. Use ⌃⇧Space.")
    );
    assert_eq!(TALK_KEY_PAUSED_PILL, out[0].pill_text().unwrap_or_default());
}

// ---------- Tap health and key-state re-read (CAP-38a, CAP-38b, CAP-45) ----------

#[test]
fn at_30_cap_38a_the_watchdog_starts_the_tap_again_after_a_timeout() {
    let (mut e, sink, clock, probe, ctl) = engine();
    e.on_tap_signal(TapSignal::DisabledByTimeout, clock.now(), &probe, &ctl);
    assert_eq!(ctl.take(), vec![TapCall::ReEnable]);

    // Repeated timeouts make the watchdog create a new tap.
    for _ in 1..TAP_FAILURE_LIMIT {
        clock.advance(ms(500));
        e.on_tap_signal(TapSignal::DisabledByTimeout, clock.now(), &probe, &ctl);
    }
    assert_eq!(ctl.take().last(), Some(&TapCall::Recreate));

    // PTT works after the timeout.
    e.on_talk_key(KeyEdge::Down);
    clock.advance(ms(400));
    e.on_talk_key(KeyEdge::Up);
    assert_eq!(sink.take(), vec![ptt(PttPhase::Down), ptt(PttPhase::Up)]);
}

#[test]
fn at_30_cap_38b_the_watchdog_makes_a_new_tap_on_wake_session_and_grant() {
    let (mut e, _sink, clock, probe, ctl) = engine();

    probe.sleep(Duration::from_secs(30 * 60));
    tick(&mut e, &clock, &probe, &ctl);
    assert_eq!(ctl.take(), vec![TapCall::Recreate], "wake");

    probe.set_session_active(false);
    tick(&mut e, &clock, &probe, &ctl);
    probe.set_session_active(true);
    tick(&mut e, &clock, &probe, &ctl);
    assert_eq!(ctl.take(), vec![TapCall::Recreate], "session activation");

    probe.set_input_monitoring(false);
    tick(&mut e, &clock, &probe, &ctl);
    probe.set_input_monitoring(true);
    tick(&mut e, &clock, &probe, &ctl);
    assert_eq!(ctl.take(), vec![TapCall::Recreate], "permission grant");
}

#[test]
fn at_30_cap_45_a_lost_key_up_ends_the_ptt_after_wake_and_after_secure_input() {
    // macOS: the key-up is lost while the Mac sleeps.
    let (mut e, sink, clock, probe, ctl) = engine();
    e.on_talk_key(KeyEdge::Down);
    clock.advance(ms(400));
    sink.take();
    probe.set_physical(PhysicalState::default());
    probe.sleep(Duration::from_secs(60));
    tick(&mut e, &clock, &probe, &ctl);
    assert_eq!(sink.take(), vec![ptt(PttPhase::Up)]);
    assert!(!e.machine().is_held());

    // A chord PTT whose key-up is lost ends when Secure Input ends.
    probe.set_secure_input(true);
    tick(&mut e, &clock, &probe, &ctl);
    e.on_chord_registered(true, clock.now());
    e.on_chord(KeyEdge::Down);
    sink.take();
    probe.set_secure_input(false);
    tick(&mut e, &clock, &probe, &ctl);
    assert_eq!(
        sink.take(),
        vec![ptt(PttPhase::Up), HotkeyOutput::TalkKeyRestored]
    );

    // Windows: unlock and resume end a PTT whose key-up the hook did not see.
    for unlock in [true, false] {
        let mut f = HookFilter::new(WinTalkKey::RightAlt);
        let (_, down) = f.on_key(KeyEvent::down(vk::RMENU), ms(0));
        assert!(matches!(down, Some(HookSignal::TalkDown { .. })));
        let end = if unlock {
            f.on_unlock(|_| false, ms(5))
        } else {
            f.on_resume(ms(5))
        };
        assert!(
            matches!(end, Some(HookSignal::TalkUp { .. })),
            "unlock={unlock}"
        );
        assert!(!f.talk_active());
    }
}

// ---------- Permissions (CAP-36, CAP-37) ----------

#[test]
fn at_30_cap_36_ptt_asks_only_for_input_monitoring_and_only_after_a_user_action() {
    let api = FakePermissions::new();
    // Start: a status read, no prompt. PTT does not start without the grant.
    assert!(check_ptt_permission(&api).is_err());
    assert_eq!(api.prompts(), vec![]);

    let granted = request_ptt_permission(&api, UserAction::from_user_click());
    assert_eq!(granted, PermissionStatus::Granted);
    assert_eq!(api.prompts(), vec![Permission::InputMonitoring]);
    assert!(check_ptt_permission(&api).is_ok());
    assert_eq!(
        api.prompts(),
        vec![Permission::InputMonitoring],
        "no second prompt"
    );
}

#[test]
fn at_30_cap_37_accessibility_is_asked_only_when_the_user_selects_insertion() {
    let api = FakePermissions::new();
    request_ptt_permission(&api, UserAction::from_user_click());
    assert!(!api.prompts().contains(&Permission::Accessibility));

    request_insertion_permission(&api, UserAction::from_user_click());
    assert_eq!(
        api.prompts(),
        vec![Permission::InputMonitoring, Permission::Accessibility]
    );
}

// ---------- Windows hook (CAP-40a, CAP-40b, CAP-41, CAP-42) ----------

#[test]
fn at_30_cap_40a_the_hook_callback_body_p99_is_under_1_ms() {
    // The real callback body with the system clock; the OS hook overhead is a device step.
    let recorder = Arc::new(LatencyRecorder::with_capacity(4 * CYCLES));
    let (tx, rx) = mpsc::channel();
    let clock: Arc<dyn MonotonicClock> = Arc::new(SystemClock::new());
    let mut core = HookCore::new(
        WinTalkKey::RightAlt,
        Arc::new(AtomicBool::new(false)),
        recorder.clone(),
        clock,
        tx,
    );
    for _ in 0..CYCLES {
        core.handle(KeyEvent::down(vk::RMENU));
        core.handle(KeyEvent::up(vk::RMENU));
    }
    assert_eq!(recorder.count(), 2 * CYCLES);
    assert_eq!(rx.try_iter().count(), 2 * CYCLES);
    let p99 = recorder.p99().expect("samples");
    assert!(p99 < CALLBACK_BUDGET, "p99 {p99:?}");
}

#[derive(Default)]
struct CountingInstaller {
    installs: usize,
    live: usize,
}

impl HookInstaller for CountingInstaller {
    type Handle = usize;
    fn install(&mut self) -> Result<usize, &'static str> {
        self.installs += 1;
        self.live += 1;
        Ok(self.installs)
    }
    fn uninstall(&mut self, _handle: usize) {
        self.live -= 1;
    }
}

#[test]
fn at_30_cap_40b_the_hook_is_installed_again_every_10_min_and_after_resume_in_an_8_h_soak() {
    let mut sup = HookSupervisor::new(CountingInstaller::default());
    let mut now = Duration::ZERO;
    sup.start(now).expect("start");
    let soak = Duration::from_secs(8 * 3600);
    let mut reinstalls = 0;
    while now < soak {
        now += Duration::from_secs(30);
        if sup.on_timer(now).expect("timer") {
            reinstalls += 1;
        }
    }
    let expected = (soak.as_secs() / REINSTALL_INTERVAL.as_secs()) as usize;
    assert_eq!(reinstalls, expected);
    sup.on_resume(now).expect("resume");
    assert_eq!(sup.installer().installs, 1 + expected + 1);
    assert_eq!(sup.installer().live, 1, "one hook at a time");
    assert!(sup.is_installed());
}

#[test]
fn at_30_cap_41_the_talk_key_and_esc_are_swallowed_during_capture_in_100_presses() {
    let mut f = HookFilter::new(WinTalkKey::RightAlt);
    for i in 0..100u64 {
        let at = ms(i * 10);
        assert_eq!(f.on_key(KeyEvent::down(vk::RMENU), at).0, Verdict::Swallow);
        let (esc, signal) = f.on_key(KeyEvent::down(vk::ESCAPE), at);
        assert_eq!((esc, signal), (Verdict::Swallow, Some(HookSignal::Escape)));
        assert_eq!(f.on_key(KeyEvent::up(vk::ESCAPE), at).0, Verdict::Swallow);
        assert_eq!(f.on_key(KeyEvent::up(vk::RMENU), at).0, Verdict::Swallow);
    }
    // Outside capture, Esc reaches the app.
    assert_eq!(
        f.on_key(KeyEvent::down(vk::ESCAPE), ms(5_000)),
        (Verdict::Pass, None)
    );
}

#[test]
fn at_30_cap_42_an_elevated_foreground_window_blocks_the_talk_key_once() {
    let mut gate = ElevationGate::new(false);
    assert_eq!(
        gate.on_foreground(42, Elevation::Elevated),
        Some(HookSignal::TalkKeyBlocked)
    );
    assert_eq!(
        gate.on_foreground(42, Elevation::Elevated),
        None,
        "first failure only"
    );
    assert_eq!(gate.on_foreground(43, Elevation::NotElevated), None);
    // An elevated Bottle sees elevated windows, so it shows nothing.
    let mut elevated = ElevationGate::new(true);
    assert_eq!(elevated.on_foreground(42, Elevation::Elevated), None);
}

// ---------- Defaults and layouts (CAP-43a, CAP-43b) ----------

#[test]
fn at_30_cap_43a_no_default_global_key_uses_ctrl_option() {
    let mac = default_bindings(Os::MacOs);
    for action in HotkeyAction::ALL {
        let b = mac
            .get(action)
            .unwrap_or_else(|| panic!("macOS has no {action:?}"));
        // Ctrl+Option is the VoiceOver modifier (COL-57).
        assert!(
            !b.has_ctrl_alt(),
            "macOS {action:?} = {}",
            b.format(Os::MacOs)
        );
    }
    // D66 gives Windows Ctrl+Win+Alt chords. Ctrl+Alt without Win is AltGr, so no default has it.
    let win = default_bindings(Os::Windows);
    for action in HotkeyAction::ALL {
        let b = win
            .get(action)
            .unwrap_or_else(|| panic!("Windows has no {action:?}"));
        let text = b.format(Os::Windows);
        assert!(
            !b.has_ctrl_alt() || text.contains("Win"),
            "Windows {action:?} = {text}"
        );
    }
    assert_eq!(
        win.get(HotkeyAction::Panic).map(|b| b.format(Os::Windows)),
        Some("Ctrl+Win+Alt+K".to_owned())
    );
    for os in [Os::MacOs, Os::Windows] {
        let d = default_bindings(os);
        assert_eq!(
            d.get(HotkeyAction::Ptt),
            Some(Binding::Talk(TalkBinding::RightOption))
        );
        // The defaults of one OS never collide with each other or with the OS.
        assert_eq!(
            first_run_check(os, &KeyboardEnv::default()).conflicts,
            vec![]
        );
    }
}

#[test]
fn at_30_cap_43b_altgr_and_copilot_layouts_move_the_talk_key_down_the_ladder() {
    let altgr = KeyboardEnv {
        alt_gr_layout: true,
        ..KeyboardEnv::default()
    };
    let r = first_run_check(Os::Windows, &altgr);
    assert_eq!(
        r.conflicts.iter().map(|c| c.kind).collect::<Vec<_>>(),
        vec![ConflictKind::AltGr]
    );
    assert_eq!(
        r.suggested.get(HotkeyAction::Ptt),
        Some(Binding::Talk(TalkBinding::RightControl))
    );

    let both = KeyboardEnv {
        alt_gr_layout: true,
        copilot_key: true,
        ..KeyboardEnv::default()
    };
    let r = first_run_check(Os::Windows, &both);
    assert_eq!(
        r.suggested.get(HotkeyAction::Ptt),
        Some(Binding::Talk(TalkBinding::CtrlWin))
    );
    let mut env_with_rctrl = hotkey_bindings::default_bindings(Os::Windows);
    env_with_rctrl.set(HotkeyAction::Ptt, Binding::Talk(TalkBinding::RightControl));
    let c = hotkey_bindings::check_conflicts(Os::Windows, &env_with_rctrl, &both);
    assert!(
        c.iter().any(|c| c.kind == ConflictKind::CopilotKey),
        "{c:?}"
    );
}

#[test]
fn at_30_cap_43b_every_first_run_conflict_shows_with_a_fix() {
    let mac = KeyboardEnv {
        dictation: Some(DictationShortcut::ControlTwice),
        voice_over: true,
        voice_apps: vec![VoiceApp::Superwhisper],
        ..KeyboardEnv::default()
    };
    let mut bs = default_bindings(Os::MacOs);
    bs.set(HotkeyAction::Ptt, Binding::Talk(TalkBinding::RightControl));
    let c = hotkey_bindings::check_conflicts(Os::MacOs, &bs, &mac);
    let kinds: Vec<_> = c.iter().map(|c| c.kind).collect();
    assert!(kinds.contains(&ConflictKind::Dictation), "{kinds:?}");
    assert!(kinds.contains(&ConflictKind::VoiceApp), "{kinds:?}");
    let vo = KeyboardEnv {
        voice_over: true,
        ..KeyboardEnv::default()
    };
    let kinds: Vec<_> = hotkey_bindings::check_conflicts(Os::MacOs, &bs, &vo)
        .iter()
        .map(|c| c.kind)
        .collect();
    assert_eq!(kinds, vec![ConflictKind::VoiceOver]);
    let fn_globe = Binding::parse(Os::MacOs, "Fn+K").expect("parse");
    let mut bs = default_bindings(Os::MacOs);
    bs.set(HotkeyAction::Session, fn_globe);
    let kinds: Vec<_> = hotkey_bindings::check_conflicts(Os::MacOs, &bs, &KeyboardEnv::default())
        .iter()
        .map(|c| c.kind)
        .collect();
    assert_eq!(kinds, vec![ConflictKind::FnGlobe]);

    for os in [Os::MacOs, Os::Windows] {
        let env = KeyboardEnv {
            dictation: Some(DictationShortcut::ControlTwice),
            voice_over: true,
            alt_gr_layout: true,
            copilot_key: true,
            voice_apps: VoiceApp::ALL.to_vec(),
        };
        let r = first_run_check(os, &env);
        assert!(!r.conflicts.is_empty(), "{os:?}");
        for c in &r.conflicts {
            let text = c.message(os);
            assert!(text.contains("Fix:"), "{os:?} {:?}: {text}", c.kind);
            assert!(matches!(
                c.fix,
                Fix::UseBinding { .. }
                    | Fix::OpenSystemSettings { .. }
                    | Fix::ChangeOtherApp { .. }
            ));
        }
    }
}

struct LayoutReads {
    alt_gr: bool,
    copilot: bool,
}

impl EnvReads for LayoutReads {
    fn dictation(&self) -> Option<DictationShortcut> {
        None
    }
    fn voice_over(&self) -> bool {
        false
    }
    fn alt_gr_layout(&self) -> bool {
        self.alt_gr
    }
    fn copilot_key(&self) -> bool {
        self.copilot
    }
    fn visit_process_names(&self, visit: &mut dyn FnMut(&str)) {
        visit("explorer.exe");
        visit("Dragon.exe");
    }
}

#[test]
fn at_30_cap_43b_the_os_reads_feed_the_check_and_keep_only_known_voice_apps() {
    let env = read_keyboard_env(
        Os::Windows,
        &LayoutReads {
            alt_gr: true,
            copilot: true,
        },
    );
    assert!(env.alt_gr_layout && env.copilot_key);
    assert_eq!(
        env.voice_apps,
        vec![VoiceApp::Dragon],
        "no other process name is kept"
    );
    let kinds: Vec<_> = first_run_check(Os::Windows, &env)
        .conflicts
        .iter()
        .map(|c| c.kind)
        .collect();
    assert!(kinds.contains(&ConflictKind::AltGr) && kinds.contains(&ConflictKind::VoiceApp));
}

#[test]
fn at_30_cap_43b_the_hook_sees_the_copilot_key_and_the_check_flags_right_ctrl() {
    // The Copilot key sends Win+Shift+F23; the hook watch sets the flag once (#332, ADR 0025).
    let (tx, rx) = mpsc::channel();
    let clock: Arc<dyn MonotonicClock> = Arc::new(SimClock::new());
    let mut core = HookCore::new(
        WinTalkKey::RightAlt,
        Arc::new(AtomicBool::new(false)),
        Arc::new(LatencyRecorder::with_capacity(16)),
        clock,
        tx,
    );
    for ev in [
        KeyEvent::down(vk::LWIN),
        KeyEvent::down(vk::LSHIFT),
        KeyEvent::down(vk::F23),
        KeyEvent::up(vk::F23),
        KeyEvent::up(vk::LSHIFT),
        KeyEvent::up(vk::LWIN),
    ] {
        assert_eq!(core.handle(ev), Verdict::Pass, "Copilot still opens");
    }
    let seen: Vec<_> = rx.try_iter().collect();
    assert_eq!(seen, vec![HookSignal::CopilotKeySeen]);

    let flag: Arc<dyn CopilotKeyFlag> = Arc::new(MemoryCopilotKeyFlag::default());
    assert!(flag.set(true));
    let reads = WithCopilotFlag::new(
        Box::new(LayoutReads {
            alt_gr: false,
            copilot: false,
        }),
        flag,
    );
    let env = read_keyboard_env(Os::Windows, &reads);
    assert!(env.copilot_key);
    let mut bs = default_bindings(Os::Windows);
    bs.set(HotkeyAction::Ptt, Binding::Talk(TalkBinding::RightControl));
    let c = hotkey_bindings::check_conflicts(Os::Windows, &bs, &env);
    let copilot = c
        .iter()
        .find(|c| c.kind == ConflictKind::CopilotKey)
        .expect("conflict");
    assert!(copilot.message(Os::Windows).contains("Fix:"));
}

// ---------- Secure Input evidence of the device run (CAP-39a, CAP-39b) ----------

const SEI_LOG: &str = "\
[t=      1.000 ms] src=sei event=initial secure_input=0
EV src=tap kind=down t_ns=1000000 lat_ms=0.4
[t=    500.000 ms] src=sei event=on poll_ms=10
[t=    500.010 ms] src=sei event=ptt_end_by_sei detect_to_end_ms=0.010 upper_bound_sei_to_end_ms=10.010
EV src=carbon kind=down t_ns=900000000 lat_ms=0.2
EV src=carbon kind=up t_ns=1700000000 lat_ms=0.3
EV src=tap kind=up t_ns=1800000000 lat_ms=0.4
[t=   2000.000 ms] src=sei event=off right_option_physically_down=0
";

#[test]
fn at_30_cap_39a_cap_39b_the_sp03_secure_input_log_reads_the_end_time_and_chord_presses() {
    let s = parse_sp03_secure_input(SEI_LOG);
    assert_eq!(s.secure_input_starts, 1);
    assert_eq!(s.sei_to_end, vec![Duration::from_micros(10_010)]);
    assert_eq!((s.chord_presses, s.chord_releases_seen), (1, 1));
    // The SP-03 probe is not the shipped HotkeyService, so its best result is partial (#323).
    assert_eq!(s.verdict(), SecureInputVerdict::WithinProbeOnly);
    let shipped = SecureInputSamples {
        tool: Some(KeySampleTool::HotkeyService),
        ..s.clone()
    };
    assert_eq!(shipped.verdict(), SecureInputVerdict::Within);

    let slow = SEI_LOG.replace(
        "upper_bound_sei_to_end_ms=10.010",
        "upper_bound_sei_to_end_ms=251.000",
    );
    assert_eq!(
        parse_sp03_secure_input(&slow).verdict(),
        SecureInputVerdict::Over {
            worst: Duration::from_millis(251)
        }
    );
    let lost_release = SEI_LOG.replace("EV src=carbon kind=up t_ns=1700000000 lat_ms=0.3\n", "");
    assert_eq!(
        parse_sp03_secure_input(&lost_release).verdict(),
        SecureInputVerdict::ChordReleaseMissed {
            presses: 1,
            releases: 0
        }
    );
    assert_eq!(
        parse_sp03_secure_input("").verdict(),
        SecureInputVerdict::NoSamples
    );
}

/// The env variable with the path of the SP-03 probe log of the Secure Input run.
const ENV_SEI_LOG: &str = "BOTTLE_BENCH_SP03_SEI_LOG";

#[test]
fn at_30_cap_39a_cap_39b_device_secure_keyboard_entry_during_a_hold() {
    let device = match testkit::device_from(|k| std::env::var(k).ok()) {
        Ok(Fixture::Ready(d)) => d,
        Ok(Fixture::Skip(reason)) => {
            eprintln!("{reason}");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    let Some(path) = std::env::var(ENV_SEI_LOG)
        .ok()
        .filter(|v| !v.trim().is_empty())
    else {
        eprintln!(
            "skipped: {ENV_SEI_LOG} is not set, so the AT-30 Secure Input run does not run on {} \
             (docs/tests/AT-MOGS-TALKKEY-A11Y.md)",
            device.id()
        );
        return;
    };
    let text = std::fs::read_to_string(path.trim()).expect("read the SP-03 Secure Input log");
    let s = parse_sp03_secure_input(&text);
    // A probe log gives at most a partial result: TODO(#323) for the HotkeyService log.
    assert_eq!(s.verdict(), SecureInputVerdict::WithinProbeOnly, "{s:?}");
}

/// The env variable that tells the layout test which keyboard the person set up:
/// `altgr` or `plain`. The Copilot key is not a layout read: the hook sees it (ADR 0025).
const ENV_LAYOUT: &str = "BOTTLE_TEST_LAYOUT";

#[test]
fn at_30_cap_43b_device_the_altgr_layout_is_read_and_flagged() {
    let device = match testkit::device_from(|k| std::env::var(k).ok()) {
        Ok(Fixture::Ready(d)) => d,
        Ok(Fixture::Skip(reason)) => {
            eprintln!("{reason}");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    let Some(layout) = std::env::var(ENV_LAYOUT).ok().map(|v| v.trim().to_owned()) else {
        eprintln!(
            "skipped: {ENV_LAYOUT} is not set, so the AT-30 layout run does not run on {} \
             (docs/tests/AT-MOGS-TALKKEY-A11Y.md)",
            device.id()
        );
        return;
    };
    let alt_gr = match layout.as_str() {
        "altgr" => true,
        "plain" => false,
        other => panic!("{ENV_LAYOUT} must be altgr or plain, not {other}"),
    };
    let os = mog_core::keyboard_env::host_os();
    assert_eq!(os, Os::Windows, "the layout run is a Windows step");
    // The real prompt-free OS reads of #322.
    let env = read_keyboard_env(os, mog_core::keyboard_env::os_env_reads().as_ref());
    assert_eq!(env.alt_gr_layout, alt_gr, "{layout}");
    let r = first_run_check(os, &env);
    let kinds: Vec<_> = r.conflicts.iter().map(|c| c.kind).collect();
    assert_eq!(kinds.contains(&ConflictKind::AltGr), alt_gr, "{kinds:?}");
    if alt_gr {
        // Right-Alt types characters on an AltGr layout, so the suggested talk key moves down.
        let talk = r.suggested.get(HotkeyAction::Ptt);
        assert_ne!(talk, Some(Binding::Talk(TalkBinding::RightOption)));
    }
}

// ---------- Pending: panic key (MB-05, #66) ----------

#[test]
#[ignore = "pending TODO(#66): the panic key and capture_stop_all are not built"]
fn at_30_mb_05_panic_stops_all_capture_within_200_ms_without_input_monitoring() {
    use mog_core::capture::{CaptureCore, CaptureRequest, CaptureService};
    use mog_core::testkit::capture::{FakeAudioDevice, RecordingCaptureEvents};
    use mog_core::types::CaptureMode;
    let request = |mog: &str, mode| CaptureRequest {
        mog_id: mog.to_string(),
        mode,
        others_track: false,
    };
    // The panic key is a registered chord, so it must work with Input Monitoring denied.
    let api = FakePermissions::new();
    api.set(Permission::InputMonitoring, PermissionStatus::Denied);
    assert_eq!(
        default_bindings(Os::MacOs)
            .get(HotkeyAction::Panic)
            .map(|b| b.format(Os::MacOs)),
        Some("Cmd+Shift+Esc".to_owned())
    );
    // TODO(#66): replace the stub with R2 and the panic handler, press the panic key on the
    // sim clock and read capture_status 200 ms later: no lease is left on any MOG.
    let capture = CaptureCore::new(
        Box::new(FakeAudioDevice::new()),
        Arc::new(SimClock::new()),
        Arc::new(RecordingCaptureEvents::default()),
    );
    // One lease per MOG: a PTT on MOG B over MOG A's hands-free capture (ARC-F9).
    let a = capture
        .start(request("mog-a", CaptureMode::HandsFree))
        .expect("TODO(#4): lease for MOG A")
        .id;
    let b = capture
        .start(request("mog-b", CaptureMode::Ptt))
        .expect("TODO(#4): lease for MOG B")
        .id;
    assert_ne!(a, b);
    assert!(
        check_ptt_permission(&api).is_err(),
        "PTT stays off without Input Monitoring"
    );
    todo!("TODO(#66): press the panic key, advance 200 ms, assert capture_status shows no lease on MOG A or B (p95 over 100 presses)");
}
