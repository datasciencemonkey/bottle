# Contributing to Bottle

Read `AGENTS.md` first. The single source of truth is `research/final-requirements.md`.

## Layout

| Path | What it is |
|---|---|
| `crates/mog-core` | Rust core: secrets, I/O, the IPC gateway (inside the secret boundary). |
| `crates/mog-core-ffi` | C ABI over `mog-core` (R15). |
| `src-tauri` | The Tauri v2 app crate `bottle`. It links `mog-core`. Capabilities per window label are in `src-tauri/capabilities/`. |
| `src/brain` | The hidden brain window: all domain logic (ARC-2). |
| `src/views` | Thin views: hub (V1), MOG (V2), card (V3). |
| `src/shared` | Shared contracts (IPC and interfaces). Only the foundation lead changes them. `interfaces.ts` is the 4.7 contract; its IPC-facing types have a serde mirror in `mog_core::types`. A change to one needs the same change to the other and to `tests/fixtures/ipc-types`. |
| `docs/adr` | Architecture decision records. |

## Branches

- One issue per branch and per PR: `issue/<n>-<slug>`, made from the latest `origin/main`.
- Use one worktree per issue: `../bottle-wt/issue-<n>-<slug>`.
- Never push to `main`. Never force-push `main`. Rebase on `origin/main` before each push.

## Commits

Use Conventional Commits with the issue number: `feat(<area>): ... (#<n>)`, `test(<area>): red for #<n>`, `refactor(...)`, `docs(...)`, `chore(...)`.
End every commit message with a blank line and this trailer:

```text
Co-authored-by: Isaac <no-reply@databricks.com>
```

## The TDD rule

1. Write the failing tests first. Commit them alone as `test(<area>): red for #<n>` and keep the failing output.
2. Write the least code that makes them pass. Commit `feat(<area>): ... (#<n>)`.
3. Refactor with the tests green.

No production code lands without a test that failed first. Paste the red and green output into the PR.

## The test-naming rule

Every test names its requirement IDs, so the coverage script can map tests to requirements:

- TypeScript: `it('[CAP-14b] stops hands-free on Esc', ...)`
- Rust: `#[test] fn cap_14b_stops_on_esc()` (the ID in lower case with `_`)
- Python: `def test_nfr_23a_...` with `[NFR-23a]` in the docstring
- Acceptance tests: `[AT-07]`

A test that names an ID that does not exist in `research/final-requirements.md` fails the coverage check.

## Run the checks locally

Prerequisites: Rust stable with `clippy` and `rustfmt`, Node 22 or later, pnpm 11, uv.

```sh
pnpm install --frozen-lockfile
pnpm lint
pnpm format:check
pnpm typecheck
pnpm test                       # Vitest; pnpm test:ci writes reports/junit-ts.xml and coverage
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm tauri build --debug        # macOS or Windows app bundle
uv run --no-project --with pytest pytest scripts/tests -q   # when scripts/tests exists
```

## Pull requests and the merge gate

- Use `.github/pull_request_template.md`. Fill every heading.
- An independent reviewer agent posts a PR comment whose first line is `REVIEW: APPROVED` or `REVIEW: CHANGES REQUESTED`.
- Merge only with all CI checks green and `REVIEW: APPROVED`. Squash merge and delete the branch.

## Never commit

Secrets, `.databrickscfg`, `.env*`, `*.pem`, `*.key`, `build/`, `*.riv`. Tokens come only from `databricks auth token --profile <p>` at run time.
