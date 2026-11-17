# ADR 0063: The mic fault in `capture.state` and the app setting `app.capture.preferBuiltInMic`

- Status: Accepted
- Date: 2026-11-17
- Issue: #453 for #22 (CAP-17b, CAP-22, CAP-01b, ARC-4, SEC-02a, SEC-03)
- Owner: contract-lead (implemented by impl-453)

## Context

#22 adds the mic zero-buffer stop (CAP-17b) and the built-in mic preference (CAP-22) in the Rust core. The contract had no field for the health of the You track (the mic). `othersTrack` covers only the Others track. So the brain could not show the mic fix-it card. The contract also had no app-wide setting and no command for one. `config_*` is per MOG (ADR 0035). The issue left these points open: a field or an event, the setting name, the commands and their windows.

## Decisions

1. **A field, not an event.** `CaptureState` gets `mic?: MicHealth`. `capture.state` already goes to the brain after each change, so the brain gets the fault with the rest of the capture state. A separate `capture.fault` event would need its own order rules against `capture.state`.
2. **Fixed codes only (CAP-01b).** `MicHealth` is `ok`, `zero_buffers`, `permission_denied` or `no_device`. It holds no device name, no device ID and no audio. An object or a free string is a parse error on both sides.
3. **Optional.** Rust `Option<MicHealth>` with `skip_serializing_if`. A payload from before #453 still parses. Absent means that the core has no reading. #22 sets the field from `CaptureCore::mic_fault()` and resets it to `ok` at the next granted lease. The TS field is an amendment after the 4.7 block (declaration merging, as ADR 0007 did for `leaseId`). The 4.5 and 4.7 blocks stay verbatim.
4. **The setting name follows Appendix F.** The App rows of Appendix F use the `app.` prefix. So the field is `app.capture.preferBuiltInMic` (boolean, default off). This ADR adds the row to Appendix F.
5. **Two hub-only commands.** `app_setting_read { key }` and `app_setting_write { key, value }` are MVP-D rows for the `hub` window class only. Settings in the hub is the only writer (CAP-22). The brain does not need the value: the host passes it to the mic. Both replies are `AppSetting` `{ key, value }`, the value now in force. `AppSettingKey` is a closed list, so an unknown key is `invalid_args`. Every app setting is a boolean now, so the write args take `value: bool`. A later setting with another type makes `value` a typed union.
6. **Host routing.** `mog_core::app_settings::AppSettings` keeps the values in memory and is the gateway component. `HostComponents.app_settings` is an `Option<&dyn GatewayComponents>`, the same as `consent`. The app manages one `Arc<AppSettings>`. With no managed settings, the host answers `host_failed`. No new error code.
7. **The mic hook.** A write calls `MicPreference::set_prefer_built_in`. `set_mic_preference` gives the current value at once, so a mic that starts after a write gets the right value.

## Consequences

- The brain can show the mic fix-it card from `capture.state` alone.
- A future app setting adds one `AppSettingKey` value, one `AppSetting` variant and one Appendix F row.

## Open work

- `TODO(#22)`: the core sets `CaptureState.mic`, and the host connects `MicDevice::set_prefer_built_in` with `set_mic_preference`.
- `TODO(#77)`: load and save the app settings in the settings store, so the value lasts across launches.
