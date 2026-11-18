//! The power-source watcher of R4 (CAP-51a, CAP-51b, ADR 0082).
//!
//! [`SystemPowerProbe`] reads the power source from the OS: macOS
//! `IOPSGetProvidingPowerSourceType`, Windows `GetSystemPowerStatus`. Raw FFI to system
//! libraries only, no new crate (the same rule as `hotkey_macos.rs`, ADR 0011). The engine
//! reads it on each residency tick.

/// Where the device takes its power from now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerSource {
    /// Mains power (also a UPS).
    Ac,
    /// The internal battery.
    Battery,
    /// The OS did not say, for example a desktop with no battery. It counts as AC power.
    Unknown,
}

/// A source of the power state. The engine calls it from its worker thread.
pub trait PowerProbe: Send + Sync + 'static {
    /// The power source now.
    fn source(&self) -> PowerSource;
}

/// The power source of this device, from the OS.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemPowerProbe;

impl PowerProbe for SystemPowerProbe {
    fn source(&self) -> PowerSource {
        platform::source()
    }
}

/// The macOS `IOPSGetProvidingPowerSourceType` value to a [`PowerSource`].
pub fn from_macos_source_type(kind: &str) -> PowerSource {
    match kind {
        "AC Power" | "UPS Power" => PowerSource::Ac,
        "Battery Power" => PowerSource::Battery,
        _ => PowerSource::Unknown,
    }
}

/// The Windows `SYSTEM_POWER_STATUS` fields to a [`PowerSource`]. `ACLineStatus` is 0 offline,
/// 1 online, 255 unknown. `BatteryFlag` 128 means that the device has no battery.
pub fn from_windows_status(ac_line_status: u8, battery_flag: u8) -> PowerSource {
    match (ac_line_status, battery_flag) {
        (_, 128) => PowerSource::Ac,
        (0, _) => PowerSource::Battery,
        (1, _) => PowerSource::Ac,
        _ => PowerSource::Unknown,
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{from_macos_source_type, PowerSource};
    use std::ffi::{c_char, c_void, CStr};

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
        fn IOPSGetProvidingPowerSourceType(snapshot: CFTypeRef) -> CFStringRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: CFTypeRef);
        fn CFStringGetCString(
            s: CFStringRef,
            buffer: *mut c_char,
            size: isize,
            encoding: u32,
        ) -> u8;
    }

    pub(super) fn source() -> PowerSource {
        // SAFETY: `IOPSCopyPowerSourcesInfo` returns an owned snapshot or null; it is released
        // once below. `IOPSGetProvidingPowerSourceType` returns a string that the snapshot owns
        // (the Get rule), read before the release. `buf` is valid for `buf.len()` bytes, and
        // `CFStringGetCString` writes a NUL-terminated string into it when it returns true.
        unsafe {
            let snapshot = IOPSCopyPowerSourcesInfo();
            if snapshot.is_null() {
                return PowerSource::Unknown;
            }
            let kind = IOPSGetProvidingPowerSourceType(snapshot);
            let mut buf = [0 as c_char; 64];
            let ok = !kind.is_null()
                && CFStringGetCString(
                    kind,
                    buf.as_mut_ptr(),
                    buf.len() as isize,
                    K_CF_STRING_ENCODING_UTF8,
                ) != 0;
            let out = if ok {
                CStr::from_ptr(buf.as_ptr())
                    .to_str()
                    .map_or(PowerSource::Unknown, from_macos_source_type)
            } else {
                PowerSource::Unknown
            };
            CFRelease(snapshot);
            out
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::{from_windows_status, PowerSource};
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    pub(super) fn source() -> PowerSource {
        let mut status = SYSTEM_POWER_STATUS {
            ACLineStatus: 255,
            BatteryFlag: 255,
            BatteryLifePercent: 255,
            SystemStatusFlag: 0,
            BatteryLifeTime: 0,
            BatteryFullLifeTime: 0,
        };
        // SAFETY: `status` is a valid, writable SYSTEM_POWER_STATUS.
        if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
            return PowerSource::Unknown;
        }
        from_windows_status(status.ACLineStatus, status.BatteryFlag)
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use super::PowerSource;

    pub(super) fn source() -> PowerSource {
        PowerSource::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_51a_os_values_map_to_a_power_source() {
        assert_eq!(from_macos_source_type("AC Power"), PowerSource::Ac);
        assert_eq!(from_macos_source_type("UPS Power"), PowerSource::Ac);
        assert_eq!(
            from_macos_source_type("Battery Power"),
            PowerSource::Battery
        );
        assert_eq!(from_macos_source_type(""), PowerSource::Unknown);
        assert_eq!(from_windows_status(0, 1), PowerSource::Battery);
        assert_eq!(from_windows_status(1, 1), PowerSource::Ac);
        assert_eq!(from_windows_status(255, 128), PowerSource::Ac);
        assert_eq!(from_windows_status(255, 1), PowerSource::Unknown);
    }

    #[test]
    fn cap_51a_the_os_probe_answers() {
        // Any answer is fine on a build machine; the call must not fail or panic.
        let _ = SystemPowerProbe.source();
    }
}
