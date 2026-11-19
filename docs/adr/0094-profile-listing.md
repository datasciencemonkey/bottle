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
     every other key by its name only and never parses its value. It reads one line at a time
     and zeroes the line buffer after each line. It never writes the file (W-08).
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
   - R5 answers `not_implemented` (only the `StubCliRunner` test double since #16): not checked.
     The token path uses the same R5, so no token can flow for that profile either.
   - Update (#16): a plaintext or no-mode reply sets the T7 flag that `cli_status` reports as
     `plaintextStorage`; a failed run blocks the profile but does not set the flag.
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

## Why the file and not `databricks auth profiles`

- The lead (agent-lead-2) directed this for issue #17: list through R5 `databricks auth profiles`, or parse only the
  section names and the `host` and `auth_type` keys of `~/.databrickscfg`. R5 was the stub (until #16), so only the second
  works now. The user direction for this wave is a demo that works.
- The read is not a credential source (W-01a): no token or secret is used, kept, logged or returned. Lead
  decision on PR #556: the file is read one byte at a time (an unbuffered `File`) into one fixed 4 KiB line
  buffer that is zeroed after each line. The whole file is never in memory, and a longer line is skipped. A
  secret line still passes through that one buffer, so this is a narrowed gap against "T7 never sees a token". `TODO(#17)`: R5 runs the CLI since #16, so list through
  `databricks auth profiles --skip-validate --output json`, and drop the file read.
- The reference plugin uses the same name-only scan (`config_sections`) for the same file.

## Review follow-ups (PR #556)

- The cache lock is not held while R5 runs `auth describe`, so R7 and R8 do not wait behind a refresh.
- `workspace_url` for a name that is not in the last list reads the file again once (a profile added by a sign-in).
- A behavioural test drives R8 on T7: a PAT, service-principal or plaintext-storage profile sends nothing and lends no token.
- The hub texts of W-03a and W-10a are #560 (`TODO(#560)`). This PR covers the host part of W-03a and W-10a.
- The `Unchecked` arm (R5 stub) does not block. Only `StubCliRunner` answers `NotImplemented`, so the arm ends
  with #16. Blocking there would hide every profile from the #163 picker until #16. No token flows through the
  stub either.

## Not in this ADR

- W-03c: the default `databricks-cli` client ID is set at `databricks auth login` (#162).
- The TS `ProfileServiceStub` (`src/brain/components/profile-service.ts`). The #163 brain code
  calls `profiles_list` directly.

## Consequences

- The UC2 demo can list the user's profiles as soon as #555 routes `profiles_list`.
- When R5 lands (#16), the storage check runs, and each list runs one `auth describe` for each
  `databricks-cli` profile. #555 also moves `profiles_list` off the main thread.
