# ADR 0045: macOS checks run on the developer Mac; App CI runs on Windows and Linux

- Status: Accepted
- Date: 2026-11-16
- Issue: #227 follow-up (NFR-13a, NFR-15a, NFR-15d, NFR-23a, SEC-26, AT-37)

## Context

GitHub-hosted `macos-15` runners held App CI jobs in the queue for hours. An earlier change ran macOS only on push to main, nightly and dispatch, but those runs also waited for hours. The `macos-14` image is near its end of life, so it is not a fallback.

## Decisions

1. **No App CI job runs on macOS.** The offline build and golden set jobs run on `windows-2025` and `ubuntu-24.04` on every event: pull_request, push to main, the nightly schedule and dispatch. The checks job runs on `windows-2025` on a pull_request, and on `windows-2025` and `ubuntu-24.04` on push to main, the nightly schedule and dispatch.
2. **A docs-only pull request skips App CI.** The pull_request trigger starts App CI only when a changed file is outside docs, research, markdown and `requirements.html`, or is a file that a test reads (for example `research/final-requirements.md`). `repo-checks.yml` still runs on every pull request. Push to main, the schedule and dispatch run everything.
3. **Linux is a CI leg, not a product target.** The Linux jobs install WebKitGTK 4.1, GTK 3, libsoup 3 and the app indicator library before the first cargo step. The code already has a non-macOS, non-Windows path (for example `hotkey_host.rs` and `keyboard_env.rs`). The custom-harness test `macos_panel_appkit` lists no tests off macOS. The package job builds the Windows NSIS installer only; it makes no Linux bundle.
4. **`scripts/pre-push.sh` is the macOS gate.** It runs the commands of the checks job, with the pinned cargo-nextest version from `app-ci.yml` and the requirement coverage report. It also runs the offline build (AT-37) and the adversarial golden set (SEC-26). It prints PASS or FAIL for each step and exits non-zero when one step fails.
5. **`scripts/package-macos.sh` makes the DMG.** It signs only when `APPLE_CERTIFICATE` and `APPLE_SIGNING_IDENTITY` are set. It notarizes and staples only when the DMG is signed and `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID` are set (NFR-15a). It writes a SHA-256 file for each DMG (NFR-15d). Its `--plan` mode prints the decision, and `scripts/tests/test_app_ci_workflow.py` tests that decision on every CI OS.

## Consequences

- A macOS regression is caught only by the local run of `scripts/pre-push.sh`. The developer must run it on a Mac before each push and paste its summary into the PR.
- No CI artifact contains a DMG. The release gate (rel-03, #170) must run `scripts/package-macos.sh` on a Mac with the Apple secrets.
- The Ubuntu checks leg runs after merge (push to main) and nightly, so a Linux-only break can reach main. The fix is a follow-up PR.
- The Linux leg adds coverage for the shared Rust and TS code. It does not test AppKit or the Windows hook code.
- To bring macOS back to CI, add a macOS runner to the matrices in `app-ci.yml`, and restore the macOS sign and notarize steps from `scripts/package-macos.sh`. Then update the tests in `scripts/tests/test_app_ci_workflow.py`.
