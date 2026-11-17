# ADR 0063: The mic fault in `capture.state` and the app setting `app.capture.preferBuiltInMic`

- Status: Accepted
- Date: 2026-11-17
- Issue: #453 for #22 (CAP-17b, CAP-22, CAP-01b, ARC-4, SEC-02a, SEC-03)
- Owner: contract-lead (implemented by impl-453)

## Context

#22 (ADR 0062) added the mic zero-buffer stop (CAP-17b) and the built-in mic preference (CAP-22) in the Rust core. The contract had no field for the health of the You track (the mic). `othersTrack` covers only the Others track. So the brain could not show the mic fix-it card. The contract also had no app-wide setting and no command for one. `config_*` is per MOG (ADR 0035). The issue left these points open: a field or an event, the setting name, the commands and their windows.

## Decisions

1. **A field, not an event.** `CaptureState` gets `mic?: MicHealth`. `capture.state` already goes to the brain after each change, so the brain gets the fault with the rest of the capture state. A separate `capture.fault` event would need its own order rules against `capture.state`.
2. **Fixed codes only (CAP-01b).** `MicHealth` is `ok`, `zero_buffers`, `permission_denied` or `no_device`. It holds no device name, no device ID and no audio. An object or a free string is a parse error on both sides.
3. **Optional.** Rust `Option<MicHealth>` with `skip_serializing_if`. A payload from before #453 still parses. Absent means that the core has no reading yet. `CaptureCore` sets it: `ok` at each granted lease, `zero_buffers` from `mic_silent` (`StopReason::ZeroBuffers`), `no_device` from `device_failed(Mic)`, and `permission_denied` or `no_device` when the mic open fails with that code. Any other open code gives no change. The TS field is an amendment after the 4.7 block (declaration merging, as ADR 0007 did for `leaseId`). The 4.5 and 4.7 blocks stay verbatim.
4. **The setting name follows Appendix F.** The App rows of Appendix F use the `app.` prefix. So the field is `app.capture.preferBuiltInMic` (boolean, default off). This ADR adds the row to Appendix F.
5. **Two brain-and-hub commands.** `app_setting_read { key }` and `app_setting_write { key, value }` are MVP-D rows for the `brain` and `hub` window classes, like the `config_*` and `hotkeys_set` settings rows. A view calls only `view_emit` (ARC-2, MOG-08a, `src/views/arc2.test.ts`), so the brain relays the Settings click. MOG and card windows cannot call them. Both replies are `AppSetting` `{ key, value }`, the value now in force. `AppSettingKey` is a closed list, so an unknown key is `invalid_args`. Every app setting is a boolean now, so the write args take `value: bool`. A later setting with another type makes `value` a typed union.
6. **Host routing.** `mog_core::app_settings::AppSettings` keeps the values in memory and is the gateway component. `HostComponents.app_settings` is an `Option<&dyn GatewayComponents>`, the same as `consent`. The app manages one `Arc<AppSettings>`. With no managed settings, the host answers `host_failed`. No new error code.
7. **The mic hook.** A write calls `MicPreference::set_prefer_built_in`. `set_mic_preference` gives the current value at once, so a mic that starts after a write gets the right value. `MicDevice::preference()` gives a `MicPreferenceHandle` that implements `MicPreference`. It shares the flag of the device, so it stays valid after the device moves into the capture core. The wiring is done: `install_capture` (#425) builds the `MicDevice` through `capture_host::mic_capture_device`, which calls `AppSettings::set_mic_preference(Arc::new(mic.preference()))` with the managed settings. The test is `cap_22_prefer_built_in_mic_setting_reaches_mic_device` (`src-tauri/tests/mic_preference_wiring.rs`).
8. **The hub.** `MicPreference` (ADR 0062) keeps its in-memory store. The Settings relay through the brain (`view_emit` to the brain, then `app_setting_*`) belongs to the Settings issue #77.

## Consequences

- The brain can show the mic fix-it card from `capture.state` alone.
- A future app setting adds one `AppSettingKey` value and one Appendix F row. A setting that is not a boolean also makes `AppSetting.value` a typed union.

## Open work

- `TODO(#77)`: the brain relays the hub `MicPreference` click to `app_setting_write` and gives the hub the stored value.
- `TODO(#77)`: load and save the app settings in the settings store, so the value lasts across launches.
