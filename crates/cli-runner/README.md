# cli-runner

CliRunner (component R5, issue #16): the only way Bottle runs the Databricks CLI.
It finds the CLI at an absolute path, refuses unsafe locations, needs version 1.17
or later, checks the path and version again at each launch, and runs every CLI
child process with a clean environment.

It also installs the official CLI after consent (`install.rs`, #161).

## Requirements covered

| ID | What | Where |
|---|---|---|
| W-02a | Resolve the CLI to an absolute path (MDM `CliPath` when set; else the first absolute PATH entry with an executable `databricks` / `databricks.exe`; relative PATH entries are skipped; symlinks are resolved) | `resolve.rs` |
| W-02b | Refuse a CLI in a temporary directory (`std::env::temp_dir`, `TMPDIR`, `TEMP`, `TMP`, `/tmp`, `/var/tmp`, `/private/tmp`, `/private/var/tmp`, `C:\Windows\Temp`; raw and canonical paths) or a world-writable one (unix: o+w bit on the binary or its folder, as found and after symlinks) | `location.rs` |
| W-02c | Parse `databricks --version` (e.g. `Databricks CLI v1.25.0`); block versions below `CliMinVersion` (default 1.17.0); warn below the recommended 1.25.0 | `version.rs`, `runner.rs` |
| W-02d | `launch_check(previous)` re-resolves and re-reads the version at every launch and reports `PathChanged` / `VersionChanged` warnings; `run` re-checks that the verified file still exists in a safe place | `runner.rs` |
| W-17b, W-17d, W-17e, SEC-20b | `install::install(plan, fetch, consent)`: no fetch without consent; the pinned checksum file `databricks_cli_1.26.0_SHA256SUMS` is checked against `PINNED_SUMS_SHA256`, then the OS/CPU zip against its line in that file, before the unpack; only the top-level `databricks` entry is unpacked into the Bottle bin folder (macOS `~/Library/Application Support/Bottle/bin`, Windows `%LOCALAPPDATA%\Bottle\bin`), mode 0755; every failure deletes the download folder. The checks are `artifact-verify` (#160, PR #257). ADR 0099. | `install.rs` |
| W-17f | `resolve` looks in the Bottle bin folder first (after `CliPath`), so an older CLI on PATH stays unchanged | `resolve.rs` |
| W-09 | Every child (including `--version`) gets `env_clear()` plus an allowlist: PATH, HOME, `DATABRICKS_CONFIG_FILE`, `XDG_CONFIG_HOME`, proxy (`HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`, `ALL_PROXY`, both cases) and CA vars (`SSL_CERT_FILE`, `SSL_CERT_DIR`, `REQUESTS_CA_BUNDLE`, `CURL_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`). Windows adds `USERPROFILE`, `APPDATA`, `LOCALAPPDATA` (home/config folders) and `SYSTEMROOT` (needed by the network stack); names match case-insensitively there. All else, including `DATABRICKS_TOKEN`, `DATABRICKS_HOST`, `DATABRICKS_CLIENT_*`, `DATABRICKS_AUTH_TYPE` and `DATABRICKS_CONFIG_PROFILE`, is dropped. | `env.rs`, `sys.rs` |

## API

```rust
use cli_runner::{CliPolicy, CliRunner};

let mut runner = CliRunner::system(CliPolicy::default()); // MDM CliPath / CliMinVersion go in CliPolicy
let report = runner.launch_check(stored_record.as_ref())?; // at each app launch
// persist report.record; show report.warnings (CLI_PATH_CHANGED, CLI_VERSION_CHANGED, CLI_BELOW_RECOMMENDED)
let status = runner.status();                              // {found, path, version} for cli_status / Settings
let out = runner.run(["auth", "profiles", "--output", "json"])?; // clean env, verified path
```

Seams: `Fs`, `Env`, `Process` traits (real: `RealFs`, `RealEnv`, `RealProcess`).
Lower-level functions: `resolve`, `check_location`, `temp_roots`, `path_within`,
`clean_env`, `clean_env_for`, `is_allowed`, `Version::parse_cli_output`.

Errors (`CliError::code()`): `CLI_MISSING` (not found, or `CliPath` not absolute),
`CLI_UNSAFE_PATH`, `CLI_UNSUPPORTED`, `CLI_VERSION_UNREADABLE`, `CLI_NOT_VERIFIED`,
`CLI_SPAWN_FAILED`. `backend_code()` maps every one to the section 4.7
`BackendErrorCode` `CLI_MISSING` (the CLI is not usable).

## How to test

```sh
cd crates/cli-runner
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
```

Most tests use the fakes in `tests/common`. `tests/real_process.rs` runs a real
child: `src/bin/fake-databricks.rs` (test fixture, never shipped) is copied into a
per-test folder under `target/real-cli-fixtures` in this crate folder (git-ignored,
removed after each test). That folder is not a temp root even when
`CARGO_TARGET_DIR` is, so no test-only override of W-02b exists. Its version comes
from a `fake-version.txt` next to it, because the clean env drops any variable a
test could set. One test copies it into the OS temp dir and expects refusal. If the
checkout itself is inside a temp directory, the real-process tests stop with a
clear message, by design.

## Out of scope

- `cli_status` / `cli_install` IPC commands and the Settings rows: src-tauri skeleton (#221) and the settings/IPC issues wire this crate in.
- `plaintextStorage` detection and PAT/SP profile rejection: profile/auth work (ProfileService, D39).
- CLI download and install after consent (U16).
- JSON parsing of CLI output and timeouts: callers parse `ProcOutput.stdout`.
- Windows ACL checks for world-writable folders: Windows gets the temp-directory rule only; mode bits are unix-only.
- Persisting the `CliRecord` between launches: the caller stores it.
