# ADR 0047: Hub MOG management and the running cap

- Status: Accepted
- Date: 2026-11-16
- Issue: #26 (MB-01, MB-03a, MB-03b, AT-01)
- Owner: shell-lead (implemented by impl-26)

## Context

MB-01 requires the hub to create, edit, duplicate, start, stop and delete MOGs without a manual file edit. MB-03a allows any number of defined MOGs. MB-03b limits the running MOGs to 8 by default (`app.maxRunningMogs`) with the text "8 MOGs are running. Stop one to start <name>." The configs are the local JSON files of #25 (ADR 0035). The R11 `WindowLedger` (ADR 0008) already refuses a 9th MOG window with `mog_cap_reached`. The issue leaves these points open: the action semantics, the delete confirm, the name of a copy, the cap values and the hub IPC.

## Decisions

1. **The brain owns the logic (ARC-2).** T1 MogManager (`src/brain/components/mog-manager.ts`) runs every action. It uses two seams over `call`: `MogConfigClient` (`config_list`, `config_read`, `config_write`, `config_delete`) and `MogWindowClient` (`window_mog_open`, `window_mog_close`). The host does not route `config_*` yet (`TODO(#379)`). PR #376 (#285, ADR 0040) routes `window_*` to R11, and R11 refusals cross as `{ "error": "<code>" }`, for example `mog_cap_reached`. Tests use the doubles in `src/test-support/mog-doubles.ts`.
2. **One action at a time.** MogManager runs its actions on one `BrainActor` queue. So two starts at the same time cannot pass the cap.
3. **Running cap.** The brain refuses a start before it calls the host when the running count is at the cap. The text is "<cap> MOGs are running. Stop one to start <name>." For a cap of 1 the text is "1 MOG is running. Stop it to start <name>." The cap is `app.maxRunningMogs`, 1 to 8. Any other value gives the default 8. The Rust host cap stays 8 (`windows::MAX_ACTIVE_MOGS`), so the brain cap cannot pass it. A cap above 8 needs a host change first. A `mog_cap_reached` refusal from the host gives the same text with 8. No settings store holds `app.maxRunningMogs` yet: `TODO(#77)`.
4. **Run state.** `MogRunState` is `stopped` or `running`. A MOG is `running` while its `mog-<id>` window is open. The state is in brain memory only, so after a restart no MOG runs. A window that closes outside the brain, or a brain reload, can make the state drift from the R11 ledger; the host cap still holds (`TODO(#419)`). `mog.autoStart` is not applied yet. #98 (MoodMapper lifecycle) and #50 (badges, ADR 0029) read `runState` and `onChange`.
5. **Create.** Create takes a name and a home profile. An empty name gives the lowest free "MOG <n>" (Appendix F). The config gets the Appendix F defaults, the `genie_one_mcp` backend on the home profile and a new UUID v4 ID. The UUID fits the ADR 0035 ID rule. The home profile is a text field until ProfileService lists profiles (`TODO(#17, #163)`).
6. **Edit.** Edit changes the name, the home profile and the persona instructions, and keeps every other field. When the backend used the old home profile, it moves to the new one. The brain checks the name (1 to 64 characters), the home (1 to 256 characters) and the persona (2,000 characters at most) before it reads the config. The core checks them again.
7. **Duplicate.** The copy gets a new ID and the name "<name> copy", cut to 64 characters. It gets no per-MOG chord, so two MOGs never share a chord (MB-14). It starts stopped.
8. **Delete.** The hub shows a confirm dialog first. The brain refuses a delete with no `confirmed: true`. A running MOG stops first. Delete removes only the config file. Memory and history of the MOG stay until Settings > Wipe, and the hub says so. Bottle deletes nothing else (no automatic deletion of user content).
9. **Hub IPC.** The hub cannot call `config_*` (ARC-2 test: views call only `view_emit`). The brain sends the list on the view-local event `hub:mogs` to the `hub` window only. The hub sends `view_emit` kind `click` with mogId `hub` and payload `{ action: 'mog.*' | 'mogs.ready', ... }`. This is the #27 pattern. Contract issue #407 moves the names into `src/shared/ipc.ts` (`TODO(#407)`).
10. **Content-free errors.** A refusal is a `MogManagerError` with a fixed code and fixed hub text. The text can hold the MOG name, which the user typed. It never holds a config value, a path or a host message.
11. **Accessibility (#96).** Every action is a native button with a name such as "Start Sales". Form fields have labels. The running count is in a `status` region, an error in an `alert` region and an info line in a polite live region. The delete dialog is an `alertdialog`, focuses Cancel, and Esc closes it. Esc also closes the edit form. Tab stays inside the dialog. When the form or the dialog closes, focus goes back to the button that opened it; after a delete it goes to the list heading. Each new notice gets a new alert element, so the same text twice is read again.

## Consequences

- When #379 and #285 land, the actions work in the app with no change to MogManager.
- MB-13 (v1 import and export) can add `import` and `export` to MogManager.
- #43 (wizard) can call `create` for the first MOG.
