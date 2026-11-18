//! #453 (CAP-22, ADR 0063): `install_capture` connects the app setting
//! `app.capture.preferBuiltInMic` to the `MicDevice` that it gives to the capture core.
//!
//! The test uses a fake `MicHost`; no test opens the user's mic.

use bottle_lib::capture_host::mic_capture_device;
use mog_core::app_settings::AppSettings;
use mog_core::audio_device::{DataFn, DevicePreference, DiscardSink, FaultFn, MicDevice, MicHost};
use mog_core::capture::{AudioSource, DeviceError};
use mog_core::types::{AppSetting, AppSettingKey};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Log = Arc<Mutex<Vec<String>>>;

/// A host that records each preference and each build, and gives a stream that does nothing.
struct FakeHost(Log);

impl MicHost for FakeHost {
    type Stream = ();

    fn build(&mut self, _on_data: DataFn, _on_fault: FaultFn) -> Result<(), DeviceError> {
        self.0.lock().unwrap().push("build".into());
        Ok(())
    }

    fn play(&mut self, _stream: &()) -> Result<(), DeviceError> {
        Ok(())
    }

    fn set_preference(&mut self, preference: DevicePreference) {
        self.0
            .lock()
            .unwrap()
            .push(format!("prefer_built_in {}", preference.prefer_built_in));
    }
}

fn wait_for_builds(log: &Log, n: usize) -> Vec<String> {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let got = log.lock().unwrap().clone();
        if got.iter().filter(|l| *l == "build").count() >= n {
            return got;
        }
        assert!(Instant::now() < end, "no build {n}: {got:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn set(settings: &AppSettings, on: bool) {
    settings.write(AppSetting {
        key: AppSettingKey::CapturePreferBuiltInMic,
        value: on,
    });
}

#[test]
fn cap_22_prefer_built_in_mic_setting_reaches_mic_device() {
    let log: Log = Arc::default();
    let host_log = Arc::clone(&log);
    let settings = AppSettings::default();
    let spawned = MicDevice::spawn(move || FakeHost(host_log), Box::new(DiscardSink));
    let (device, link) = mic_capture_device(spawned, Some(&settings));
    assert!(link.is_some(), "a spawned mic gives its failure link");

    // A write after `install_capture` reaches the device that the capture core holds.
    set(&settings, true);
    device.open(AudioSource::Mic).expect("open");
    assert_eq!(wait_for_builds(&log, 1), ["prefer_built_in true", "build"]);
    device.close(AudioSource::Mic);

    set(&settings, false);
    device.open(AudioSource::Mic).expect("open again");
    assert_eq!(
        wait_for_builds(&log, 2)[2..],
        ["prefer_built_in false", "build"]
    );
    device.close(AudioSource::Mic);
}

#[test]
fn cap_22_a_setting_written_before_install_capture_reaches_mic_device() {
    let log: Log = Arc::default();
    let host_log = Arc::clone(&log);
    let settings = AppSettings::default();
    set(&settings, true);
    let spawned = MicDevice::spawn(move || FakeHost(host_log), Box::new(DiscardSink));
    let (device, _link) = mic_capture_device(spawned, Some(&settings));
    device.open(AudioSource::Mic).expect("open");
    assert_eq!(wait_for_builds(&log, 1), ["prefer_built_in true", "build"]);
    device.close(AudioSource::Mic);
}

#[test]
fn cap_22_a_mic_that_fails_to_spawn_is_the_unwired_mic() {
    let settings = AppSettings::default();
    let spawned: std::io::Result<(MicDevice, _)> = Err(std::io::Error::other("no thread"));
    let (device, link) = mic_capture_device(spawned, Some(&settings));
    assert!(link.is_none());
    assert_eq!(
        device.open(AudioSource::Mic),
        Err(bottle_lib::capture_host::UNWIRED_MIC)
    );
    // A write with no mic is still kept.
    set(&settings, true);
    assert!(settings.read(AppSettingKey::CapturePreferBuiltInMic).value);
}
