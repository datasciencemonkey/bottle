//! AT-03 "Talk key and permissions" (#230), section 15.
//!
//! The AT script: hold and release the talk key; repeat with Esc, focus loss, a tap < 250 ms,
//! no key-up for 120 s, denied mic permission, click-to-listen and drag.
//!
//! The talk-key rules of R1 ship (#6, #7, #24), so the hold, the tap and the cancel rules run
//! here on the #229 simulated clock. The capture side (R2 CaptureService, #4, #14, #21, #22)
//! and the click rules (#83, #98) are not built. Their tests are written against
//! `mog_core::capture::CaptureService` and `mog_core::types` and stay `#[ignore]` with the
//! feature issue, so they never count as a pass. The drag part of AT-03 ("a drag starts no
//! capture") is in `src-tauri/tests/at_01_mogs.rs`. The device run is a person's step:
//! docs/tests/AT-MOGS-TALKKEY-A11Y.md.

use std::sync::Arc;
use std::time::Duration;

use mog_core::capture::{
    CaptureCore, CaptureRequest, CaptureService, HandsFreeStop, StopReason, DEFAULT_VAD_END_MS,
};
use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::hotkey_core::{
    HotkeyEngine, HotkeyOutput, KeyEdge, PttPhase, OTHER_KEY_CANCEL_WINDOW, TAP_MAX_HOLD,
};
use mog_core::hotkey_windows::{vk, HookFilter, HookSignal, KeyEvent, TalkKey, Verdict};
use mog_core::testkit::capture::{FakeAudioDevice, RecordingCaptureEvents};
use mog_core::testkit::hotkey::RecordingSink;
use mog_core::types::{CaptureMode, SessionPolicy};

/// R2 on the shared sim clock with the testkit fake device (#4).
fn r2(clock: &SimClock) -> CaptureCore {
    CaptureCore::new(
        Box::new(FakeAudioDevice::new()),
        Arc::new(clock.clone()),
        Arc::new(RecordingCaptureEvents::default()),
    )
}

fn capture(mode: CaptureMode) -> CaptureRequest {
    CaptureRequest {
        mog_id: MOG_A.to_string(),
        mode,
        others_track: false,
    }
}

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

fn ptt(phase: PttPhase) -> HotkeyOutput {
    HotkeyOutput::Ptt(phase)
}

fn engine() -> (HotkeyEngine, Arc<RecordingSink>, SimClock) {
    let clock = SimClock::new();
    let sink = RecordingSink::new();
    let e = HotkeyEngine::with_clock(sink.clone(), Arc::new(clock.clone()));
    (e, sink, clock)
}

fn hold(e: &mut HotkeyEngine, clock: &SimClock, d: Duration) {
    e.on_talk_key(KeyEdge::Down);
    clock.advance(d);
    e.on_talk_key(KeyEdge::Up);
}

// ---------- Hold and release; tap < 250 ms (CAP-44a) ----------

#[test]
fn at_03_cap_44a_hold_and_release_starts_and_ends_one_ptt() {
    let (mut e, sink, clock) = engine();
    hold(&mut e, &clock, ms(1_500));
    assert_eq!(sink.take(), vec![ptt(PttPhase::Down), ptt(PttPhase::Up)]);
    assert!(!e.machine().is_held());
}

#[test]
fn at_03_cap_44a_a_200_ms_hold_is_a_tap_and_its_ptt_is_cancelled() {
    let (mut e, sink, clock) = engine();
    hold(&mut e, &clock, ms(200));
    // Cancel comes before Tap, so the audio of the tap is dropped and no STT call starts.
    assert_eq!(
        sink.take(),
        vec![
            ptt(PttPhase::Down),
            ptt(PttPhase::Cancel),
            HotkeyOutput::Tap
        ]
    );

    // The rule edge: 250 ms or more is a PTT.
    hold(&mut e, &clock, TAP_MAX_HOLD);
    assert_eq!(sink.take(), vec![ptt(PttPhase::Down), ptt(PttPhase::Up)]);
    hold(&mut e, &clock, TAP_MAX_HOLD - ms(1));
    assert_eq!(sink.take().last(), Some(&HotkeyOutput::Tap));
}

// ---------- Another key in the first 1 s (CAP-44c) ----------

#[test]
fn at_03_cap_44c_another_key_in_the_first_second_cancels_the_ptt() {
    let (mut e, sink, clock) = engine();
    e.on_talk_key(KeyEdge::Down);
    clock.advance(ms(600));
    assert!(e.other_key_window_open());
    e.on_other_key_down();
    // The later key-up gives nothing: the cancelled PTT makes no STT call.
    e.on_talk_key(KeyEdge::Up);
    assert_eq!(
        sink.take(),
        vec![ptt(PttPhase::Down), ptt(PttPhase::Cancel)]
    );

    // After the first second, another key is ignored.
    e.on_talk_key(KeyEdge::Down);
    clock.advance(OTHER_KEY_CANCEL_WINDOW);
    assert!(!e.other_key_window_open());
    e.on_other_key_down();
    e.on_talk_key(KeyEdge::Up);
    assert_eq!(sink.take(), vec![ptt(PttPhase::Down), ptt(PttPhase::Up)]);
}

#[test]
fn at_03_cap_44c_windows_hook_reports_another_key_during_a_hold() {
    let (mut e, sink, clock) = engine();
    let mut f = HookFilter::new(TalkKey::RightAlt);
    let (_, down) = f.on_key(KeyEvent::down(vk::RMENU), clock.now());
    let (verdict, other) = f.on_key(KeyEvent::down(u32::from(b'A')), clock.now());
    // The other key reaches the app; R1 learns only that "a key" went down.
    assert_eq!(
        (verdict, other),
        (Verdict::Pass, Some(HookSignal::OtherKeyDown))
    );
    for s in [down, other].into_iter().flatten() {
        mog_core::hotkey_windows::forward_to_engine(s, &mut e);
    }
    assert_eq!(
        sink.take(),
        vec![ptt(PttPhase::Down), ptt(PttPhase::Cancel)]
    );
}

// ---------- Esc (Windows hook part of CAP-14b) ----------

#[test]
fn at_03_windows_hook_swallows_esc_and_reports_it_during_a_hold_and_hands_free() {
    for hands_free in [false, true] {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        f.set_hands_free(hands_free);
        if !hands_free {
            f.on_key(KeyEvent::down(vk::RMENU), ms(0));
        }
        assert_eq!(
            f.on_key(KeyEvent::down(vk::ESCAPE), ms(10)),
            (Verdict::Swallow, Some(HookSignal::Escape)),
            "hands_free={hands_free}"
        );
    }
}

// ---------- Pending: the capture side (R2) ----------

const MOG_A: &str = "7b0c1f3e-0000-4000-8000-00000000000a";

fn policy() -> SessionPolicy {
    // The Appendix F defaults (SessionPolicy in section 4.7).
    SessionPolicy {
        check_in_at_min: 55,
        soft_cap_min: 60,
        extend_min: 30,
        silence_nudge_min: 5,
        silence_stop_min: 15,
        hard_cap_min: 240,
        lock_stop_min: 10,
        hold_max_sec: 120,
        hands_free_max_min: 5,
    }
}

#[test]
#[ignore = "pending TODO(#4): R2 leases and the 120 s / 5 min caps are not built"]
fn at_03_cap_04a_cap_04b_a_missing_key_up_stops_after_two_minutes_and_hands_free_after_five() {
    let clock = SimClock::new();
    let p = policy();
    assert_eq!(
        Duration::from_secs(u64::from(p.hold_max_sec)),
        Duration::from_secs(120)
    );
    assert_eq!(p.hands_free_max_min, 5);
    // TODO(#4): R2 with `clock`: capture_start {mode: ptt}, no key-up, advance 120 s ± 1 s and
    // read capture_status; then the same with hands_free and 5 min.
    let r2 = r2(&clock);
    let lease = r2
        .start(capture(CaptureMode::Ptt))
        .expect("TODO(#4): a PTT lease");
    clock.advance(Duration::from_secs(121));
    assert!(
        r2.stop(lease.id).is_err(),
        "R2 ended the lease at the 120 s cap, so a late release is refused"
    );
    todo!("TODO(#4): read capture_status at 119 s (recording) and 121 s (idle); repeat for hands-free at 5 min ± 1 s");
}

#[test]
#[ignore = "pending TODO(#21): no pre-roll and the mic close on pause are not built"]
fn at_03_cap_13a_cap_13b_no_audio_before_key_down_and_the_mic_closes_on_pause() {
    // TODO(#21): R2 has no frame before capture start, and capture_pause closes the stream.
    let r2 = r2(&SimClock::new());
    let lease = r2
        .start(capture(CaptureMode::Ptt))
        .expect("TODO(#21): a lease");
    r2.stop(lease.id)
        .expect("TODO(#21): the mic closes when no lease is left");
    todo!("TODO(#21): assert no frame exists before the lease and the OS mic is closed after capture_pause");
}

#[test]
fn at_03_cap_14a_cap_14b_cap_14c_cap_44b_hands_free_starts_and_stops_and_tap_audio_is_dropped() {
    let (mut e, sink, clock) = engine();
    // A double-tap of 400 ms or less gives the `double_tap` phase (CAP-14a).
    hold(&mut e, &clock, ms(100));
    clock.advance(ms(200));
    hold(&mut e, &clock, ms(100));
    let out = sink.take();
    assert_eq!(out.iter().filter(|o| **o == HotkeyOutput::Tap).count(), 2);
    assert_eq!(out.last(), Some(&ptt(PttPhase::DoubleTap)));

    // R2: the tap leases are discarded, so no tap audio reaches the pipeline (CAP-44b).
    let events = Arc::new(RecordingCaptureEvents::default());
    let device = FakeAudioDevice::new();
    let r2 = CaptureCore::new(
        Box::new(device.clone()),
        Arc::new(clock.clone()),
        events.clone(),
    );
    let tap = r2.start(capture(CaptureMode::Ptt)).expect("a tap lease");
    r2.discard(tap.id).expect("the tap lease stops");
    assert!(events.last_stop().expect("a stop").reason.discards_audio());

    // The double-tap starts hands-free; each of the three stops closes the mic at once
    // (CAP-14b, 200 ms or less).
    r2.start(capture(CaptureMode::HandsFree))
        .expect("a hands-free lease");
    r2.stop_hands_free(HandsFreeStop::Tap);
    assert!(!device.log().mic_open);
    assert_eq!(events.last_stop().unwrap().reason, StopReason::Tap);

    r2.start(capture(CaptureMode::HandsFree)).unwrap();
    r2.stop_hands_free(HandsFreeStop::Escape);
    assert!(!device.log().mic_open);
    assert_eq!(events.last_stop().unwrap().reason, StopReason::Escape);

    r2.start(capture(CaptureMode::HandsFree)).unwrap();
    let cap = r2.tick().expect("the 5 min cap deadline");
    clock.advance(Duration::from_millis(cap) - clock.now());
    r2.tick();
    assert!(!device.log().mic_open);
    assert_eq!(events.last_stop().unwrap().reason, StopReason::HandsFreeCap);

    // The VAD end: 1,500 ms of silence by default (CAP-14c).
    r2.start(capture(CaptureMode::HandsFree)).unwrap();
    r2.voice_activity(true);
    let end = r2.voice_activity(false).expect("the VAD end deadline");
    assert_eq!(
        end,
        u64::try_from(clock.now().as_millis()).unwrap() + DEFAULT_VAD_END_MS
    );
    clock.advance(ms(DEFAULT_VAD_END_MS));
    r2.tick();
    assert_eq!(events.last_stop().unwrap().reason, StopReason::VadEnd);
}

#[test]
#[ignore = "pending TODO(#22): mic permission after a user action and the fix-it card are not built"]
fn at_03_cap_17a_cap_17b_cap_22_denied_mic_makes_no_transcript() {
    // TODO(#22): no prompt at launch; 2 s of zero buffers shows the fix-it card and no
    // capture.segment is sent; the hub shows "Prefer the built-in mic ...".
    let r2 = r2(&SimClock::new());
    let lease = r2
        .start(capture(CaptureMode::Ptt))
        .expect("TODO(#22): a lease with the mic denied");
    r2.stop(lease.id).expect("release");
    todo!("TODO(#22): assert no prompt at launch, the fix-it card after 2 s of zero buffers and no capture.segment");
}

#[test]
#[ignore = "pending TODO(#83): click-to-listen and the mascot click rules are not built (MOG-09 #98)"]
fn at_03_mog_09_mog_12a_mog_12b_click_to_listen_never_starts_a_meeting_and_idle_has_no_mic() {
    // TODO(#83): a click (< 4 px, < 300 ms) toggles hands-free; 100 clicks start no session.
    // TODO(#98): 8 idle MOGs for 1 h hold no lease.
    let r2 = r2(&SimClock::new());
    for _ in 0..100 {
        let lease = r2
            .start(capture(CaptureMode::HandsFree))
            .expect("TODO(#83): a hands-free lease");
        r2.stop(lease.id).expect("release");
    }
    todo!("TODO(#83): assert 100 mascot clicks toggle hands-free and start no session; TODO(#98): 8 idle MOGs hold no lease for 1 h");
}
