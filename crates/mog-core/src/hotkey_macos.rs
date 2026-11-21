// Adapted from cjpais/Handy src-tauri/src/secure_input.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! R1 HotkeyService on macOS (#6): listen-only CGEventTap, watchdog, Secure Event Input.
//!
//! The pure part of this file (tap spec, key map, flag decode) compiles on every OS so its
//! tests run everywhere. The OS calls are in [`sys`], built only on macOS.
//!
//! Privacy: the tap listens to `FlagsChanged` only. It never sees `KeyDown` or `KeyUp` of
//! typed text, so no key contents reach Bottle, a log or telemetry (ADR 0011). The other-key
//! tap of CAP-44c compares the key code with G only and keeps no key identity (ADR 0098).
//!
//! Ask key (RTR-17a, ADR 0098): during a talk-key hold a Carbon hot key grabs the talk-key
//! modifier + G. The OS gives a registered hot key to Bottle only, so the focused app gets no
//! "g" or "©". The tap stays listen-only (CAP-35a).
//!
//! Record key (ADR 0103): with the Call recording knob on, the same hold also grabs the
//! talk-key modifier + R, so the app gets no "r" or "®". With the knob off, R is not grabbed.
//!
//! Requirements: CAP-35a, CAP-35b, CAP-36, CAP-37, CAP-38a, CAP-38b, CAP-39a, CAP-39b,
//! CAP-44c, CAP-45, RTR-17a. Real-device proof (AT-30) waits for SP-03: TODO(#5).

use crate::hotkey_core::{HotkeyEngine, KeyEdge, PhysicalState, TalkKey};

/// `kCGSessionEventTap`: the tap sees the events of the login session.
pub const CG_SESSION_EVENT_TAP: u32 = 1;
/// `kCGHeadInsertEventTap`.
pub const CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
/// `kCGEventTapOptionDefault`: an active tap that can change events (needs Accessibility).
pub const CG_EVENT_TAP_OPTION_DEFAULT: u32 = 0;
/// `kCGEventTapOptionListenOnly`: a passive tap (needs Input Monitoring only).
pub const CG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 1;
/// `kCGEventFlagsChanged`.
pub const CG_EVENT_FLAGS_CHANGED: u32 = 12;
/// `kCGEventKeyDown`.
pub const CG_EVENT_KEY_DOWN: u32 = 10;
/// `kCGEventKeyUp`.
pub const CG_EVENT_KEY_UP: u32 = 11;
/// `kCGEventTapDisabledByTimeout`.
pub const CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
/// `kCGEventTapDisabledByUserInput`.
pub const CG_EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

/// `kVK_Space`, the key of the fallback chord.
pub const VK_SPACE: u16 = 49;
/// `kVK_ANSI_G`, the key of the ask key (RTR-17a, ADR 0098).
pub const VK_ANSI_G: u16 = 0x05;
/// Carbon `cmdKey`.
pub const CARBON_CMD_KEY: u32 = 0x0100;
/// Carbon `shiftKey`.
pub const CARBON_SHIFT_KEY: u32 = 0x0200;
/// Carbon `optionKey`.
pub const CARBON_OPTION_KEY: u32 = 0x0800;
/// Carbon `controlKey`.
pub const CARBON_CONTROL_KEY: u32 = 0x1000;
/// The Carbon hot-key ID of the fallback chord. Each handler takes only its own ID.
pub const CHORD_HOT_KEY_ID: u32 = 1;
/// The Carbon hot-key ID of the ask key.
pub const ASK_HOT_KEY_ID: u32 = 2;
/// `kVK_ANSI_R`, the key of the record key (ADR 0103).
pub const VK_ANSI_R: u16 = 0x0F;
/// The Carbon hot-key ID of the record key. Its modifier is [`ask_key_modifiers`]: ⌥R.
pub const RECORD_HOT_KEY_ID: u32 = 3;

/// The Carbon modifier of the ask key: the generic modifier of the talk key, so "hold the talk
/// key and tap G" works for each talk key. The default, Right-Option, gives Option+G (⌥G).
pub fn ask_key_modifiers(talk: TalkKey) -> u32 {
    match talk {
        TalkKey::RightOption => CARBON_OPTION_KEY,
        TalkKey::RightControl => CARBON_CONTROL_KEY,
        TalkKey::RightCommand => CARBON_CMD_KEY,
        TalkKey::RightShift => CARBON_SHIFT_KEY,
    }
}
/// `kCGEventFlagMaskControl`.
pub const CG_FLAG_CONTROL: u64 = 0x0004_0000;
/// `kCGEventFlagMaskShift`.
pub const CG_FLAG_SHIFT: u64 = 0x0002_0000;

/// How the tap is created. [`sys`] passes these fields to `CGEventTapCreate` unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TapSpec {
    /// `CGEventTapLocation`.
    pub location: u32,
    /// `CGEventTapPlacement`.
    pub placement: u32,
    /// `CGEventTapOptions`.
    pub options: u32,
    /// `CGEventMask`.
    pub events_of_interest: u64,
}

/// The one tap spec of the build (CAP-35a).
pub const TAP_SPEC: TapSpec = TapSpec {
    location: CG_SESSION_EVENT_TAP,
    placement: CG_HEAD_INSERT_EVENT_TAP,
    options: CG_EVENT_TAP_OPTION_LISTEN_ONLY,
    events_of_interest: 1 << CG_EVENT_FLAGS_CHANGED,
};

/// The second listen-only tap of CAP-44c (ADR 0017). It asks for `KeyDown` only, never
/// `KeyUp`. It is created off and the OS layer turns it on only while
/// [`crate::hotkey_core::HotkeyEngine::other_key_window_open`] is true: the first 1 s of a
/// talk-key hold. Its callback reads one field, the key code, and only compares it with G
/// (ADR 0098) and R (ADR 0103). It never stores or logs the code.
pub const OTHER_KEY_TAP_SPEC: TapSpec = TapSpec {
    location: CG_SESSION_EVENT_TAP,
    placement: CG_HEAD_INSERT_EVENT_TAP,
    options: CG_EVENT_TAP_OPTION_LISTEN_ONLY,
    events_of_interest: 1 << CG_EVENT_KEY_DOWN,
};

/// A key went down during the first 1 s of a hold. It has no field, so no key identity can
/// cross the callback boundary (CAP-44c, SEC rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtherKeyDown {
    /// G: the ask key (RTR-17a). It marks the ask and does not cancel (ADR 0098).
    Ask,
    /// R: the record key (ADR 0103). The engine decides: with the Call recording knob on it
    /// marks the record toggle; with the knob off it is any other key.
    Record,
    /// Any other key: it cancels the PTT (CAP-44c).
    Other,
}

/// Classify an event of the other-key tap. `keycode` is only compared with G and R; it is not
/// kept.
pub fn other_key_from_event(event_type: u32, keycode: i64) -> Option<OtherKeyDown> {
    (event_type == CG_EVENT_KEY_DOWN).then_some(if keycode == i64::from(VK_ANSI_G) {
        OtherKeyDown::Ask
    } else if keycode == i64::from(VK_ANSI_R) {
        OtherKeyDown::Record
    } else {
        OtherKeyDown::Other
    })
}

/// The flag bit that is set while the modifier key `keycode` is down, or `None` for a key
/// that is not a known modifier. Device-dependent bits from IOLLEvent.h tell the left and
/// right keys apart; Fn and Caps Lock use their generic bits.
fn modifier_down_bit(keycode: u16) -> Option<u64> {
    Some(match keycode {
        56 => 0x02,        // Left-Shift
        60 => 0x04,        // Right-Shift
        59 => 0x01,        // Left-Control
        62 => 0x2000,      // Right-Control
        58 => 0x20,        // Left-Option
        61 => 0x40,        // Right-Option
        55 => 0x08,        // Left-Command
        54 => 0x10,        // Right-Command
        63 => 0x0080_0000, // Fn / Globe (kCGEventFlagMaskSecondaryFn)
        _ => return None,
    })
}

/// `kVK_CapsLock`. Its flag bit shows the lock state, not the key state, so each Caps Lock
/// event counts as a key-down.
const VK_CAPS_LOCK: u16 = 57;

/// What one `FlagsChanged` event means for R1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagsEvent {
    /// The talk key went down or up.
    Talk(KeyEdge),
    /// Another modifier key went down (CAP-44c). Its identity is dropped here.
    OtherModifierDown,
    /// Anything else (another modifier went up, an unknown key).
    Ignore,
}

/// Classify one `FlagsChanged` event for the talk key `key` (CAP-35b, CAP-44c).
pub fn classify_flags_changed(key: TalkKey, keycode: u16, flags: u64) -> FlagsEvent {
    if let Some(edge) = decode_flags_changed(key, keycode, flags) {
        return FlagsEvent::Talk(edge);
    }
    if keycode == VK_CAPS_LOCK {
        return FlagsEvent::OtherModifierDown;
    }
    match modifier_down_bit(keycode) {
        Some(bit) if flags & bit != 0 => FlagsEvent::OtherModifierDown,
        _ => FlagsEvent::Ignore,
    }
}

/// The virtual key code and the device-dependent flag bit of a talk key.
pub fn talk_key_code(key: TalkKey) -> (u16, u64) {
    // kVK_* codes from HIToolbox Events.h; NX_DEVICE*KEYMASK bits from IOLLEvent.h.
    match key {
        TalkKey::RightOption => (61, 0x40),
        TalkKey::RightControl => (62, 0x2000),
        TalkKey::RightCommand => (54, 0x10),
        TalkKey::RightShift => (60, 0x04),
    }
}

/// Decode one `FlagsChanged` event for `key`. Returns the edge, or `None` for another key.
pub fn decode_flags_changed(key: TalkKey, keycode: u16, flags: u64) -> Option<KeyEdge> {
    let (code, mask) = talk_key_code(key);
    if keycode != code {
        return None;
    }
    Some(if flags & mask != 0 {
        KeyEdge::Down
    } else {
        KeyEdge::Up
    })
}

/// Give one classified `FlagsChanged` event to the engine. The tap callback calls it with the
/// engine lock held. The talk-key edges also end the physical hold that keeps the ask and
/// record keys grabbed (#604), so a Right-Option up always reaches the engine.
pub fn apply_flags_event(engine: &mut HotkeyEngine, event: FlagsEvent) {
    match event {
        FlagsEvent::Talk(edge) => engine.on_talk_key(edge),
        FlagsEvent::OtherModifierDown => engine.on_other_key_down(),
        FlagsEvent::Ignore => {}
    }
}

/// Give one classified event of the other-key tap to the engine (CAP-44c, RTR-17a, ADR 0103).
/// The grabbed Carbon hot keys of G and R call [`HotkeyEngine::on_ask_key`] and
/// [`HotkeyEngine::on_record_key`] the same way.
pub fn apply_other_key(engine: &mut HotkeyEngine, key: OtherKeyDown) {
    match key {
        // RTR-17a: G marks the ask and does not cancel. This also covers a G that comes
        // before the main thread has registered the ask key.
        OtherKeyDown::Ask => engine.on_ask_key(),
        // ADR 0103: the engine checks the Call recording knob.
        OtherKeyDown::Record => engine.on_record_key(),
        OtherKeyDown::Other => engine.on_other_key_down(),
    }
}

/// The physical state from the HID key state of the talk key, the HID flag state and the
/// HID key state of Space (CAP-45). The talk key uses its own key state, not the
/// device-dependent flag bits, which a source-state read may not carry.
pub fn physical_from_hid(talk_key_down: bool, flags: u64, space_down: bool) -> PhysicalState {
    let chord_mods = CG_FLAG_CONTROL | CG_FLAG_SHIFT;
    PhysicalState {
        talk_key_down,
        chord_down: space_down && flags & chord_mods == chord_mods,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey_core::{HotkeyOutput, PttMachine, PttPhase};

    #[test]
    fn rtr_17a_the_ask_key_is_g_with_the_talk_key_modifier() {
        // kVK_ANSI_G = 0x05 (HIToolbox Events.h).
        assert_eq!(VK_ANSI_G, 0x05);
        // Carbon optionKey, cmdKey, shiftKey and controlKey (HIToolbox Events.h).
        assert_eq!(ask_key_modifiers(TalkKey::RightOption), 0x0800);
        assert_eq!(ask_key_modifiers(TalkKey::RightCommand), 0x0100);
        assert_eq!(ask_key_modifiers(TalkKey::RightShift), CARBON_SHIFT_KEY);
        assert_eq!(ask_key_modifiers(TalkKey::RightControl), CARBON_CONTROL_KEY);
    }

    #[test]
    fn rtr_17a_the_ask_hot_key_id_differs_from_the_chord_id() {
        assert_ne!(ASK_HOT_KEY_ID, CHORD_HOT_KEY_ID);
    }

    #[test]
    fn cap_35a_at_30_the_tap_is_listen_only() {
        assert_eq!(TAP_SPEC.options, CG_EVENT_TAP_OPTION_LISTEN_ONLY);
        assert_ne!(TAP_SPEC.options, CG_EVENT_TAP_OPTION_DEFAULT);
        assert_eq!(TAP_SPEC.location, CG_SESSION_EVENT_TAP);
    }

    #[test]
    fn cap_35a_the_tap_listens_to_flags_changed_only() {
        assert_eq!(TAP_SPEC.events_of_interest, 1u64 << CG_EVENT_FLAGS_CHANGED);
        assert_eq!(TAP_SPEC.events_of_interest & (1u64 << CG_EVENT_KEY_DOWN), 0);
        assert_eq!(TAP_SPEC.events_of_interest & (1u64 << CG_EVENT_KEY_UP), 0);
    }

    #[test]
    fn cap_35b_right_option_down_and_up_decode() {
        // Right-Option: kVK_RightOption = 61, NX_DEVICERALTKEYMASK = 0x40, plus the generic
        // Option mask 0x80000.
        assert_eq!(talk_key_code(TalkKey::RightOption), (61, 0x40));
        assert_eq!(
            decode_flags_changed(TalkKey::RightOption, 61, 0x0008_0140),
            Some(KeyEdge::Down)
        );
        assert_eq!(
            decode_flags_changed(TalkKey::RightOption, 61, 0x0000_0100),
            Some(KeyEdge::Up)
        );
    }

    #[test]
    fn cap_35b_right_option_up_while_left_option_is_held() {
        // Left-Option (0x20) is still down; the generic Option bit stays set.
        assert_eq!(
            decode_flags_changed(TalkKey::RightOption, 61, 0x0008_0120),
            Some(KeyEdge::Up)
        );
    }

    #[test]
    fn cap_35b_other_keys_are_ignored() {
        // Left-Option (58) and Left-Shift (56) are not the talk key.
        assert_eq!(
            decode_flags_changed(TalkKey::RightOption, 58, 0x0008_0120),
            None
        );
        assert_eq!(
            decode_flags_changed(TalkKey::RightOption, 56, 0x0002_0102),
            None
        );
    }

    #[test]
    fn cap_35b_every_modifier_talk_key_decodes() {
        for (key, code, mask) in [
            (TalkKey::RightOption, 61u16, 0x40u64),
            (TalkKey::RightControl, 62, 0x2000),
            (TalkKey::RightCommand, 54, 0x10),
            (TalkKey::RightShift, 60, 0x04),
        ] {
            assert_eq!(talk_key_code(key), (code, mask));
            assert_eq!(decode_flags_changed(key, code, mask), Some(KeyEdge::Down));
            assert_eq!(decode_flags_changed(key, code, 0), Some(KeyEdge::Up));
        }
    }

    #[test]
    fn cap_35b_at_30_1000_flag_cycles_give_1000_key_ups() {
        let mut m = PttMachine::new();
        let mut ups = 0;
        for _ in 0..1000 {
            for flags in [0x0008_0140u64, 0x0000_0100] {
                if let Some(edge) = decode_flags_changed(TalkKey::RightOption, 61, flags) {
                    ups += m
                        .on_talk_key(edge)
                        .iter()
                        .filter(|o| **o == HotkeyOutput::Ptt(PttPhase::Up))
                        .count();
                }
            }
        }
        assert_eq!(ups, 1000);
    }

    #[test]
    fn cap_44c_other_key_tap_is_listen_only_and_key_down_only() {
        assert_eq!(OTHER_KEY_TAP_SPEC.options, CG_EVENT_TAP_OPTION_LISTEN_ONLY);
        assert_eq!(OTHER_KEY_TAP_SPEC.location, CG_SESSION_EVENT_TAP);
        assert_eq!(
            OTHER_KEY_TAP_SPEC.events_of_interest,
            1u64 << CG_EVENT_KEY_DOWN
        );
        assert_eq!(
            OTHER_KEY_TAP_SPEC.events_of_interest & (1u64 << CG_EVENT_KEY_UP),
            0
        );
        // The talk-key tap keeps the ADR 0011 spec.
        assert_eq!(TAP_SPEC.events_of_interest, 1u64 << CG_EVENT_FLAGS_CHANGED);
    }

    #[test]
    fn cap_44c_other_key_events_carry_no_key_identity() {
        // The key code goes in only to be compared with G. A field-less marker comes out.
        assert!(std::mem::size_of::<OtherKeyDown>() <= 1);
        assert_eq!(
            other_key_from_event(CG_EVENT_KEY_DOWN, 0),
            Some(OtherKeyDown::Other)
        );
        assert_eq!(
            other_key_from_event(CG_EVENT_KEY_DOWN, 53),
            Some(OtherKeyDown::Other)
        );
        for t in [
            CG_EVENT_KEY_UP,
            CG_EVENT_FLAGS_CHANGED,
            CG_EVENT_TAP_DISABLED_BY_TIMEOUT,
            CG_EVENT_TAP_DISABLED_BY_USER_INPUT,
        ] {
            assert_eq!(other_key_from_event(t, 0), None, "type {t}");
            assert_eq!(
                other_key_from_event(t, i64::from(VK_ANSI_G)),
                None,
                "type {t}"
            );
        }
    }

    #[test]
    fn cap_44c_g_is_the_ask_key_and_not_an_other_key() {
        assert_eq!(
            other_key_from_event(CG_EVENT_KEY_DOWN, i64::from(VK_ANSI_G)),
            Some(OtherKeyDown::Ask)
        );
        // Only the G code: a code that is out of range is another key.
        assert_eq!(
            other_key_from_event(CG_EVENT_KEY_DOWN, i64::from(VK_ANSI_G) + 0x1_0000),
            Some(OtherKeyDown::Other)
        );
        assert_eq!(
            other_key_from_event(CG_EVENT_KEY_DOWN, -1),
            Some(OtherKeyDown::Other)
        );
    }

    #[test]
    fn rec_the_record_key_is_r_with_the_talk_key_modifier() {
        // kVK_ANSI_R = 0x0F (HIToolbox Events.h). Same modifier as the ask key: ⌥R by default.
        assert_eq!(VK_ANSI_R, 0x0F);
        assert_ne!(RECORD_HOT_KEY_ID, CHORD_HOT_KEY_ID);
        assert_ne!(RECORD_HOT_KEY_ID, ASK_HOT_KEY_ID);
    }

    #[test]
    fn rec_cap_44c_r_is_the_record_key_and_not_an_other_key() {
        assert_eq!(
            other_key_from_event(CG_EVENT_KEY_DOWN, i64::from(VK_ANSI_R)),
            Some(OtherKeyDown::Record)
        );
        assert_eq!(
            other_key_from_event(CG_EVENT_KEY_DOWN, i64::from(VK_ANSI_R) + 0x1_0000),
            Some(OtherKeyDown::Other)
        );
        assert_eq!(
            other_key_from_event(CG_EVENT_KEY_UP, i64::from(VK_ANSI_R)),
            None
        );
    }

    #[test]
    fn cap_44c_another_modifier_going_down_is_an_other_key() {
        // Right-Option is held (0x40); Left-Shift (56) goes down (0x02, generic 0x20000).
        assert_eq!(
            classify_flags_changed(TalkKey::RightOption, 56, 0x000_0002_0142),
            FlagsEvent::OtherModifierDown
        );
        // Left-Command (55, 0x08) goes down.
        assert_eq!(
            classify_flags_changed(TalkKey::RightOption, 55, 0x0010_0148),
            FlagsEvent::OtherModifierDown
        );
        // Caps Lock (57) counts whether it turns the lock on or off.
        for flags in [0x0001_0140u64, 0x0000_0140] {
            assert_eq!(
                classify_flags_changed(TalkKey::RightOption, 57, flags),
                FlagsEvent::OtherModifierDown
            );
        }
        // Fn/Globe (63) goes down: kCGEventFlagMaskSecondaryFn.
        assert_eq!(
            classify_flags_changed(TalkKey::RightOption, 63, 0x0080_0140),
            FlagsEvent::OtherModifierDown
        );
    }

    #[test]
    fn cap_44c_another_modifier_going_up_is_ignored() {
        // Left-Shift (56) goes up while Right-Option stays down.
        assert_eq!(
            classify_flags_changed(TalkKey::RightOption, 56, 0x0008_0140),
            FlagsEvent::Ignore
        );
        // An unknown key code is ignored.
        assert_eq!(
            classify_flags_changed(TalkKey::RightOption, 200, 0xFFFF_FFFF),
            FlagsEvent::Ignore
        );
    }

    #[test]
    fn cap_44c_talk_key_edges_still_classify_as_talk() {
        assert_eq!(
            classify_flags_changed(TalkKey::RightOption, 61, 0x0008_0140),
            FlagsEvent::Talk(KeyEdge::Down)
        );
        assert_eq!(
            classify_flags_changed(TalkKey::RightOption, 61, 0x0000_0100),
            FlagsEvent::Talk(KeyEdge::Up)
        );
    }

    #[test]
    fn cap_45_physical_state_uses_the_talk_key_state_not_device_bits() {
        // No device bits in the flags: the key state alone decides.
        assert!(physical_from_hid(true, 0, false).talk_key_down);
        assert!(!physical_from_hid(false, 0x0008_0140, false).talk_key_down);
        let chord = CG_FLAG_CONTROL | CG_FLAG_SHIFT;
        assert!(physical_from_hid(false, chord, true).chord_down);
        assert!(!physical_from_hid(false, chord, false).chord_down);
        assert!(!physical_from_hid(false, CG_FLAG_CONTROL, true).chord_down);
    }

    // ---- #604: ⌥G and ⌥R stay grabbed until Right-Option is physically up ----

    /// The key seam: it records each grab (`true`) and release (`false`) of ⌥G and ⌥R.
    #[derive(Default)]
    struct Grabs {
        ask: std::sync::Mutex<Vec<bool>>,
        record: std::sync::Mutex<Vec<bool>>,
    }
    impl crate::hotkey_core::AskKeyControl for Grabs {
        fn set_ask_key(&self, on: bool) {
            self.ask.lock().unwrap().push(on);
        }
        fn drain(&self) {}
        fn set_record_key(&self, on: bool) {
            self.record.lock().unwrap().push(on);
        }
    }
    impl Grabs {
        fn take(&self) -> (Vec<bool>, Vec<bool>) {
            (
                std::mem::take(&mut *self.ask.lock().unwrap()),
                std::mem::take(&mut *self.record.lock().unwrap()),
            )
        }
    }
    #[derive(Default)]
    struct Outs(std::sync::Mutex<Vec<HotkeyOutput>>);
    impl crate::hotkey_core::HotkeySink for Outs {
        fn emit(&self, o: HotkeyOutput) {
            self.0.lock().unwrap().push(o);
        }
    }

    /// The flags of the tap: Right-Option (61, bit 0x40) and Left-Option (58, bit 0x20), with
    /// the generic Option bit while either is down.
    fn flags(right: bool, left: bool) -> u64 {
        let mut f = 0x100;
        if right {
            f |= 0x0008_0040;
        }
        if left {
            f |= 0x0008_0020;
        }
        f
    }

    fn mac_engine() -> (
        std::sync::Arc<Outs>,
        crate::clock::SimClock,
        std::sync::Arc<Grabs>,
        HotkeyEngine,
    ) {
        use std::sync::Arc;
        let outs = Arc::new(Outs::default());
        let clock = crate::clock::SimClock::new();
        let grabs = Arc::new(Grabs::default());
        let e = HotkeyEngine::with_clock(outs.clone(), Arc::new(clock.clone()))
            .with_ask_key(grabs.clone())
            .with_record_gate(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        (outs, clock, grabs, e)
    }

    fn flags_event(e: &mut HotkeyEngine, keycode: u16, f: u64) {
        apply_flags_event(e, classify_flags_changed(TalkKey::RightOption, keycode, f));
    }

    #[test]
    fn issue_604_r_twice_in_one_right_option_hold_stays_grabbed_until_the_key_up() {
        let (outs, clock, grabs, mut e) = mac_engine();
        flags_event(&mut e, 61, flags(true, false));
        assert_eq!(grabs.take(), (vec![true], vec![true]));
        clock.advance(std::time::Duration::from_millis(400));
        // The first R comes from the other-key tap, the next ones from the Carbon hot key.
        apply_other_key(
            &mut e,
            other_key_from_event(CG_EVENT_KEY_DOWN, i64::from(VK_ANSI_R)).unwrap(),
        );
        for _ in 0..5 {
            clock.advance(std::time::Duration::from_millis(33));
            e.on_record_key();
            e.on_ask_key();
        }
        assert_eq!(grabs.take(), (vec![], vec![]), "⌥G and ⌥R stay grabbed");
        assert_eq!(
            std::mem::take(&mut *outs.0.lock().unwrap()),
            vec![
                HotkeyOutput::Ptt(PttPhase::Down),
                HotkeyOutput::Ptt(PttPhase::Cancel),
                HotkeyOutput::RecordToggle
            ]
        );
        // Right-Option up, with Left-Option still down: both grabs end, so a Left-Option + R
        // types "®" again.
        flags_event(&mut e, 58, flags(true, true));
        assert_eq!(grabs.take(), (vec![], vec![]));
        flags_event(&mut e, 61, flags(false, true));
        assert_eq!(grabs.take(), (vec![false], vec![false]));
        e.on_record_key();
        assert_eq!(grabs.take(), (vec![], vec![]));
        assert!(outs.0.lock().unwrap().is_empty());
    }

    #[test]
    fn issue_604_left_option_alone_never_grabs_r_or_g() {
        let (outs, _clock, grabs, mut e) = mac_engine();
        flags_event(&mut e, 58, flags(false, true));
        e.on_record_key();
        e.on_ask_key();
        flags_event(&mut e, 58, flags(false, false));
        assert_eq!(grabs.take(), (vec![], vec![]));
        assert!(outs.0.lock().unwrap().is_empty());
    }
}

#[cfg(target_os = "macos")]
pub use sys::{run_main_loop, MacHotkeyService, MacPermissions, MacProbe};

/// The macOS OS calls. Raw FFI to system frameworks only, no new crates (ADR 0011).
#[cfg(target_os = "macos")]
mod sys {
    use std::ffi::{c_char, c_void, CStr};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::ptr;
    use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
    use std::sync::{mpsc, Arc, Mutex, MutexGuard, PoisonError, Weak};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use super::*;
    use crate::clock::{MonotonicClock, SystemClock};
    use crate::error::{CoreError, CoreResult};
    use crate::hotkey::HotkeyService;
    use crate::hotkey_core::{
        check_ptt_permission, AskKeyControl, HotkeySink, Permission, PermissionApi,
        PermissionStatus, PlatformProbe, TapControl, TapSignal, WATCHDOG_TICK,
    };

    const ID: &str = "R1";

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFDictionaryRef = *const c_void;
    type CFMachPortRef = *mut c_void;
    type CFRunLoopRef = *mut c_void;
    type CFRunLoopSourceRef = *mut c_void;
    type CGEventRef = *mut c_void;
    type CGEventTapProxy = *mut c_void;
    type CGEventTapCallBack =
        extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;
    type EventHandlerCallRef = *mut c_void;
    type EventRef = *mut c_void;
    type EventTargetRef = *mut c_void;
    type EventHandlerRef = *mut c_void;
    type EventHotKeyRef = *mut c_void;
    type EventHandlerProc = extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> i32;

    #[repr(C)]
    struct CFDictionaryCallBacks {
        _opaque: [u8; 0],
    }

    #[repr(C)]
    struct DispatchQueue {
        _opaque: [u8; 0],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct EventTypeSpec {
        event_class: u32,
        event_kind: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct EventHotKeyId {
        signature: u32,
        id: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct MachTimebaseInfo {
        numer: u32,
        denom: u32,
    }

    /// `kCGKeyboardEventKeycode`.
    const CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;
    /// `kCGEventSourceStateHIDSystemState`.
    const CG_EVENT_SOURCE_STATE_HID: i32 = 1;
    /// `kIOHIDRequestTypeListenEvent`: the Input Monitoring permission.
    const IOHID_REQUEST_LISTEN_EVENT: u32 = 1;
    /// `kIOHIDAccessTypeGranted` / `Denied`.
    const IOHID_ACCESS_GRANTED: u32 = 0;
    const IOHID_ACCESS_DENIED: u32 = 1;
    /// `kCFStringEncodingUTF8`.
    const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    /// Carbon `kEventClassKeyboard` ('keyb'), `kEventHotKeyPressed`, `kEventHotKeyReleased`.
    const EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
    const EVENT_HOT_KEY_PRESSED: u32 = 5;
    const EVENT_HOT_KEY_RELEASED: u32 = 6;
    /// `eventNotHandledErr`.
    const EVENT_NOT_HANDLED_ERR: i32 = -9874;
    /// The signature of the Bottle hot keys ('BTLE').
    const HOT_KEY_SIGNATURE: u32 = u32::from_be_bytes(*b"BTLE");
    /// Carbon `kEventDurationNoWait`: a pull that does not wait.
    const EVENT_DURATION_NO_WAIT: f64 = 0.0;
    /// Carbon `kEventParamDirectObject` ('----') and `typeEventHotKeyID` ('hkid').
    const EVENT_PARAM_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
    const TYPE_EVENT_HOT_KEY_ID: u32 = u32::from_be_bytes(*b"hkid");

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFRunLoopCommonModes: CFStringRef;
        static kCFBooleanTrue: CFTypeRef;
        static kCFTypeDictionaryKeyCallBacks: CFDictionaryCallBacks;
        static kCFTypeDictionaryValueCallBacks: CFDictionaryCallBacks;
        fn CFRelease(cf: CFTypeRef);
        fn CFMachPortCreateRunLoopSource(
            allocator: *const c_void,
            port: CFMachPortRef,
            order: isize,
        ) -> CFRunLoopSourceRef;
        fn CFMachPortInvalidate(port: CFMachPortRef);
        fn CFRunLoopGetCurrent() -> CFRunLoopRef;
        fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
        fn CFRunLoopRun();
        fn CFRunLoopStop(rl: CFRunLoopRef);
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            count: isize,
            key_callbacks: *const CFDictionaryCallBacks,
            value_callbacks: *const CFDictionaryCallBacks,
        ) -> CFDictionaryRef;
        fn CFGetTypeID(cf: CFTypeRef) -> usize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(boolean: CFTypeRef) -> bool;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventTapCreate(
            tap: u32,
            place: u32,
            options: u32,
            events_of_interest: u64,
            callback: CGEventTapCallBack,
            user_info: *mut c_void,
        ) -> CFMachPortRef;
        fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
        fn CGEventTapIsEnabled(tap: CFMachPortRef) -> bool;
        fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
        fn CGEventGetFlags(event: CGEventRef) -> u64;
        fn CGEventSourceFlagsState(state: i32) -> u64;
        fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
        fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        static kAXTrustedCheckOptionPrompt: CFStringRef;
        // C `Boolean` is `unsigned char`.
        fn AXIsProcessTrusted() -> u8;
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
    }

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOHIDCheckAccess(request: u32) -> u32;
        fn IOHIDRequestAccess(request: u32) -> bool;
    }

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        // Carbon HIToolbox; Boolean is an unsigned char (as in Handy secure_input.rs).
        fn IsSecureEventInputEnabled() -> u8;
        fn GetApplicationEventTarget() -> EventTargetRef;
        fn InstallEventHandler(
            target: EventTargetRef,
            handler: EventHandlerProc,
            num_types: u32,
            list: *const EventTypeSpec,
            user_data: *mut c_void,
            out_ref: *mut EventHandlerRef,
        ) -> i32;
        fn RemoveEventHandler(handler: EventHandlerRef) -> i32;
        fn RegisterEventHotKey(
            key_code: u32,
            modifiers: u32,
            id: EventHotKeyId,
            target: EventTargetRef,
            options: u32,
            out_ref: *mut EventHotKeyRef,
        ) -> i32;
        fn UnregisterEventHotKey(hot_key: EventHotKeyRef) -> i32;
        fn GetEventKind(event: EventRef) -> u32;
        fn ReceiveNextEvent(
            num_types: usize,
            list: *const EventTypeSpec,
            timeout: f64,
            pull_event: u8,
            out_event: *mut EventRef,
        ) -> i32;
        fn SendEventToEventTarget(event: EventRef, target: EventTargetRef) -> i32;
        fn GetEventDispatcherTarget() -> EventTargetRef;
        fn ReleaseEvent(event: EventRef);
        fn GetEventParameter(
            event: EventRef,
            name: u32,
            desired_type: u32,
            actual_type: *mut u32,
            buffer_size: usize,
            actual_size: *mut usize,
            data: *mut c_void,
        ) -> i32;
        fn RunApplicationEventLoop();
    }

    extern "C" {
        static _dispatch_main_q: DispatchQueue;
        fn dispatch_async_f(
            queue: *const DispatchQueue,
            context: *mut c_void,
            work: extern "C" fn(*mut c_void),
        );
        fn mach_absolute_time() -> u64;
        fn mach_continuous_time() -> u64;
        fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
    }

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Run the Carbon application event loop on the main thread. Carbon sends the chord
    /// hot-key events through it, and it also runs the main dispatch queue. A Tauri app runs
    /// its own NSApplication loop instead; the device probe example calls this.
    pub fn run_main_loop() {
        // SAFETY: called on the main thread; it returns when the app quits.
        unsafe { RunApplicationEventLoop() }
    }

    /// The prompt-free OS reads of the watchdog (CAP-38b, CAP-39a, CAP-45).
    #[derive(Debug)]
    pub struct MacProbe {
        talk_key: TalkKey,
        numer: u64,
        denom: u64,
    }

    impl MacProbe {
        /// A probe for `talk_key`.
        pub fn new(talk_key: TalkKey) -> Self {
            let mut info = MachTimebaseInfo::default();
            // SAFETY: `info` is a valid out pointer.
            unsafe { mach_timebase_info(&mut info) };
            Self {
                talk_key,
                numer: u64::from(info.numer.max(1)),
                denom: u64::from(info.denom.max(1)),
            }
        }

        fn session_bool(dict: CFDictionaryRef, key: &CStr) -> Option<bool> {
            // SAFETY: `key` is NUL-terminated; every CF object made here is released here.
            unsafe {
                let k =
                    CFStringCreateWithCString(ptr::null(), key.as_ptr(), CF_STRING_ENCODING_UTF8);
                if k.is_null() {
                    return None;
                }
                let v = CFDictionaryGetValue(dict, k);
                CFRelease(k);
                if v.is_null() || CFGetTypeID(v) != CFBooleanGetTypeID() {
                    return None;
                }
                Some(CFBooleanGetValue(v))
            }
        }
    }

    impl PlatformProbe for MacProbe {
        fn secure_input_active(&self) -> bool {
            // SAFETY: no arguments; a read-only HIToolbox query.
            unsafe { IsSecureEventInputEnabled() != 0 }
        }

        fn physical_state(&self) -> PhysicalState {
            // SAFETY: read-only queries of the HID system state.
            let (code, _) = talk_key_code(self.talk_key);
            // SAFETY: read-only queries of the HID system state.
            let (talk, flags, space) = unsafe {
                (
                    CGEventSourceKeyState(CG_EVENT_SOURCE_STATE_HID, code),
                    CGEventSourceFlagsState(CG_EVENT_SOURCE_STATE_HID),
                    CGEventSourceKeyState(CG_EVENT_SOURCE_STATE_HID, VK_SPACE),
                )
            };
            physical_from_hid(talk, flags, space)
        }

        fn slept(&self) -> Duration {
            // The continuous clock runs during sleep; the absolute clock does not.
            // SAFETY: no arguments.
            let ticks = unsafe { mach_continuous_time().saturating_sub(mach_absolute_time()) };
            let nanos = u128::from(ticks) * u128::from(self.numer) / u128::from(self.denom);
            Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
        }

        fn session_active(&self) -> bool {
            // SAFETY: the copied dictionary is released below.
            let dict = unsafe { CGSessionCopyCurrentDictionary() };
            if dict.is_null() {
                return true;
            }
            let on_console = Self::session_bool(dict, c"kCGSSessionOnConsoleKey").unwrap_or(true);
            let locked = Self::session_bool(dict, c"CGSSessionScreenIsLocked").unwrap_or(false);
            // SAFETY: `dict` came from a Copy function.
            unsafe { CFRelease(dict) };
            on_console && !locked
        }

        fn input_monitoring_granted(&self) -> bool {
            // SAFETY: IOHIDCheckAccess never prompts.
            unsafe { IOHIDCheckAccess(IOHID_REQUEST_LISTEN_EVENT) == IOHID_ACCESS_GRANTED }
        }
    }

    /// The macOS permission API (CAP-36, CAP-37). `status` never prompts.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct MacPermissions;

    impl PermissionApi for MacPermissions {
        fn status(&self, permission: Permission) -> PermissionStatus {
            match permission {
                Permission::InputMonitoring => {
                    // SAFETY: IOHIDCheckAccess never prompts.
                    match unsafe { IOHIDCheckAccess(IOHID_REQUEST_LISTEN_EVENT) } {
                        IOHID_ACCESS_GRANTED => PermissionStatus::Granted,
                        IOHID_ACCESS_DENIED => PermissionStatus::Denied,
                        _ => PermissionStatus::Unknown,
                    }
                }
                // SAFETY: AXIsProcessTrusted never prompts.
                Permission::Accessibility => match unsafe { AXIsProcessTrusted() } != 0 {
                    true => PermissionStatus::Granted,
                    false => PermissionStatus::Unknown,
                },
            }
        }

        fn request(&self, permission: Permission) -> PermissionStatus {
            match permission {
                Permission::InputMonitoring => {
                    // SAFETY: shows the Input Monitoring prompt once; no pointers.
                    match unsafe { IOHIDRequestAccess(IOHID_REQUEST_LISTEN_EVENT) } {
                        true => PermissionStatus::Granted,
                        false => self.status(permission),
                    }
                }
                Permission::Accessibility => {
                    // SAFETY: a one-entry CF dictionary {prompt: true}, released after use.
                    let trusted = unsafe {
                        let keys = [kAXTrustedCheckOptionPrompt];
                        let values = [kCFBooleanTrue];
                        let opts = CFDictionaryCreate(
                            ptr::null(),
                            keys.as_ptr(),
                            values.as_ptr(),
                            1,
                            &kCFTypeDictionaryKeyCallBacks,
                            &kCFTypeDictionaryValueCallBacks,
                        );
                        let trusted = AXIsProcessTrustedWithOptions(opts) != 0;
                        if !opts.is_null() {
                            CFRelease(opts);
                        }
                        trusted
                    };
                    if trusted {
                        PermissionStatus::Granted
                    } else {
                        PermissionStatus::Unknown
                    }
                }
            }
        }

        fn open_settings(&self, permission: Permission) {
            use objc2_app_kit::NSWorkspace;
            use objc2_foundation::{NSString, NSURL};
            // A fixed Privacy pane URL: no user text, no prompt. A failed open changes nothing.
            let url = NSString::from_str(crate::hotkey_core::settings_pane(permission));
            if let Some(url) = NSURL::URLWithString(&url) {
                let _ = NSWorkspace::sharedWorkspace().openURL(&url);
            }
        }
    }

    struct TapThread {
        run_loop: usize,
        join: JoinHandle<()>,
    }

    /// A registered Carbon hot key and its handler: the fallback chord or the ask key.
    struct HotKey {
        hot_key: usize,
        handler: usize,
        // Keeps `Shared` alive while Carbon holds its pointer.
        _owner: Arc<Shared>,
    }

    impl HotKey {
        /// Unregister the hot key and remove its handler. Main thread only.
        fn release(self) {
            // SAFETY: both refs came from the Carbon register calls.
            unsafe {
                UnregisterEventHotKey(self.hot_key as EventHotKeyRef);
                RemoveEventHandler(self.handler as EventHandlerRef);
            }
        }
    }

    struct Shared {
        me: Weak<Shared>,
        talk_key: TalkKey,
        engine: Mutex<HotkeyEngine>,
        probe: MacProbe,
        /// One clock for the engine and the watchdog, so their times compare.
        clock: Arc<SystemClock>,
        tap_port: AtomicPtr<c_void>,
        /// The other-key tap of CAP-44c (ADR 0017). It is on only while
        /// `HotkeyEngine::other_key_window_open` is true.
        other_port: AtomicPtr<c_void>,
        other_on: AtomicBool,
        tap: Mutex<Option<TapThread>>,
        recreate_requested: AtomicBool,
        chord: Mutex<Option<HotKey>>,
        /// The ask key of RTR-17a (ADR 0098). It is registered only during a talk-key hold.
        ask_key: Mutex<Option<HotKey>>,
        /// The record key of ADR 0103. It is registered only during a talk-key hold with the
        /// Call recording knob on.
        record_key: Mutex<Option<HotKey>>,
        stop: AtomicBool,
    }

    /// The engine's [`AskKeyControl`]. It holds a weak ref, so the engine inside `Shared` does
    /// not keep `Shared` alive.
    struct AskKeyHandle(Weak<Shared>);

    impl AskKeyControl for AskKeyHandle {
        fn set_ask_key(&self, on: bool) {
            let Some(me) = self.0.upgrade() else { return };
            let ctx = Box::into_raw(Box::new((me, on))).cast::<c_void>();
            // SAFETY: Carbon hot keys run on the main thread; `ask_key_on_main` frees `ctx`.
            // The main queue is FIFO, so a grab and its release run in order.
            unsafe { dispatch_async_f(&_dispatch_main_q, ctx, ask_key_on_main) };
        }

        fn drain(&self) {
            let Some(me) = self.0.upgrade() else { return };
            let ctx = Box::into_raw(Box::new(me)).cast::<c_void>();
            // SAFETY: `ask_key_drain_on_main` frees `ctx`. It runs on the main thread, where
            // Carbon delivers the hot-key events.
            unsafe { dispatch_async_f(&_dispatch_main_q, ctx, ask_key_drain_on_main) };
        }

        fn set_record_key(&self, on: bool) {
            let Some(me) = self.0.upgrade() else { return };
            let ctx = Box::into_raw(Box::new((me, on))).cast::<c_void>();
            // SAFETY: as `set_ask_key`; `record_key_on_main` frees `ctx`.
            unsafe { dispatch_async_f(&_dispatch_main_q, ctx, record_key_on_main) };
        }
    }

    /// ADR 0103: grab or release ⌥R on the main thread, as `ask_key_on_main` does for ⌥G.
    extern "C" fn record_key_on_main(ctx: *mut c_void) {
        // SAFETY: `ctx` is the box made in `set_record_key`, used once.
        let (shared, on) = *unsafe { Box::from_raw(ctx.cast::<(Arc<Shared>, bool)>()) };
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let mut key = lock(&shared.record_key);
            if on {
                if key.is_none() {
                    // A refused register leaves R to the app; the other-key tap still marks
                    // the toggle in the first 1 s of the hold.
                    *key = register_hot_key(
                        &shared,
                        u32::from(VK_ANSI_R),
                        ask_key_modifiers(shared.talk_key),
                        super::RECORD_HOT_KEY_ID,
                        record_key_handler,
                        &[EVENT_HOT_KEY_PRESSED],
                    );
                }
            } else if let Some(k) = key.take() {
                k.release();
            }
        }));
    }

    extern "C" fn record_key_handler(
        _next: EventHandlerCallRef,
        event: EventRef,
        user: *mut c_void,
    ) -> i32 {
        catch_unwind(AssertUnwindSafe(|| {
            if hot_key_id(event) != Some(super::RECORD_HOT_KEY_ID) {
                return EVENT_NOT_HANDLED_ERR;
            }
            // SAFETY: `user` is the `Shared` kept alive by `HotKey::_owner`.
            let shared = unsafe { &*user.cast::<Shared>() };
            lock(&shared.engine).on_record_key();
            0
        }))
        .unwrap_or(EVENT_NOT_HANDLED_ERR)
    }

    /// The most queued hot-key presses that one drain handles. A held G repeats, so the count
    /// is bounded; the engine's ASK_UP_GRACE bounds the wait in any case.
    const DRAIN_MAX_EVENTS: usize = 32;

    /// RTR-17a (review M1): handle every hot-key press that Carbon has queued, then let the
    /// engine decide the waiting key-up. So a G tapped before the release is seen first, also
    /// when the main thread was busy. The engine lock is not held while the events run, because
    /// `ask_key_handler` takes it.
    extern "C" fn ask_key_drain_on_main(ctx: *mut c_void) {
        // SAFETY: `ctx` is the box made in `drain`, used once.
        let shared = *unsafe { Box::from_raw(ctx.cast::<Arc<Shared>>()) };
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let types = [EventTypeSpec {
                event_class: EVENT_CLASS_KEYBOARD,
                event_kind: EVENT_HOT_KEY_PRESSED,
            }];
            for _ in 0..DRAIN_MAX_EVENTS {
                let mut event: EventRef = ptr::null_mut();
                // SAFETY: a no-wait pull of one queued hot-key press; the event is sent to
                // the dispatcher (so its handler runs) and then released.
                unsafe {
                    if ReceiveNextEvent(1, types.as_ptr(), EVENT_DURATION_NO_WAIT, 1, &mut event)
                        != 0
                        || event.is_null()
                    {
                        break;
                    }
                    SendEventToEventTarget(event, GetEventDispatcherTarget());
                    ReleaseEvent(event);
                }
            }
            lock(&shared.engine).on_ask_key_drained();
        }));
    }

    extern "C" fn ask_key_on_main(ctx: *mut c_void) {
        // SAFETY: `ctx` is the box made in `set_ask_key`, used once.
        let (shared, on) = *unsafe { Box::from_raw(ctx.cast::<(Arc<Shared>, bool)>()) };
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let mut key = lock(&shared.ask_key);
            if on {
                if key.is_none() {
                    // A refused register leaves G to the app; the other-key tap still marks an
                    // ask in the first 1 s of the hold.
                    *key = register_hot_key(
                        &shared,
                        u32::from(VK_ANSI_G),
                        ask_key_modifiers(shared.talk_key),
                        super::ASK_HOT_KEY_ID,
                        ask_key_handler,
                        &[EVENT_HOT_KEY_PRESSED],
                    );
                }
            } else if let Some(k) = key.take() {
                k.release();
            }
        }));
    }

    /// The Carbon hot-key ID of `event`, if it is a Bottle hot key.
    fn hot_key_id(event: EventRef) -> Option<u32> {
        let mut id = EventHotKeyId {
            signature: 0,
            id: 0,
        };
        // SAFETY: `event` is the Carbon event of the calling handler; `id` is a valid buffer of
        // the size passed.
        let status = unsafe {
            GetEventParameter(
                event,
                EVENT_PARAM_DIRECT_OBJECT,
                TYPE_EVENT_HOT_KEY_ID,
                ptr::null_mut(),
                std::mem::size_of::<EventHotKeyId>(),
                ptr::null_mut(),
                (&raw mut id).cast::<c_void>(),
            )
        };
        (status == 0 && id.signature == HOT_KEY_SIGNATURE).then_some(id.id)
    }

    extern "C" fn ask_key_handler(
        _next: EventHandlerCallRef,
        event: EventRef,
        user: *mut c_void,
    ) -> i32 {
        catch_unwind(AssertUnwindSafe(|| {
            if hot_key_id(event) != Some(super::ASK_HOT_KEY_ID) {
                return EVENT_NOT_HANDLED_ERR;
            }
            // SAFETY: `user` is the `Shared` kept alive by `HotKey::_owner`.
            let shared = unsafe { &*user.cast::<Shared>() };
            lock(&shared.engine).on_ask_key();
            0
        }))
        .unwrap_or(EVENT_NOT_HANDLED_ERR)
    }

    impl TapControl for Shared {
        fn re_enable(&self) {
            let port = self.tap_port.load(Ordering::Acquire);
            if !port.is_null() {
                // SAFETY: `port` is the live tap; the tap thread clears it before release.
                unsafe { CGEventTapEnable(port, true) };
            }
        }

        fn recreate(&self) {
            // The watchdog thread does it, outside the engine lock and off the tap thread.
            self.recreate_requested.store(true, Ordering::Release);
        }

        fn tap_alive(&self) -> bool {
            !self.tap_port.load(Ordering::Acquire).is_null()
        }

        fn set_chord(&self, on: bool) {
            let Some(me) = self.me.upgrade() else { return };
            let ctx = Box::into_raw(Box::new((me, on))).cast::<c_void>();
            // SAFETY: Carbon hot keys run on the main thread; `chord_on_main` frees `ctx`.
            unsafe { dispatch_async_f(&_dispatch_main_q, ctx, chord_on_main) };
        }
    }

    extern "C" fn chord_on_main(ctx: *mut c_void) {
        // SAFETY: `ctx` is the box made in `set_chord`, used once.
        let (shared, on) = *unsafe { Box::from_raw(ctx.cast::<(Arc<Shared>, bool)>()) };
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let mut chord = lock(&shared.chord);
            if on {
                if chord.is_none() {
                    *chord = register_hot_key(
                        &shared,
                        u32::from(VK_SPACE),
                        CARBON_CONTROL_KEY | CARBON_SHIFT_KEY,
                        super::CHORD_HOT_KEY_ID,
                        chord_handler,
                        &[EVENT_HOT_KEY_PRESSED, EVENT_HOT_KEY_RELEASED],
                    );
                }
                let ok = chord.is_some();
                drop(chord);
                // Report the result, so a refused chord is not silent (CAP-39b).
                let now = shared.clock.now();
                lock(&shared.engine).on_chord_registered(ok, now);
            } else if let Some(c) = chord.take() {
                c.release();
            }
        }));
    }

    /// Register the Carbon hot key `key_code` + `mods` with `id`, and install `handler` for
    /// the event `kinds` on the application target. Main thread only.
    fn register_hot_key(
        shared: &Arc<Shared>,
        key_code: u32,
        mods: u32,
        id: u32,
        handler_proc: EventHandlerProc,
        kinds: &[u32],
    ) -> Option<HotKey> {
        let types: Vec<EventTypeSpec> = kinds
            .iter()
            .map(|&event_kind| EventTypeSpec {
                event_class: EVENT_CLASS_KEYBOARD,
                event_kind,
            })
            .collect();
        let count = u32::try_from(types.len()).ok()?;
        let mut handler: EventHandlerRef = ptr::null_mut();
        let mut hot_key: EventHotKeyRef = ptr::null_mut();
        // SAFETY: valid out pointers; `user_data` stays alive through `HotKey::_owner`.
        unsafe {
            let target = GetApplicationEventTarget();
            let user = Arc::as_ptr(shared).cast_mut().cast::<c_void>();
            if InstallEventHandler(
                target,
                handler_proc,
                count,
                types.as_ptr(),
                user,
                &mut handler,
            ) != 0
            {
                return None;
            }
            let id = EventHotKeyId {
                signature: HOT_KEY_SIGNATURE,
                id,
            };
            if RegisterEventHotKey(key_code, mods, id, target, 0, &mut hot_key) != 0 {
                RemoveEventHandler(handler);
                return None;
            }
        }
        Some(HotKey {
            hot_key: hot_key as usize,
            handler: handler as usize,
            _owner: Arc::clone(shared),
        })
    }

    extern "C" fn chord_handler(
        _next: EventHandlerCallRef,
        event: EventRef,
        user: *mut c_void,
    ) -> i32 {
        catch_unwind(AssertUnwindSafe(|| {
            if hot_key_id(event) != Some(super::CHORD_HOT_KEY_ID) {
                return EVENT_NOT_HANDLED_ERR;
            }
            // SAFETY: `user` is the `Shared` kept alive by `HotKey::_owner`.
            let shared = unsafe { &*user.cast::<Shared>() };
            // SAFETY: `event` is the Carbon event of this call.
            let edge = match unsafe { GetEventKind(event) } {
                EVENT_HOT_KEY_PRESSED => KeyEdge::Down,
                EVENT_HOT_KEY_RELEASED => KeyEdge::Up,
                _ => return EVENT_NOT_HANDLED_ERR,
            };
            // No other-key sync here: the window opens only for a talk-key hold (taps get
            // no KeyDown under Secure Event Input), and this runs on the main thread.
            lock(&shared.engine).on_chord(edge);
            0
        }))
        .unwrap_or(EVENT_NOT_HANDLED_ERR)
    }

    /// Turn the other-key tap on or off to match the engine (CAP-44c). It listens only in the
    /// first 1 s of a talk-key hold, so Bottle gets no key-down at any other time.
    fn sync_other_key_tap(shared: &Shared, engine: &HotkeyEngine) {
        let want = engine.other_key_window_open();
        if shared.other_on.swap(want, Ordering::AcqRel) == want {
            return;
        }
        let port = shared.other_port.load(Ordering::Acquire);
        if !port.is_null() {
            // SAFETY: the caller holds the engine lock (it passes `engine`), and the tap
            // thread clears `other_port` under that lock before it invalidates and releases
            // the port. So a non-null `port` read here stays alive until this call returns.
            unsafe { CGEventTapEnable(port, want) };
        }
    }

    extern "C" fn other_key_callback(
        _proxy: CGEventTapProxy,
        event_type: u32,
        event: CGEventRef,
        user: *mut c_void,
    ) -> CGEventRef {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: `user` is the `Shared` that the tap thread keeps alive.
            let shared = unsafe { &*user.cast::<Shared>() };
            // Privacy (ADR 0017, ADR 0098, ADR 0103): the event type is used, and for a
            // key-down the key code is compared with G and R only. The code and the characters
            // are never stored or logged, so no key identity crosses this boundary.
            if matches!(
                event_type,
                CG_EVENT_TAP_DISABLED_BY_TIMEOUT | CG_EVENT_TAP_DISABLED_BY_USER_INPUT
            ) {
                // The OS turned the tap off; the next sync turns it on again if needed.
                shared.other_on.store(false, Ordering::Release);
            }
            let keycode = if event_type == CG_EVENT_KEY_DOWN {
                // SAFETY: `event` is the key-down event of this callback.
                unsafe { CGEventGetIntegerValueField(event, CG_KEYBOARD_EVENT_KEYCODE) }
            } else {
                -1
            };
            let mut engine = lock(&shared.engine);
            if let Some(key) = other_key_from_event(event_type, keycode) {
                apply_other_key(&mut engine, key);
            }
            sync_other_key_tap(shared, &engine);
        }));
        // A listen-only tap cannot change the event; it passes on unchanged.
        event
    }

    extern "C" fn tap_callback(
        _proxy: CGEventTapProxy,
        event_type: u32,
        event: CGEventRef,
        user: *mut c_void,
    ) -> CGEventRef {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: `user` is the `Shared` that the tap thread keeps alive.
            let shared = unsafe { &*user.cast::<Shared>() };
            let signal = match event_type {
                CG_EVENT_TAP_DISABLED_BY_TIMEOUT => TapSignal::DisabledByTimeout,
                CG_EVENT_TAP_DISABLED_BY_USER_INPUT => TapSignal::DisabledByUserInput,
                CG_EVENT_FLAGS_CHANGED => {
                    // Only the key code is compared, and only with the talk key. It is
                    // never stored or logged.
                    // SAFETY: `event` is the event of this callback.
                    let (code, flags) = unsafe {
                        (
                            CGEventGetIntegerValueField(event, CG_KEYBOARD_EVENT_KEYCODE),
                            CGEventGetFlags(event),
                        )
                    };
                    let code = u16::try_from(code).unwrap_or(u16::MAX);
                    let class = classify_flags_changed(shared.talk_key, code, flags);
                    if class != FlagsEvent::Ignore {
                        let mut engine = lock(&shared.engine);
                        apply_flags_event(&mut engine, class);
                        sync_other_key_tap(shared, &engine);
                    }
                    return;
                }
                _ => return,
            };
            let now = shared.clock.now();
            lock(&shared.engine).on_tap_signal(signal, now, &shared.probe, shared);
        }));
        // A listen-only tap cannot change the event; it passes on unchanged.
        event
    }

    fn spawn_tap(shared: &Arc<Shared>) -> CoreResult<()> {
        let (tx, rx) = mpsc::channel::<Option<usize>>();
        let owner = Arc::clone(shared);
        let join = thread::Builder::new()
            .name("bottle-hotkey-tap".into())
            .spawn(move || {
                let user = Arc::as_ptr(&owner).cast_mut().cast::<c_void>();
                // SAFETY: TAP_SPEC is the listen-only spec (CAP-35a); `user` lives as long as
                // `owner`, which lives until this thread ends, after the port is invalidated.
                unsafe {
                    let port = CGEventTapCreate(
                        TAP_SPEC.location,
                        TAP_SPEC.placement,
                        TAP_SPEC.options,
                        TAP_SPEC.events_of_interest,
                        tap_callback,
                        user,
                    );
                    if port.is_null() {
                        let _ = tx.send(None);
                        return;
                    }
                    let source = CFMachPortCreateRunLoopSource(ptr::null(), port, 0);
                    let run_loop = CFRunLoopGetCurrent();
                    CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
                    CGEventTapEnable(port, true);
                    // The other-key tap (CAP-44c, ADR 0017): created off on the same run
                    // loop. If the OS refuses it, PTT still works; only the key-down cancel
                    // of non-modifier keys is lost (modifiers still come by FlagsChanged).
                    let other = CGEventTapCreate(
                        OTHER_KEY_TAP_SPEC.location,
                        OTHER_KEY_TAP_SPEC.placement,
                        OTHER_KEY_TAP_SPEC.options,
                        OTHER_KEY_TAP_SPEC.events_of_interest,
                        other_key_callback,
                        user,
                    );
                    let other_source = if other.is_null() {
                        ptr::null_mut()
                    } else {
                        CGEventTapEnable(other, false);
                        let src = CFMachPortCreateRunLoopSource(ptr::null(), other, 0);
                        if !src.is_null() {
                            CFRunLoopAddSource(run_loop, src, kCFRunLoopCommonModes);
                        }
                        src
                    };
                    owner.other_on.store(false, Ordering::Release);
                    owner.other_port.store(other, Ordering::Release);
                    owner.tap_port.store(port, Ordering::Release);
                    let _ = tx.send(Some(run_loop as usize));
                    CFRunLoopRun();
                    let _ = owner.tap_port.compare_exchange(
                        port,
                        ptr::null_mut(),
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    if !other.is_null() {
                        // Clear the port under the engine lock, so no `sync_other_key_tap`
                        // can hold a copy of it while it is released below.
                        {
                            let _engine = lock(&owner.engine);
                            let _ = owner.other_port.compare_exchange(
                                other,
                                ptr::null_mut(),
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            );
                        }
                        CFMachPortInvalidate(other);
                        if !other_source.is_null() {
                            CFRelease(other_source);
                        }
                        CFRelease(other);
                    }
                    CFMachPortInvalidate(port);
                    CFRelease(source);
                    CFRelease(port);
                }
            })
            .map_err(|_| unavailable("event_tap_failed"))?;
        match rx.recv() {
            Ok(Some(run_loop)) => {
                *lock(&shared.tap) = Some(TapThread { run_loop, join });
                Ok(())
            }
            _ => {
                let _ = join.join();
                Err(unavailable("event_tap_failed"))
            }
        }
    }

    fn stop_tap(shared: &Shared) {
        if let Some(t) = lock(&shared.tap).take() {
            // SAFETY: the run loop belongs to the live tap thread; a stop before the run
            // starts is kept by CoreFoundation, so the thread still ends.
            unsafe { CFRunLoopStop(t.run_loop as CFRunLoopRef) };
            let _ = t.join.join();
        }
    }

    fn unavailable(reason: &'static str) -> CoreError {
        CoreError::Unavailable {
            component: ID,
            reason,
        }
    }

    fn watchdog_loop(shared: Arc<Shared>) {
        while !shared.stop.load(Ordering::Acquire) {
            let now = shared.clock.now();
            {
                let mut engine = lock(&shared.engine);
                let port = shared.tap_port.load(Ordering::Acquire);
                if !port.is_null() {
                    // SAFETY: `port` is the live tap.
                    let enabled = unsafe { CGEventTapIsEnabled(port) };
                    engine.on_tap_signal(
                        TapSignal::HealthCheck { enabled },
                        now,
                        &shared.probe,
                        &*shared,
                    );
                }
                engine.tick(now, &shared.probe, &*shared);
                // Close the other-key window 1 s into a hold (CAP-44c).
                sync_other_key_tap(&shared, &engine);
            }
            if shared.recreate_requested.swap(false, Ordering::AcqRel) {
                stop_tap(&shared);
                // On a failure no tap is alive; the engine asks again every
                // TAP_RETRY_INTERVAL while Input Monitoring is granted (CAP-38b).
                let _ = spawn_tap(&shared);
            }
            thread::sleep(WATCHDOG_TICK);
        }
    }

    /// R1 HotkeyService on macOS (#6). The Tauri host creates it in `src-tauri/src/hotkey_host.rs` (#305).
    ///
    /// The sink is called with the engine lock held, from the tap thread, the watchdog thread
    /// or the main thread. It must return fast and must not call back into this service.
    pub struct MacHotkeyService {
        shared: Arc<Shared>,
        permissions: MacPermissions,
        watchdog: Mutex<Option<JoinHandle<()>>>,
    }

    impl std::fmt::Debug for MacHotkeyService {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MacHotkeyService")
                .field("talk_key", &self.shared.talk_key)
                .finish_non_exhaustive()
        }
    }

    impl MacHotkeyService {
        /// A service for `talk_key` (D66 default: [`TalkKey::RightOption`]) that sends its
        /// outputs to `sink`. Nothing starts until [`HotkeyService::start`].
        pub fn new(talk_key: TalkKey, sink: Arc<dyn HotkeySink>) -> Self {
            let clock = Arc::new(SystemClock::new());
            let shared = Arc::new_cyclic(|me| Shared {
                me: me.clone(),
                talk_key,
                engine: Mutex::new(
                    HotkeyEngine::with_clock(sink, clock.clone())
                        .with_ask_key(Arc::new(AskKeyHandle(me.clone()))),
                ),
                probe: MacProbe::new(talk_key),
                clock,
                tap_port: AtomicPtr::new(ptr::null_mut()),
                other_port: AtomicPtr::new(ptr::null_mut()),
                other_on: AtomicBool::new(false),
                tap: Mutex::new(None),
                recreate_requested: AtomicBool::new(false),
                chord: Mutex::new(None),
                ask_key: Mutex::new(None),
                record_key: Mutex::new(None),
                stop: AtomicBool::new(false),
            });
            Self {
                shared,
                permissions: MacPermissions,
                watchdog: Mutex::new(None),
            }
        }

        /// The same service with the Call recording knob `gate` (ADR 0103): while it is true,
        /// each talk-key hold also grabs ⌥R (the talk-key modifier + R).
        pub fn with_record_gate(self, gate: Arc<AtomicBool>) -> Self {
            lock(&self.shared.engine).set_record_gate(gate);
            self
        }
    }

    impl HotkeyService for MacHotkeyService {
        /// Check Input Monitoring with no prompt (CAP-36), create the listen-only tap on its
        /// own run-loop thread (CAP-35a) and start the watchdog (CAP-38a, CAP-38b, CAP-39a).
        /// Without the permission it returns `input_monitoring_required`; the host asks for
        /// it after a user action and calls `start` again.
        fn start(&self) -> CoreResult<()> {
            let mut watchdog = lock(&self.watchdog);
            if watchdog.is_some() {
                return Ok(());
            }
            check_ptt_permission(&self.permissions)?;
            spawn_tap(&self.shared)?;
            self.shared.stop.store(false, Ordering::Release);
            let shared = Arc::clone(&self.shared);
            let join = thread::Builder::new()
                .name("bottle-hotkey-watchdog".into())
                .spawn(move || watchdog_loop(shared));
            match join {
                Ok(join) => {
                    *watchdog = Some(join);
                    Ok(())
                }
                Err(_) => {
                    stop_tap(&self.shared);
                    Err(unavailable("watchdog_failed"))
                }
            }
        }

        fn stop(&self) -> CoreResult<()> {
            if let Some(join) = lock(&self.watchdog).take() {
                self.shared.stop.store(true, Ordering::Release);
                let _ = join.join();
            }
            stop_tap(&self.shared);
            self.shared.set_chord(false);
            lock(&self.shared.engine).reset();
            Ok(())
        }

        fn secure_input_active(&self) -> CoreResult<bool> {
            Ok(self.shared.probe.secure_input_active())
        }
    }

    impl Drop for MacHotkeyService {
        /// Stops the tap and the watchdog. While the chord or the ask key is registered,
        /// `Shared` is also owned by `HotKey::_owner` (Carbon holds its pointer). The unregister runs on the
        /// main dispatch queue, so a host that never runs the main queue keeps `Shared` alive
        /// after drop. A Tauri app always runs it.
        fn drop(&mut self) {
            let _ = HotkeyService::stop(self);
        }
    }
}
