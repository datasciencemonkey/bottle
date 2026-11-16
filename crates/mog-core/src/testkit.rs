//! Gate fixtures for live-workspace and device tests (#229, AT-22, NFR-23b).
//!
//! Live and device tests read the environment variables below. When a variable is not set, the
//! test skips with a clear message and does not pass silently as if it ran. A malformed value is
//! an error, so a wrong setup cannot look like a skip. The interface is in `docs/tests/README.md`.
//! The fixtures themselves (devices, the `bottle-test` U2M profile, the HID rig) need a person:
//! TODO(#270).
//!
//! This module never reads a token. It reads a profile name only; tokens come from
//! `databricks auth token --profile <p>` at run time.

use std::fmt;

/// The Databricks CLI profile for live tests (U2M, no Lakebase).
pub const ENV_PROFILE: &str = "BOTTLE_TEST_PROFILE";
/// An optional SQL warehouse ID for live tests.
pub const ENV_WAREHOUSE_ID: &str = "BOTTLE_TEST_WAREHOUSE_ID";
/// The reference device that runs a device test.
pub const ENV_DEVICE: &str = "BOTTLE_TEST_DEVICE";
/// The serial port of the Pi Pico HID rig (SP-03).
pub const ENV_HID_PORT: &str = "BOTTLE_TEST_HID_PORT";

const HELP: &str = "see docs/tests/README.md";

/// The result of a fixture lookup: ready, or skip with a reason to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fixture<T> {
    /// The fixture is configured.
    Ready(T),
    /// The fixture is not configured; the test must skip and print the reason.
    Skip(String),
}

/// A fixture variable has a value that is not valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureError(String);

impl fmt::Display for FixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FixtureError {}

/// The live-workspace fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveWorkspace {
    /// The CLI profile name. Never a token.
    pub profile: String,
    /// The optional SQL warehouse ID.
    pub warehouse_id: Option<String>,
}

/// The reference devices of section 15.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceDevice {
    /// Apple M1 8 GB on macOS 14.4 (`mac-m1-8gb-14.4`).
    MacM1_8Gb14_4,
    /// Apple M1 on the current macOS (`mac-m1-current`).
    MacM1Current,
    /// Windows 11, Intel Core i5, 16 GB (`win11-i5-16gb`).
    Win11I5_16Gb,
}

impl ReferenceDevice {
    /// The value of `BOTTLE_TEST_DEVICE` for this device.
    pub fn id(self) -> &'static str {
        match self {
            Self::MacM1_8Gb14_4 => "mac-m1-8gb-14.4",
            Self::MacM1Current => "mac-m1-current",
            Self::Win11I5_16Gb => "win11-i5-16gb",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        [Self::MacM1_8Gb14_4, Self::MacM1Current, Self::Win11I5_16Gb]
            .into_iter()
            .find(|d| d.id() == value)
    }
}

/// The Pi Pico HID rig fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HidRig {
    /// The device that the rig is plugged into.
    pub device: ReferenceDevice,
    /// The serial port, for example `/dev/cu.usbmodem1` or `COM5`.
    pub port: String,
}

fn value(lookup: &impl Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    lookup(key)
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

fn not_set(key: &str, what: &str) -> String {
    format!("skipped: {key} is not set, so {what} tests do not run ({HELP})")
}

fn valid_profile(p: &str) -> bool {
    p.len() <= 128
        && p.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Read the live-workspace fixture through `lookup` (for example `|k| std::env::var(k).ok()`).
pub fn live_from(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Fixture<LiveWorkspace>, FixtureError> {
    let Some(profile) = value(&lookup, ENV_PROFILE) else {
        return Ok(Fixture::Skip(not_set(ENV_PROFILE, "live-workspace")));
    };
    if !valid_profile(&profile) {
        return Err(FixtureError(format!(
            "{ENV_PROFILE} must be a CLI profile name (letters, digits, '-', '_', '.'); {HELP}"
        )));
    }
    Ok(Fixture::Ready(LiveWorkspace {
        profile,
        warehouse_id: value(&lookup, ENV_WAREHOUSE_ID),
    }))
}

/// Read the device fixture through `lookup`.
pub fn device_from(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Fixture<ReferenceDevice>, FixtureError> {
    let Some(raw) = value(&lookup, ENV_DEVICE) else {
        return Ok(Fixture::Skip(not_set(ENV_DEVICE, "device")));
    };
    ReferenceDevice::parse(&raw)
        .map(Fixture::Ready)
        .ok_or_else(|| {
            FixtureError(format!(
                "{ENV_DEVICE} must be one of mac-m1-8gb-14.4, mac-m1-current, win11-i5-16gb; {HELP}"
            ))
        })
}

/// Read the HID rig fixture through `lookup`. It needs a device and a port.
pub fn hid_rig_from(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Fixture<HidRig>, FixtureError> {
    let device = match device_from(&lookup)? {
        Fixture::Ready(d) => d,
        Fixture::Skip(reason) => return Ok(Fixture::Skip(reason)),
    };
    let Some(port) = value(&lookup, ENV_HID_PORT) else {
        return Ok(Fixture::Skip(not_set(ENV_HID_PORT, "HID rig")));
    };
    Ok(Fixture::Ready(HidRig { device, port }))
}

/// Read the live-workspace fixture from the process environment.
pub fn live() -> Result<Fixture<LiveWorkspace>, FixtureError> {
    live_from(|k| std::env::var(k).ok())
}

/// In a test that returns `()`: get the [`LiveWorkspace`], or print the skip reason and return.
/// A malformed value panics, so the test fails. With no argument it reads the process
/// environment; with a lookup closure it reads that.
#[macro_export]
macro_rules! live_or_skip {
    () => {
        $crate::live_or_skip!(|k: &str| ::std::env::var(k).ok())
    };
    ($lookup:expr) => {
        match $crate::testkit::live_from($lookup) {
            Ok($crate::testkit::Fixture::Ready(w)) => w,
            Ok($crate::testkit::Fixture::Skip(reason)) => {
                eprintln!("{reason}");
                return;
            }
            Err(e) => panic!("{e}"),
        }
    };
}
