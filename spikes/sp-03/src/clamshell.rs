// Adapted from cjpais/Handy src-tauri/src/helpers/clamshell.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! Lid state through the `ioreg` tool. The probe reads IOKit directly (`src/mac.rs`) and uses
//! this as a cross-check.

/// `AppleClamshellState` from `ioreg -r -k AppleClamshellState -d 4` text. `None` if absent
/// (for example a desktop Mac).
pub fn parse_clamshell_ioreg(ioreg: &str) -> Option<bool> {
    ioreg.lines().find_map(|l| {
        let v = l.split("\"AppleClamshellState\" = ").nth(1)?.trim();
        match v {
            "Yes" => Some(true),
            "No" => Some(false),
            _ => None,
        }
    })
}

/// Lid state from the `ioreg` tool: `Some(true)` closed, `Some(false)` open.
#[cfg(target_os = "macos")]
pub fn clamshell_ioreg() -> Option<bool> {
    let out = std::process::Command::new("ioreg")
        .args(["-r", "-k", "AppleClamshellState", "-d", "4"])
        .output()
        .ok()?;
    parse_clamshell_ioreg(&String::from_utf8_lossy(&out.stdout))
}
