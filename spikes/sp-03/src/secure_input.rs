// Adapted from cjpais/Handy src-tauri/src/secure_input.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! Secure Event Input (SEI): the `IsSecureEventInputEnabled` binding and the best-effort lookup
//! of the process that holds SEI (`ioreg` `kCGSSessionSecureInputPID`, then `ps`).

/// The holder PID of Secure Event Input from `ioreg -l -w 0` text (`kCGSSessionSecureInputPID`).
/// Best effort: Apple documents no reliable API, and the PID can be missing or wrong.
pub fn parse_secure_input_pid(ioreg: &str) -> Option<i32> {
    ioreg
        .lines()
        .find_map(|l| l.split("\"kCGSSessionSecureInputPID\"=").nth(1))?
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

#[cfg(target_os = "macos")]
mod imp {
    use super::parse_secure_input_pid;
    use std::process::Command;

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        // Carbon HIToolbox; Boolean is an unsigned char.
        fn IsSecureEventInputEnabled() -> u8;
    }

    /// True if some process has Secure Event Input on (live check).
    pub fn is_enabled() -> bool {
        // SAFETY: no arguments.
        unsafe { IsSecureEventInputEnabled() != 0 }
    }

    /// The SEI holder as `pid=<n> name="<binary>"`, or `none_visible` / `unknown`.
    /// Slow (`ioreg -l` takes tens of ms): call it off any poll thread.
    pub fn holder() -> String {
        let Ok(out) = Command::new("ioreg").args(["-l", "-w", "0"]).output() else {
            return "unknown".into();
        };
        let Some(pid) = parse_secure_input_pid(&String::from_utf8_lossy(&out.stdout)) else {
            return "none_visible".into();
        };
        // `ps -o comm=` returns the full path; keep the binary name.
        let name = Command::new("ps")
            .args(["-o", "comm=", "-p", &pid.to_string()])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| s.rsplit('/').next().unwrap_or(&s).to_string())
            .unwrap_or_else(|| "(not running)".into());
        format!("pid={pid} name={name:?}")
    }
}

#[cfg(target_os = "macos")]
pub use imp::{holder, is_enabled};
