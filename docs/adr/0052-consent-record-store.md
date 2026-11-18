# ADR 0052: Consent records in the local store, with a device-wide MEM-01 exemption

- Status: Accepted
- Date: 2026-11-16
- Issue: #63 (SEC-32a, SEC-15b, AT-24, AT-16, MEM-01, U9)

## Context

T12 ConsentService keeps the notice and attestation logic in the brain (section 4.3). SEC-32a needs each consent record in the local store, and the records must outlive the transcript. SEC-15b needs a 13-month expiry, and section 9.2 gives an admin range of 12-84 months. Migration 0001 (#35) has the `consent_record` table. Its `mog_id` is nullable because a notice acceptance has no MOG. The MEM-01 guard (`require_mog_id_filter`) accepts only statements with a `mog_id = ?` filter. The fixed statement catalog (#37) is not on main yet.

## Decisions

1. **The Rust module `mog_core::consent` keeps the records.** `ConsentRecords` writes and reads `consent_record` through the crate-private connection of `SqlCipherLocalStore`. It does not use `exec` and adds no IPC command. Its six statements are fixed and parameterised, with IDs `consent.*`, so #37 can move them into the catalog.
2. **Field names.** SEC-32a `user` is `owner_principal`, `device` is `device_id`, and `accepted_at` (9.3 `consent_ts`) is `recorded_at`. The notice version is stored as decimal text, because the column is `TEXT` and the IPC value is a number.
3. **MEM-01 with a NULL `mog_id`.** Each statement has a scope. The session statements (`consent.session.insert`, `consent.session.get`) are `Mog` scope, and a test runs `require_mog_id_filter` on them. The notice insert, the notice lookup, the device list and the purge are `DeviceWide` scope, each with a written reason. A `mog_id = ?` filter cannot match a NULL `mog_id`, so these statements cannot pass the guard honestly. A test checks that every consent statement touches only `consent_record`. The table holds no content, and no agent or MOG context reads it. The notice insert names `mog_id` and binds NULL. It passes the guard as text, but it is listed as `DeviceWide`, because it is not MOG-scoped.
4. **Expiry by the current retention.** `purge_expired` deletes rows with `recorded_at <= now - retention` (calendar months, UTC). So an admin change applies to old records too. `expires_at` is written as `recorded_at + retention` at write time, for the v1 Lakebase upload (SEC-32b). Where the day of the month clamps, a record can stay up to 3 days past its `expires_at`. It is never deleted early.
5. **Retention range.** `ConsentRetention::months` accepts 12-84 and refuses other values with `consent_retention_out_of_range`. It does not clamp. The default is 13. Appendix C has no key for this value yet, so the caller passes it (`TODO(#186)`).
6. **No automatic job.** The purge is a function. WipeService `run_security_expiries` (#124) calls it. There is no trigger and no timer (U9). No foreign key ties a record to a session, a transcript or a MOG, so a delete of those leaves the record.
7. **Wall clock.** `clock::WallClock` gives Unix milliseconds. `SystemWallClock` is the production clock, and `SimWallClock` is the test double. The monotonic clock of #229 is not a calendar time, so it does not apply.

## Consequences

- #37 must keep the `DeviceWide` statements out of `mem_local_exec`, or add an explicit device-wide entry for them. The MEM-01 test lists the statement IDs, so a new consent statement must state its scope.
- The first-run notice (#27) and `session_attest` (#64) need an IPC path to reach `ConsentRecords`. That path is contract change #430; the wiring is #431. Neither is in this ADR.
- An admin who shortens the retention deletes older records on the next purge.

## Amendment (#431, ADR 0071)

- The six statements are now the `local_store::catalog::CONSENT` entry set, with the same IDs and scopes. They have named, typed params. `consent::STATEMENTS` and `consent::Scope` are gone; the scope is `catalog::Scope`.
- The host opens the records at start with `consent::open_device_records` (store in `<app local data dir>/memory`, device ID in `<app local data dir>/device-id`). The brain reaches them through #430.
