# ADR 0104: The hub's `stt_model_*` commands and the packaged local-stt build

- Status: Accepted
- Date: 2026-11-19
- Issues: #413 (also #45 and #529)
- Owner: impl-413

## Context

#45 built the backend of the local Parakeet model install in `crates/mog-core/src/stt_model.rs`
(`ModelInstaller`, `ConsentGate`, content-free error codes) and `crates/mog-core/src/stt_setup.rs`
(`SttSetup`: the W-17e models folder, the stored consent, one install or verify at a time, cancel).
#413's wizard step component (`SttSetupStep`, now `SttModelStep`) was built against a
`SttModelApi` interface of five calls (`status`, `setConsent`, `install`, `verify`, `cancel`), with
a `TODO(#529)` for the real `stt_model_*` hub commands and a placeholder
(`unwiredSttModelApi`) that always answered `stt_setup_unavailable`. #529 (the commands
themselves) was never opened as separate rows on main, so the hub's Local speech step shipped
with no way to ever install the model.

Two more gaps blocked "a new user who installs Bottle can get local dictation with no developer
command" (the #413 goal):

- The `local-stt` feature is off by default (App CI has no vendored sherpa-onnx, #534, #414), and
  stays off in every packaged build unless something turns it on.
- `stt_transcribe`'s `stt_model_missing` reason was not in `ipc_gateway::ERROR_CODES`, so a
  dictation attempt with no installed model crossed to the brain as `host_failed`: the MOG
  pop-up showed the generic "Could not transcribe. Retry", which a Retry alone cannot fix.

## Decisions

1. **Five new rows, hub-only.** `stt_model_status`, `stt_model_consent`, `stt_model_install`,
   `stt_model_verify` and `stt_model_cancel` are new IPC rows (not in 4.5), each a thin,
   one-to-one wrapper of an `SttSetup` method. They are `hub`-only, the same shape as
   `view_notice_subscribe` (ADR 0091): the hub's `SttModelStep` calls them directly (`call()`
   from `src/shared/ipc.ts`), not through the brain. `stt_model_install`'s progress Channel
   (`onProgress`) therefore always resolves on the hub webview
   (`stt_model_host::hub_progress_resolver`, mirroring `capture_host::brain_level_resolver`,
   ADR 0067), with no extra hop through the brain.
2. **`stt_model_verify` stays separate from `install`.** `SttSetup::install` already no-ops (no
   network) when the files on disk match their pins, but `verify` is the one call that is
   documented, and tested, to never send a request even when consent was never asked. The
   launch/mount check in `SttModelStep` calls `verify`, never `install`, so "never download again
   what is on disk and verifies" holds even if a future edit reorders the component's checks.
3. **Content-free codes cross as themselves.** `mog_core::ipc_gateway::ERROR_CODES` gains the
   nine `stt_setup::SetupError` / `stt_model::InstallError` codes, `stt_setup_unavailable` (the
   host-side code for a build with no `local-stt` feature) and `stt_model_missing` (so
   `stt_transcribe`'s existing reason finally crosses instead of becoming `host_failed`). Each
   gets a matching `IpcError` variant (`SttConsentRequired`, `SttHashMismatch`, ...), the same
   registry shape as `CliFailed`/`ProfileExists`/`AgentUnavailable`: the component still raises
   `CoreError::Unavailable { component: "R4", reason: code }` directly (`stt_model_host.rs`), the
   `IpcError` variant exists only so the fixed vocabulary stays one-to-one with `ERROR_CODES`
   (`ipc_gateway_hardening.rs`'s exhaustiveness test).
4. **The host, `src-tauri/src/stt_model_host.rs`.** One `SttModelHost` per process wraps the one
   managed `SttSetup` (`SttSetup::for_this_device`), built at app setup next to `install_capture`.
   Without `local-stt`, `status`/`verify`/`cancel` still answer from the files on disk (pure
   hashing, no network: `mog_core::stt_model`'s hashing has no feature gate), but `install`
   answers `stt_setup_unavailable` at once, before it touches `SttSetup` at all: this build has no
   HTTP client for the download, because `mog_core::stt_local` (the real `RangeFetch` and the
   archive extractor) is itself gated on the feature. `stt_model_install` runs on a worker thread
   (`runs_off_main_thread`): it can take minutes over the network.
5. **The packaged macOS DMG always builds with `local-stt`.** `scripts/package-macos.sh` vendors
   the pinned sherpa-onnx archive (CAP-50a, idempotent, skipped when the archive is already
   there) and adds `--features local-stt` to the `tauri build` call. App CI (`.github/workflows/
   app-ci.yml`) is unchanged and stays off the feature (#534, #414): this is a packaging-time
   decision only, documented in `AGENTS.md` section 4.
6. **The MOG pop-up gets a clear, separate message.** `src/brain/components/stt-provider.ts`
   tracks a `local_parakeet` failure whose code is `stt_model_missing`, and (with no Whisper
   fallback configured) raises a new `SttFailure` kind `model_missing` instead of the generic
   `could_not_transcribe`, with its own fixed text pointing at the hub's Download step. It carries
   no path, no profile and no transcript text (SEC-03); the Download button itself lives only in
   the hub's `SttModelStep`.
7. **One component, for the hub's own composition.** The step is exported as `SttModelStep`
   (`src/views/hub/SttModelStep.tsx`, renamed from `SttSetupStep`), a self-contained
   `<section>` with no assumption about where it mounts, and `sttModelApi`
   (`src/views/hub/sttModelApi.ts`) is its real `SttModelApi` over the five commands. `Hub.tsx`
   passes `api={sttModelApi}`; a later hub layout (a Setup region, or Settings > Local speech) can
   mount the same component elsewhere without touching its logic.

## Consequences

- `src/views/arc2.test.ts` now allows seven `view_*`-shaped direct calls: `view_emit`,
  `view_notice_subscribe` and the five `stt_model_*` rows.
- `crates/mog-core/tests/ipc_contract.rs`, `ipc_gateway_hardening.rs`,
  `src-tauri/tests/host_routing.rs` and `src-tauri/tests/ipc_capabilities.rs` (regenerated
  `src-tauri/schemas/ipc-commands.json`) all name the five rows.
- `src/test-support/cloud-calls.ts` classes `stt_model_install` as `cloud` (it downloads the
  pinned archive or files) and the other four as `local`.
- A future hub layout change (the Setup region work) only needs to move `<SttModelStep
  api={sttModelApi} />`; the component and the adapter do not change.
