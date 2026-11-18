# ADR 0071: The brain ConsentService and the first-run notice on the Rust consent records

- Status: Accepted
- Date: 2026-11-17
- Issue: #431 (SEC-32a, SEC-15b, AT-24, MB-08a, MB-08b, MEM-01, MEM-34a)

## Context

ADR 0052 (#63) put the consent records in the local store (`mog_core::consent::ConsentRecords`). ADR 0060 (#430) added the brain-only commands `consent_notice_record` and `consent_records_list`. The brain T12 ConsentService still kept the notice record in Web Storage (ADR 0044, decision 3). The hub, MOG and card views share that storage. The app opened no local store, so both commands answered `consent_storage_failed`. The issue left these points open: where the store and the device ID live, what happens to the old Web Storage record, how the brain reads the gate, where the consent statements go in the catalog of #37 (ADR 0054), and where the user comes from.

## Decisions

1. **T12 has no storage of its own.** `BrainConsentService` uses a `ConsentRecordsPort`. The production port calls the two #430 commands. The brain sends only the notice version. Rust fills the user, the device and the time (SEC-32a). `records()` returns the content-free `ConsentRecordView` list. `NoticeRecord` (with the user and the device) is gone from the brain, because these fields do not cross IPC.
2. **The gate reads the store each time.** `noticeState()` and `noticeAccepted()` call `consent_records_list` on each call, with no cache. So a record that the SEC-15b purge removes also closes the gate, and the notice shows again. A failed call, or a reply that is not the #430 shape, reads as "not accepted" (fail closed). Each call is one short SQLite query.
3. **One record per version.** `acceptNotice(version)` writes no record when a record of that version is already stored. Calls that overlap in one brain share one write, so a fast double click or two hubs write one record.
4. **The old Web Storage record is removed, not moved.** At start, T12 removes the key `bottle.consent.notice` from the brain's Web Storage. It does not copy the old record into the local store. A view could have written that record, and Rust stamps the time of a new record, so a copy would not be honest. The user accepts the current notice one more time.
5. **Where the files are.** The host (`src-tauri/src/consent_host.rs`) opens the one R10 store (MEM-08a) in `<app local data dir>/memory`, with the key from the OS key store (`os_key_vault`). The device ID is a random UUID v4 in `<app local data dir>/device-id`. It is made once with an exclusive create and is never overwritten. A file that does not hold a UUID fails with `device_id_invalid`. The local data dir does not roam on Windows. The host manages `Arc<SqlCipherLocalStore>`, so `mem_local_exec` (#426) can use the same store, and `Arc<ConsentRecords>`, the type that `ipc::route_from` reads.
6. **Fail closed.** If the data dir, the key store or the device ID is not available, the host manages nothing. Then `consent_*` answers `consent_storage_failed`, the notice stays open, and capture stays blocked (MB-08a). There is no plaintext or Web Storage fallback. An unsigned macOS build cannot use the data protection keychain (#34). So a person must run the notice check on a signed build.
7. **The consent statements are a catalog entry set.** The six statements move from `consent::STATEMENTS` into `local_store::catalog::CONSENT`, with the same IDs. Each entry is a catalog `Stmt` with named, typed params and `Access::Core`, plus a `Scope`. The two session statements are `Scope::Mog` and pass `require_mog_id_filter`. The notice insert, the notice lookup, the list and the purge are `Scope::DeviceWide`, each with its reason. `CONSENT` is not in `STATEMENTS`, so `find`, `LocalStore::exec` and `mem_local_exec` never run a consent statement. `consent.rs` checks the params with `catalog::check_params` (the catalog check with no `mogId` rule) and binds each `:name`.
8. **The user.** The `SignedInUser` seam of ADR 0060 gives the user. T7 ProfileService (#17) is not on main, and the host builds the component with `NoSignedInUser` in the contract file `src-tauri/src/ipc.rs`. So a notice record has no user yet. This is a known gap: 9.3 step 5 and SEC-32a need the user. Contract change #478 makes the host read a managed `SignedInUser`, and #431 stays open until #478 and #17 land.

## Consequences

- No consent record is in storage that a view can reach. The hub list of consent records must come from the brain (ADR 0060).
- Each capture start costs one `consent_records_list` call.
- Users of a #27 build see the notice once more after the update.
- `TODO(#478)`: the signed-in user in the records. `TODO(#64)`: session records through `session_attest`. `TODO(#186)`: the admin retention.
