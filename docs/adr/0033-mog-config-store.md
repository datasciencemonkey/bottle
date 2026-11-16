# ADR 0033: MOG configs as local JSON files

- Status: Accepted
- Date: 2026-11-16
- Issue: #25 (MB-02, AT-17)
- Owner: shell-lead (implemented by impl-25)

## Context

MB-02 requires the hub to save each MOG config as local JSON with no token, password, key or memory content. `config_write` must reject a token-like value or a key named token, password, secret or pat. Section 4.5 lists `config_list`, `config_read`, `config_write` and `config_delete` for the brain and the hub. The issue leaves these points open: the file layout, the JSON shape, the bounds, the error codes and the IPC route. `token_guard::check_config_json` (#224, ADR 0013) already holds the MB-02 detector. `mog_core::ipc::handle` and `src-tauri/src/ipc.rs` are contract files.

## Decisions

1. **One file per MOG.** The store keeps `<app config dir>/mogs/<id>.json`. The host finds the app config dir with the Tauri path API in Rust. No window gets the `fs` plugin (ARC-4). An ID has 1 to 64 lower-case ASCII letters, digits and `-`, and is not a Windows device name (`con`, `nul`, `com1` and so on). So an ID cannot hold a path separator or a dot, and two IDs never share a file on a case-insensitive file system. This rule is a subset of the window-label rule, so each config ID also names its `mog-<id>` window. A test checks this.
2. **Shape.** The JSON keys are the Appendix F `mog.*` keys without the `mog.` prefix, nested at each dot, in camelCase. Example: `mog.capture.vadEndMs` is `capture.vadEndMs`. `binding.backend` is the 4.7 `BackendBinding` union, tagged by `kind`, with the three G-01 kinds only. The config also has `schemaVersion: 1`. The ID is not in the JSON; it is the file name.
3. **Strict schema.** Every object level uses `deny_unknown_fields`, the backend variants too. `schemaVersion`, `name` and `binding.home` are required. Every other field gets its Appendix F default. A missing backend is `genie_one_mcp` on the home profile. `write` returns the saved config with all defaults filled in. The shared fixture `crates/mog-core/tests/fixtures/mog-config-v1.json` is the default config, and the Rust and TS tests both read it.
4. **Bounds.** A config is at most 32 KiB as compact JSON: the request, and the saved file with its defaults filled in. The store writes compact JSON, and `write` checks the saved bytes too, so every saved config passes the same cap on `read`. `name` has 1 to 64 characters. `persona` has at most 2,000 characters (Appendix F). Profile names, IDs and model names have 1 to 256 characters and no control characters. `profileAliases` has at most 32 entries. Numbers have ranges: `dozeAfterMin` up to 1,440, `vadEndMs` and `previewMs` up to 60,000, `chord` 0 to 9, `autoHideSec` up to 3,600, `maxBlurbs` 1 to 10. Capture modes are unique. A `custom_agent` `appUrl` starts with `https://`; the full 4.6 rule 1 check runs when Bottle calls the app (v1). The store keeps at most 256 configs.
5. **Check order.** `write` checks the ID, the size, then the MB-02 secret check, then `schemaVersion`, the schema and the bounds. The secret check runs before the schema, so a secret under an unknown key gives the secret code. This module adds no second detector. `deny_unknown_fields` also covers the "key" part of MB-02: a field such as `apiKey` or `privateKey` is not in the schema, so it fails.
6. **Atomic write.** The store writes a temp file (`.<id>.json.tmp-<random>`) in the same folder with `create_new`, calls `fsync`, and renames it over `<id>.json`. On Unix it then syncs the folder. A rejected or failed write leaves the old file as it was. On Unix the folder is `0700` and the file is `0600`. One lock covers each write and delete.
7. **Read checks the file again.** `read` and `list` run the same checks on each file. A file that fails gets an error code and stays on disk. `list` puts its ID in `unreadable`. Bottle never deletes user content automatically. `delete` runs only when the user deletes a MOG in the hub.
8. **Content-free errors.** Errors are fixed codes: `invalid_config_id`, `config_too_large`, `secret_named_key`, `token_like_value`, `unsupported_schema_version`, `invalid_config`, `config_not_found`, `config_limit_reached` and `config_storage_failed`. They cross as `CoreError::Unavailable { component: "MogConfigStore", reason }`. An error never holds a path (the path has the user name), a key or a value. serde messages are dropped. Nothing logs.
9. **Replies.** `config_list` gives `{ items: [{ id, name }], unreadable: [id] }`. `config_read` and `config_write` give `{ id, json }`. `config_delete` gives `{ deleted }`. The 4.5 result types are `unknown`, so the contract does not change.
10. **IPC route.** `MogConfigComponent` serves `config_*` on the R12 seam `ipc_gateway::GatewayComponents`, behind the window and schema checks of `hardened_gateway_with`. The Tauri host still calls `ipc::handle` directly, and that answers `not_implemented`. Contract change #333 routes the host through the hardened gateway: `TODO(#333)`. The comment `TODO(#25)` in `ipc::handle` (contract file) then becomes stale, because the component does the check. #333 can remove it.

## Consequences

- #26 (hub MOG management) and T1 MogManager use `MogConfig` (`src/brain/components/mog-config.ts`) and the four commands, once #333 lands.
- A new field needs a schema change here, a fixture update and a TS type update. A field change that old files cannot read needs `schemaVersion: 2` and a migration.
- MB-13 (v1 export and import) can reuse `parse_config` for the import check.
