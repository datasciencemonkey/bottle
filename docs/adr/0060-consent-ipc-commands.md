# ADR 0060: IPC commands for the consent records

- Status: Accepted
- Date: 2026-11-17
- Issue: #430 (SEC-32a, SEC-15b, AT-24, ARC-4, SEC-02a, SEC-03)
- Owner: contract-lead (implemented by impl-430)

## Context

ADR 0052 (#63) put the consent records in the local store (`mog_core::consent::ConsentRecords`). No IPC command reached them. So the brain ConsentService kept the notice record in Web Storage. The hub, MOG and card views share that origin and could write the record (#63, impl-27 comment). #431 wires the brain. It needs a command to write a notice record and a command to read the records. The issue left these points open: the windows, the reply shapes, the error code and the source of the user.

## Decisions

1. **Two rows, brain only.** `consent_notice_record` and `consent_records_list` are MVP-D rows for the `brain` window class only. T12 ConsentService is in the brain (4.3, COL-89). The hub shows the notice, and the brain takes the Continue click. A hub row would give a view the write reach that #63 removes (SEC-02a, ARC-4). A hub consent list gets its data from the brain.
2. **The caller names only the notice version.** `consent_notice_record { noticeVersion }` uses `deny_unknown_fields`. A `user`, `device`, `acceptedAt` or `mogId` key is `invalid_args`. Version 0 is `invalid_args`. `consent_records_list` takes no arguments.
3. **Rust fills the user, the device and the time (SEC-32a).** `consent_ipc::ConsentComponent` calls `ConsentRecords::record_notice`. The records give the device ID and the wall-clock time. The user comes from the `SignedInUser` seam. `NoSignedInUser` gives `None`. A notice accepted before the first sign-in has no user, as in 9.3 step 5.
4. **Content-free replies.** `consent_notice_record` returns `{ recordId }`. `consent_records_list` returns `{ records: ConsentRecordView[] }`, oldest first. `ConsentRecordView` is `{ recordId, kind, noticeVersion, sessionId, acceptedAt }`. `kind` is `notice`, `only_me` or `others_agreed`, which is the brain `ConsentRecord.kind`. `sessionId` is required and `null` for a notice. `acceptedAt` is Unix ms. No user, device or MOG ID crosses IPC. `mog_core::types` has the serde mirror, and shared fixtures check both sides.
5. **One new code: `consent_storage_failed`.** The host answers it when no consent records are managed. The component answers it when a store read or write fails. It is in Rust `ERROR_CODES`, TS `IPC_ERROR_CODES` and the schema document.
6. **Host routing.** `HostComponents.consent` is an `Option<&dyn GatewayComponents>`, the same as the `config_*` routing of #379. The host gets the records with `try_state::<Arc<ConsentRecords>>()`. The commands run inline on the IPC thread: each call is one short SQLite statement.

## Consequences

- No view can write or read a consent record. The brain must relay a hub list.
- The SEC-15b purge (WipeService, #124) also removes a record from the list.

## Open work

- `TODO(#431)`: manage `Arc<ConsentRecords>` in the app (open the local store, make the device ID once), and give the signed-in user from the T7 profile state. Until then both commands answer `consent_storage_failed` in the app.
- `TODO(#64)`: `session_attest` writes the session records. This ADR adds no session write command.
