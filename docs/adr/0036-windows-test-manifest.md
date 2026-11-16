# ADR 0036: Rust tests in CI: a libtest harness for AppKit tests and a Windows manifest for every executable

- Status: Accepted
- Date: 2026-11-16
- Issues: #381 (Windows), #382 (macOS) (NFR-13a, NFR-23a, MOG-01c, MOG-02, MOG-03a, UX-19a)
- Owner: impl-ci-rust

## Context

The App CI `Checks` job runs `cargo nextest run --workspace --locked --profile ci` on macos-15 and
windows-2025. Both runs failed on main and on every PR. A failed Format step hid them until #359.

- **macOS (#382).** `src-tauri/tests/macos_panel_appkit.rs` has `harness = false`, because AppKit
  windows must be made on the main thread. Its `main` ran all checks and ignored the arguments.
  nextest lists tests with `--list --format terse` and needs one `<name>: test` line per test.
  So "creating test list failed" stopped the whole run.
- **Windows (#381).** Every `bottle` test executable failed to load with `0xc0000139`
  (STATUS_ENTRYPOINT_NOT_FOUND). `tauri` imports `TaskDialogIndirect` from comctl32 v6. Windows
  loads comctl32 v6 only for an executable with a Common Controls v6 application manifest.
  tauri-build puts that manifest in a resource that `embed-resource` links only into the app
  binary (`rustc-link-arg-bins`). The lib tests and integration tests get no manifest, so Windows
  loads comctl32 v5, which has no `TaskDialogIndirect`.

## Decision

1. **A small libtest protocol module, not a new crate.** `src-tauri/tests/support/libtest_protocol.rs`
   implements the part of the libtest command line that nextest and `cargo test` use:
   `--list` (with `--format terse`), filters, `--exact`, `--skip`, `--ignored` and
   `--include-ignored`. It accepts the other libtest flags and ignores them. No test in the harness
   is ignored, so `--ignored` selects nothing. Tests run one after another on the main thread.
   A failed or panicking test gives exit code 101, as libtest does. `tests/libtest_protocol.rs`
   tests the module with the argument lists that nextest and `cargo test` pass.
   We did not use `libtest-mimic`: it adds new crates (for example `clap`) to `Cargo.lock` for
   about 150 lines of code, and it runs tests on worker threads unless `--test-threads 1` is set.
2. **Each AppKit check is one test.** The 20 checks in `macos_panel_appkit.rs` keep their names.
   Each check is now a function that makes its own windows, so nextest can run it alone in its own
   process. A check that used a window from an earlier check now does the same setup step first.
   The check `ux_19a_panel_step_never_turns_a_mog_panel_into_a_key_panel` now also compares the
   full read-back with the Status panel values, so the read-back after a refused step is still tested.
   On other OSes the harness has no tests, so nextest lists nothing for it.
3. **The linker embeds the manifest in every Windows MSVC executable.** `src-tauri/build.rs`
   does these steps only when `CARGO_CFG_TARGET_OS` is `windows` and `CARGO_CFG_TARGET_ENV` is `msvc`:
   - It passes `WindowsAttributes::new_without_app_manifest()` to tauri-build, so the app binary
     gets no second manifest resource. tauri-build still adds the icon and version resources.
   - It emits `cargo:rustc-link-arg=/MANIFEST:EMBED` and
     `cargo:rustc-link-arg=/MANIFESTINPUT:<src-tauri/windows-app-manifest.xml>`. `rustc-link-arg`
     applies to every linked target of the crate: the app, the lib tests and the integration tests.
   - It emits `cargo:rustc-link-arg=/WX`, so a linker warning (for example a manifest conflict)
     fails the build.
   This is the documented fix that tauri uses for its own tests (`embed_manifest_for_tests` in the
   tauri 2.12.5 `build.rs`, from tauri PR #4383). `src-tauri/windows-app-manifest.xml` is a copy of
   the tauri-build 2.7.4 default manifest (`src/windows-app-manifest.xml`), so the app binary keeps
   the same manifest content as before.
4. **Loopback reads see `close` on Windows too.** With the manifest in place, the Windows run
   reached the tests and `arc_7_rule_9_close_ends_open_connections` failed: `close` waited 4.8 s.
   `LoopbackSession::close` shuts each open socket. That wakes a blocked read on macOS and Linux,
   but not on Windows, so the connection thread waited for its 5 s request deadline.
   `read_some` in `crates/mog-core/src/loopback.rs` now waits in slices of 50 ms and checks the
   session `stop` flag between them. The unit test
   `arc_7_rule_9_a_blocked_read_returns_soon_after_stop_without_a_shutdown` proves this on every OS.
5. `tests/windows_manifest.rs` checks the manifest file and the build script on every OS. On
   Windows it also reads the manifest resource (ID 1, `RT_MANIFEST`) of the running test executable.

## Alternatives not chosen

- `cargo:rustc-link-arg-tests` only: it applies to integration tests, not to the lib unit test
  executable (`bottle_lib-<hash>.exe`), which also failed.
- The `embed-manifest` crate: one more build dependency for the same two linker arguments.
- Exclude the AppKit target or the Windows test executables from nextest: this hides tests.

## Consequences

- The `Checks` job lists and runs every Rust test on both OSes. The JUnit report now contains the
  20 AppKit tests, so the requirement coverage report counts their requirement IDs.
- A GNU (`windows-gnu`) build keeps the tauri-build default manifest. The app supports MSVC only.
- If tauri-build changes its default manifest, update `src-tauri/windows-app-manifest.xml` too.

## Amendment (2026-11-16, #35)

`/WX` stays on. The build script adds one exception, `/IGNORE:4099`. The vendored OpenSSL that
SQLCipher needs (#35, SP-21, ADR 0033) is a static library whose objects name an
`ossl_static.pdb` that `openssl-src` does not ship, so `link.exe` gives LNK4099 ("PDB not found")
for each object, and `/WX` turned that into LNK1218. LNK4099 is about third-party debug info only.
A test (`nfr_13a_build_script_ignores_only_the_missing_vendored_pdb_warning`) checks that 4099 is
the only ignored warning.
