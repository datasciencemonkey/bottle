# ADR 0094: T7 lists profiles from the CLI config file and blocks plaintext token storage

- Status: accepted
- Issue: #17 (W-01a, W-03a, W-03b, W-08, W-10a, W-10b)
- Date: 2026-11-18
- Owner: impl-17 (agent-lead-2)

## Context

The #163 profile pick card calls `profiles_list`. The host did not serve it. #17 asks for the
profile list, the `databricks-cli` filter and the token storage check. The issue left these
points open:

1. Where the list comes from. R5 `CliRunner` on main is still the stub (#16), so
   `databricks auth profiles` cannot run yet.
2. How a profile with plaintext token storage shows in `ProfileInfo`, which has no field for it.
3. What happens when the storage check cannot run.
4. Where the `profiles_list` reply is built, because the host routing is in a contract file.

## Decisions

1. The new module `mog_core::profile_service` reads the CLI config file that the CLI reads
   with the clean environment of R5: `~/.databrickscfg`, under `USERPROFILE` on Windows.
   `DATABRICKS_CONFIG_FILE` is not used, because R5 does not pass it to the CLI.
   - The parse keeps only the `[section]` names and the `host` and `auth_type` values. It finds
     every other key by its name only and never parses its value. It overwrites the file bytes
     with zeros after the parse. It never writes the file (W-08).
   - It follows the CLI's INI rules: keys before the first header belong to `DEFAULT`, a key
     ends at the first `=` or `:`, and a repeated section merges into the first one.
     `[__settings__]` and a section with no `host` are not profiles.
   - `host` becomes the bare `https://host[:port]` that R7 needs (ADR 0046).
   - No file means no profiles. An unreadable file is `host_failed`.
   - The file is read on the first call and on `refresh: true`. Other calls return the last list.
2. `supported` is true only when `auth_type` is `databricks-cli` (W-03a) and the token storage
   is not blocked (W-10a). PAT and service-principal profiles are listed with `supported: false`.
   A plaintext-storage profile shows `supported: false` too. The reason field for the W-10a hub
   warning text is an optional contract change (#555).
3. The storage check runs `databricks auth describe --profile <p> --output json` through R5 for
   each `databricks-cli` profile only. It never passes `--sensitive`. It reads only
   `token_storage.mode`:
   - `secure`: not blocked.
   - `plaintext`, another mode, no mode (an older CLI keeps tokens in a plaintext file) or a
     failed run: blocked. This follows W-10b, because MDM `RequireSecureTokenStorage` defaults
     to true.
   - R5 answers `not_implemented` (the stub on main): not checked, `TODO(#16)`. The token path
     uses the same R5, so no token can flow for that profile either.
   The profile stays blocked until a refresh finds `secure` storage (W-10b).
4. `ProfileService` is also R7 `ProfileHosts`. `workspace_url` answers only for a supported
   profile, else `profile_not_supported`. So R6, R7 and R8 never send for a PAT,
   service-principal or plaintext-storage profile (W-03b).
5. `src-tauri/src/profiles_host.rs` manages one `ProfileService` per app. `mcp_host::install`
   uses it as the profile hosts of R6, R7 and R8. The `profiles_list` route in
   `HostComponents` (`src-tauri/src/ipc.rs`) is the contract change #555 (`TODO(#555)`).
6. T7 never picks a profile, not even when only one exists. The user picks (#163).
7. W-01a: a source scan test fails if non-test code names the CLI token cache, a token or secret
   environment variable, or the config file outside T7.

## Not in this ADR

- W-03c: the default `databricks-cli` client ID is set at `databricks auth login` (#162).
- The TS `ProfileServiceStub` (`src/brain/components/profile-service.ts`). The #163 brain code
  calls `profiles_list` directly.

## Consequences

- The UC2 demo can list the user's profiles as soon as #555 routes `profiles_list`.
- When R5 lands (#16), the storage check runs, and each list runs one `auth describe` for each
  `databricks-cli` profile. #555 also moves `profiles_list` off the main thread.
