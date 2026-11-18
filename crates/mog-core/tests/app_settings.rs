//! #453 (CAP-22): the app-wide settings store and its gateway component.

use std::sync::{Arc, Mutex};

use mog_core::app_settings::{AppSettings, MicPreference};
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::types::{AppSetting, AppSettingKey};
use serde_json::json;

#[derive(Default)]
struct FakeMic(Mutex<Vec<bool>>);

impl MicPreference for FakeMic {
    fn set_prefer_built_in(&self, on: bool) {
        self.0.lock().unwrap().push(on);
    }
}

#[test]
fn cap_22_prefer_built_in_mic_is_off_by_default() {
    let s = AppSettings::default();
    assert_eq!(
        s.read(AppSettingKey::CapturePreferBuiltInMic),
        AppSetting {
            key: AppSettingKey::CapturePreferBuiltInMic,
            value: false
        }
    );
}

#[test]
fn cap_22_a_write_is_read_back_and_reaches_the_mic() {
    let s = AppSettings::default();
    let mic = Arc::new(FakeMic::default());
    s.set_mic_preference(mic.clone());
    assert_eq!(
        s.write(AppSetting {
            key: AppSettingKey::CapturePreferBuiltInMic,
            value: true
        }),
        AppSetting {
            key: AppSettingKey::CapturePreferBuiltInMic,
            value: true
        }
    );
    assert_eq!(
        s.read(AppSettingKey::CapturePreferBuiltInMic),
        AppSetting {
            key: AppSettingKey::CapturePreferBuiltInMic,
            value: true
        }
    );
    s.write(AppSetting {
        key: AppSettingKey::CapturePreferBuiltInMic,
        value: false,
    });
    // The hook gets the current value when it is set, then each write.
    assert_eq!(*mic.0.lock().unwrap(), vec![false, true, false]);
}

#[test]
fn cap_22_a_mic_set_after_a_write_gets_the_current_value() {
    let s = AppSettings::default();
    s.write(AppSetting {
        key: AppSettingKey::CapturePreferBuiltInMic,
        value: true,
    });
    let mic = Arc::new(FakeMic::default());
    s.set_mic_preference(mic.clone());
    assert_eq!(*mic.0.lock().unwrap(), vec![true]);
}

#[test]
fn cap_22_the_component_answers_the_two_commands() {
    let s = AppSettings::default();
    let key = "app.capture.preferBuiltInMic";
    assert_eq!(
        s.call("app_setting_read", json!({"key": key})).unwrap(),
        json!({"key": key, "value": false})
    );
    assert_eq!(
        s.call("app_setting_write", json!({"key": key, "value": true}))
            .unwrap(),
        json!({"key": key, "value": true})
    );
    assert_eq!(
        s.call("app_setting_read", json!({"key": key})).unwrap(),
        json!({"key": key, "value": true})
    );
}

#[test]
fn cap_22_the_component_refuses_bad_args_and_other_commands_with_fixed_codes() {
    let s = AppSettings::default();
    let code = |r: mog_core::error::CoreResult<serde_json::Value>| match r {
        Err(mog_core::error::CoreError::IpcRefused { code }) => code,
        other => panic!("expected a refusal, got {other:?}"),
    };
    assert_eq!(
        code(s.call(
            "app_setting_write",
            json!({"key": "app.capture.preferBuiltInMic", "value": "on"})
        )),
        "invalid_args"
    );
    assert_eq!(
        code(s.call("app_setting_read", json!({"key": "x"}))),
        "invalid_args"
    );
    assert_eq!(
        code(s.call("config_read", json!({"id": "a1"}))),
        "not_implemented"
    );
}
