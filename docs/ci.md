# Continuous integration

Two workflows run on every pull request and on pushes to `main`:

| Workflow | File | What it checks |
|---|---|---|
| Repo checks | `.github/workflows/repo-checks.yml` | `tools/ci_checks.py`: requirements HTML, mascot source, brand strings, no build output. |
| Build and test | `.github/workflows/build-test.yml` (#227) | Build, lint and test every Rust and Node project, then requirement coverage. |

## Build and test jobs

| Job | What it does |
|---|---|
| Discover projects | `scripts/ci_discover.py` lists every Rust root (a `[workspace]`, or a crate with no enclosing workspace) and every Node root (a `package.json` that is not a workspace member). New crates and packages join CI with no workflow change. `research/`, `mog-rive/`, `target/` and `node_modules/` are skipped. |
| Script tests | `pytest scripts/tests` (the #182 coverage, ledger and checklist scripts, and discovery). |
| Rust (Linux, macOS, Windows) | Per root: `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -D warnings`, `cargo nextest run` with JUnit, `cargo test --doc`. For the Tauri workspace it installs WebKitGTK on Linux and writes a placeholder `dist/` for the Tauri context macro. |
| Node | Per root: pnpm (version from `packageManager`) or npm, then `lint`, `typecheck`, and `test:ci` (or `vitest` with JUnit, or `test`). |
| Requirement coverage | Downloads every JUnit file and runs `scripts/req_coverage.py` (#182). It fails on a test that names an unknown requirement ID. The report is in the job summary and the `requirements-coverage` artifact. |
| Build and test ok | Passes only if every job passed or was skipped. **Make this and `Repo checks / checks` the required status checks on `main`.** |

All actions are pinned to commit SHAs (AT-37).

## Name tests by requirement ID

See `docs/tests/README.md`. TypeScript: `it('[CAP-14b] stops hands-free on Esc', ...)`. Rust and Python: `fn cap_14b_stops_on_esc()`.

## Run it locally

```
uv run --no-project --with pytest pytest scripts/tests -q
python3 scripts/ci_discover.py
# each Rust root
cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo nextest run --workspace --all-targets --locked
# each Node root
pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test:ci     # or npm ci / npm test
uv run --no-project scripts/req_coverage.py --requirements research/final-requirements.md --junit <files> --out reports/requirements-coverage.md
```

Signing, notarization and release builds are not here: they stay in #227 and run on tags only.
