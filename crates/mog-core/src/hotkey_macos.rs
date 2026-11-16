// Adapted from cjpais/Handy src-tauri/src/secure_input.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! R1 HotkeyService on macOS (#6): listen-only CGEventTap, watchdog, Secure Event Input.
//!
//! The pure part of this file (tap spec, key map, flag decode) compiles on every OS so its
//! tests run everywhere. The OS calls are in [`sys`], built only on macOS.
//!
//! Privacy: the tap listens to `FlagsChanged` only. It never sees `KeyDown` or `KeyUp` of
//! typed text, so no key contents reach Bottle, a log or telemetry (ADR 0008).
//!
//! Requirements: CAP-35a, CAP-35b, CAP-36, CAP-37, CAP-38a, CAP-38b, CAP-39a, CAP-39b,
//! CAP-45. Real-device proof (AT-30) waits for SP-03: TODO(#5).

use crate::hotkey_core::{KeyEdge, PhysicalState, TalkKey};

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
/// Carbon `controlKey`.
pub const CARBON_CONTROL_KEY: u32 = 0x1000;
/// Carbon `shiftKey`.
pub const CARBON_SHIFT_KEY: u32 = 0x0200;
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
    options: CG_EVENT_TAP_OPTION_DEFAULT,
    events_of_interest: 0,
};

/// The virtual key code and the device-dependent flag bit of a talk key.
pub fn talk_key_code(_key: TalkKey) -> (u16, u64) {
    todo!()
}

/// Decode one `FlagsChanged` event for `key`. Returns the edge, or `None` for another key.
pub fn decode_flags_changed(_key: TalkKey, _keycode: u16, _flags: u64) -> Option<KeyEdge> {
    todo!()
}

/// The physical state from the HID flag state and the Space key state (CAP-45).
pub fn physical_from_flags(_key: TalkKey, _flags: u64, _space_down: bool) -> PhysicalState {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey_core::{HotkeyOutput, PttMachine, PttPhase};

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
        assert_eq!(decode_flags_changed(TalkKey::RightOption, 58, 0x0008_0120), None);
        assert_eq!(decode_flags_changed(TalkKey::RightOption, 56, 0x0002_0102), None);
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
    fn cap_45_physical_state_reads_the_device_flag() {
        let s = physical_from_flags(TalkKey::RightOption, 0x0008_0140, false);
        assert!(s.talk_key_down);
        assert!(!s.chord_down);
        let s = physical_from_flags(TalkKey::RightOption, 0x0008_0120, false);
        assert!(!s.talk_key_down);
        let chord = CG_FLAG_CONTROL | CG_FLAG_SHIFT;
        assert!(physical_from_flags(TalkKey::RightOption, chord, true).chord_down);
        assert!(!physical_from_flags(TalkKey::RightOption, chord, false).chord_down);
        assert!(!physical_from_flags(TalkKey::RightOption, CG_FLAG_SHIFT, true).chord_down);
    }
}
