//! The app-wide settings of Appendix F, scope App (#453).
//!
//! [`AppSettings`] keeps each app setting in memory and serves `app_setting_read` and
//! `app_setting_write` on the R12 gateway seam ([`GatewayComponents`]). The gateway lets only
//! the hub window call them. The only setting now is CAP-22 `app.capture.preferBuiltInMic`
//! (default off). A write passes the value to the mic through [`MicPreference`].
//!
//! TODO(#77): load and save the values in the settings store, so they last across launches.

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::error::{CoreError, CoreResult};
use crate::ipc::AppSettingReadArgs;
use crate::ipc_gateway::GatewayComponents;
use crate::types::{AppSetting, AppSettingKey};

/// Where CAP-22 goes: the mic device that picks the input (`MicDevice::set_prefer_built_in`,
/// #22).
pub trait MicPreference: Send + Sync {
    /// `true`: when a Bluetooth headset connects, record from the built-in mic.
    fn set_prefer_built_in(&self, on: bool);
}

#[derive(Default)]
struct Values {
    prefer_built_in_mic: bool,
}

/// The app-wide settings of the process, in memory.
#[derive(Default)]
pub struct AppSettings {
    values: Mutex<Values>,
    mic: Mutex<Option<Arc<dyn MicPreference>>>,
}

impl AppSettings {
    /// The current value of `key`.
    pub fn read(&self, key: AppSettingKey) -> AppSetting {
        let v = self.values.lock().unwrap_or_else(|e| e.into_inner());
        let value = match key {
            AppSettingKey::CapturePreferBuiltInMic => v.prefer_built_in_mic,
        };
        AppSetting { key, value }
    }

    /// Keep `setting`, apply it, and return the value now in force.
    pub fn write(&self, setting: AppSetting) -> AppSetting {
        match setting.key {
            AppSettingKey::CapturePreferBuiltInMic => {
                let on = setting.value;
                self.values
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .prefer_built_in_mic = on;
                if let Some(mic) = self.mic() {
                    mic.set_prefer_built_in(on);
                }
            }
        }
        self.read(setting.key)
    }

    /// Connect the mic. It gets the current value at once, then each write.
    pub fn set_mic_preference(&self, mic: Arc<dyn MicPreference>) {
        mic.set_prefer_built_in(self.read(AppSettingKey::CapturePreferBuiltInMic).value);
        *self.mic.lock().unwrap_or_else(|e| e.into_inner()) = Some(mic);
    }

    fn mic(&self) -> Option<Arc<dyn MicPreference>> {
        self.mic.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

fn refused(code: &'static str) -> CoreError {
    CoreError::IpcRefused { code }
}

impl GatewayComponents for AppSettings {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        let reply = match command {
            "app_setting_read" => {
                let a: AppSettingReadArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                self.read(a.key)
            }
            "app_setting_write" => {
                let s: AppSetting =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                self.write(s)
            }
            _ => return Err(refused("not_implemented")),
        };
        serde_json::to_value(reply).map_err(|_| refused("host_failed"))
    }
}
