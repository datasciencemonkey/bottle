// Adapted from cjpais/Handy src-tauri/src/secure_input.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! R1 HotkeyService on macOS (#6): listen-only CGEventTap, watchdog, Secure Event Input.
//!
//! The pure part of this file (tap spec, key map, flag decode) compiles on every OS so its
//! tests run everywhere. The OS calls are in [`sys`], built only on macOS.
//!
//! Privacy: the tap listens to `FlagsChanged` only. It never sees `KeyDown` or `KeyUp` of
//! typed text, so no key contents reach Bottle, a log or telemetry (ADR 0011).
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
    options: CG_EVENT_TAP_OPTION_LISTEN_ONLY,
    events_of_interest: 1 << CG_EVENT_FLAGS_CHANGED,
};

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

/// The physical state from the HID key state of the talk key, the HID flag state and the
/// HID key state of Space (CAP-45). The talk key uses its own key state, not the
/// device-dependent flag bits, which a source-state read may not carry.
pub fn physical_from_hid(_talk_key_down: bool, _flags: u64, _space_down: bool) -> PhysicalState {
    todo!()
}

/// The physical state from the HID flag state and the Space key state (CAP-45).
pub fn physical_from_flags(key: TalkKey, flags: u64, space_down: bool) -> PhysicalState {
    let (_, mask) = talk_key_code(key);
    let chord_mods = CG_FLAG_CONTROL | CG_FLAG_SHIFT;
    PhysicalState {
        talk_key_down: flags & mask != 0,
        chord_down: space_down && flags & chord_mods == chord_mods,
    }
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
        check_ptt_permission, HotkeyEngine, HotkeySink, Permission, PermissionApi,
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
    /// The signature of the Bottle hot key ('BTLE').
    const HOT_KEY_SIGNATURE: u32 = u32::from_be_bytes(*b"BTLE");

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
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
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

    /// Run the main-thread run loop. The Carbon chord handler runs on it. A Tauri app already
    /// runs it; the device probe example calls this.
    pub fn run_main_loop() {
        // SAFETY: CFRunLoopRun runs the run loop of the calling thread.
        unsafe { CFRunLoopRun() }
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
            let (flags, space) = unsafe {
                (
                    CGEventSourceFlagsState(CG_EVENT_SOURCE_STATE_HID),
                    CGEventSourceKeyState(CG_EVENT_SOURCE_STATE_HID, VK_SPACE),
                )
            };
            physical_from_flags(self.talk_key, flags, space)
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
                Permission::Accessibility => match unsafe { AXIsProcessTrusted() } {
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
                        let trusted = AXIsProcessTrustedWithOptions(opts);
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
    }

    struct TapThread {
        run_loop: usize,
        join: JoinHandle<()>,
    }

    struct Chord {
        hot_key: usize,
        handler: usize,
        // Keeps `Shared` alive while Carbon holds its pointer.
        _owner: Arc<Shared>,
    }

    struct Shared {
        me: Weak<Shared>,
        talk_key: TalkKey,
        engine: Mutex<HotkeyEngine>,
        probe: MacProbe,
        clock: SystemClock,
        tap_port: AtomicPtr<c_void>,
        tap: Mutex<Option<TapThread>>,
        recreate_requested: AtomicBool,
        chord: Mutex<Option<Chord>>,
        stop: AtomicBool,
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
            if on && chord.is_none() {
                *chord = register_chord(&shared);
            } else if !on {
                if let Some(c) = chord.take() {
                    // SAFETY: both refs came from the Carbon register calls.
                    unsafe {
                        UnregisterEventHotKey(c.hot_key as EventHotKeyRef);
                        RemoveEventHandler(c.handler as EventHandlerRef);
                    }
                }
            }
        }));
    }

    fn register_chord(shared: &Arc<Shared>) -> Option<Chord> {
        let types = [
            EventTypeSpec {
                event_class: EVENT_CLASS_KEYBOARD,
                event_kind: EVENT_HOT_KEY_PRESSED,
            },
            EventTypeSpec {
                event_class: EVENT_CLASS_KEYBOARD,
                event_kind: EVENT_HOT_KEY_RELEASED,
            },
        ];
        let mut handler: EventHandlerRef = ptr::null_mut();
        let mut hot_key: EventHotKeyRef = ptr::null_mut();
        // SAFETY: valid out pointers; `user_data` stays alive through `Chord::_owner`.
        unsafe {
            let target = GetApplicationEventTarget();
            let user = Arc::as_ptr(shared).cast_mut().cast::<c_void>();
            if InstallEventHandler(target, chord_handler, 2, types.as_ptr(), user, &mut handler)
                != 0
            {
                return None;
            }
            let id = EventHotKeyId {
                signature: HOT_KEY_SIGNATURE,
                id: 1,
            };
            let mods = CARBON_CONTROL_KEY | CARBON_SHIFT_KEY;
            if RegisterEventHotKey(u32::from(VK_SPACE), mods, id, target, 0, &mut hot_key) != 0 {
                RemoveEventHandler(handler);
                return None;
            }
        }
        Some(Chord {
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
            // SAFETY: `user` is the `Shared` kept alive by `Chord::_owner`.
            let shared = unsafe { &*user.cast::<Shared>() };
            // SAFETY: `event` is the Carbon event of this call.
            let edge = match unsafe { GetEventKind(event) } {
                EVENT_HOT_KEY_PRESSED => KeyEdge::Down,
                EVENT_HOT_KEY_RELEASED => KeyEdge::Up,
                _ => return EVENT_NOT_HANDLED_ERR,
            };
            lock(&shared.engine).on_chord(edge);
            0
        }))
        .unwrap_or(EVENT_NOT_HANDLED_ERR)
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
                    if let Some(edge) = decode_flags_changed(shared.talk_key, code, flags) {
                        lock(&shared.engine).on_talk_key(edge);
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
                    owner.tap_port.store(port, Ordering::Release);
                    let _ = tx.send(Some(run_loop as usize));
                    CFRunLoopRun();
                    let _ = owner.tap_port.compare_exchange(
                        port,
                        ptr::null_mut(),
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
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
            }
            if shared.recreate_requested.swap(false, Ordering::AcqRel) {
                stop_tap(&shared);
                // A failure here is retried on the next permission grant or wake.
                let _ = spawn_tap(&shared);
            }
            thread::sleep(WATCHDOG_TICK);
        }
    }

    /// R1 HotkeyService on macOS (#6).
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
            let shared = Arc::new_cyclic(|me| Shared {
                me: me.clone(),
                talk_key,
                engine: Mutex::new(HotkeyEngine::new(sink)),
                probe: MacProbe::new(talk_key),
                clock: SystemClock::new(),
                tap_port: AtomicPtr::new(ptr::null_mut()),
                tap: Mutex::new(None),
                recreate_requested: AtomicBool::new(false),
                chord: Mutex::new(None),
                stop: AtomicBool::new(false),
            });
            Self {
                shared,
                permissions: MacPermissions,
                watchdog: Mutex::new(None),
            }
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
        fn drop(&mut self) {
            let _ = HotkeyService::stop(self);
        }
    }
}
