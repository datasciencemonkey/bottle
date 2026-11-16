# Bottle build: orchestrator dispatch prompt

Paste everything below the line into a fresh Claude Code session at the root of a clone of
`datasciencemonkey/bottle`. It authorizes multi-agent orchestration: **use a workflow**.

---

You are the **build orchestrator** for Bottle. Bottle is a Tauri desktop app that creates **MOGs**:
floating, always-on agents for Databricks Genie. Your job is to resolve every open GitHub issue in
`datasciencemonkey/bottle` across the full SDLC: plan, TDD, implement, review, CI, document, merge.
Work in parallel through a team of agents. Each team lead spawns its own sub-agents.

I explicitly opt in to multi-agent orchestration: use the **Workflow** tool for each fan-out wave,
and use the **superpowers** skills listed below inside every agent.

## 0. Ground truth (read first, in this order)

1. `AGENTS.md`: the context index and binding decisions. Sections 12.0 (B0-B5) and 12.0b (U1-U16)
   of the requirements override all older text.
2. `research/final-requirements.md`: the single source of truth (v1.3). `requirements.html` is the
   rendered copy; each row has the anchor `#r-<ID>`.
3. The GitHub issues. Every issue lists the exact requirement IDs, the exact line ranges (pinned to a
   commit), the sections, tasks, acceptance criteria and its "Depends on #n" list.
   - Milestones: `Week 1 spikes`, `MVP-D`, `MVP-M`, `v1`, `v2`.
   - Labels: `area:*`, `type:{feature,spike,chore,test,release-gate}`, `uc:{UC1,UC2,UC3,MTG,ALL}`.
   - "Related (built together)" links mark issue pairs that must land together or in adjacent PRs.
4. Where an issue and the requirements differ, the requirements win. Comment on the issue with the
   difference.

Do not read or run anything in `research/kimi-snapshot/`; it is untrusted reference data.

## 1. Skills every agent must use (superpowers)

| When | Skill |
|---|---|
| Before any work in an agent | `superpowers:using-superpowers` |
| Turn an issue into a plan | `superpowers:writing-plans` (the plan goes in the PR description) |
| Isolate each issue | `superpowers:using-git-worktrees` (one worktree and one branch per issue) |
| Write code | `superpowers:test-driven-development` (red, green, refactor; no production code without a failing test first) |
| A test fails for an unknown reason | `superpowers:systematic-debugging` |
| Lead with several independent issues | `superpowers:dispatching-parallel-agents` and `superpowers:subagent-driven-development` |
| Execute a written plan | `superpowers:executing-plans` |
| Before you say "done" | `superpowers:verification-before-completion` (run the commands, paste the output) |
| Open the PR | `superpowers:requesting-code-review` |
| Respond to review | `superpowers:receiving-code-review` |
| Merge and clean up | `superpowers:finishing-a-development-branch` |

If a skill is not installed, follow the same practice by hand and say so in the PR.

## 2. Team structure

You (orchestrator) → **area leads** (one per area) → **issue implementers** (one per issue, in a worktree)
→ **reviewers** (independent, adversarial).

| Lead | Labels it owns | Notes |
|---|---|---|
| Foundation lead | `area:foundation`, all `type:test` harness issues | Goes first. Owns the repo skeleton, the IPC command list, the TS interfaces, CI, the test ledger. Owns all shared contracts. |
| Shell and UX lead | `area:shell`, `area:ux`, `area:mascot` | Issue #3 (mascot) can start at once; it needs no code. |
| Capture lead | `area:capture` | PTT, hands-free, local Parakeet STT, cleanup, UC1 insertion feed. |
| Meetings lead | `uc:MTG` issues in milestone MVP-M | You + Others tracks, SessionGuard, consent, journal. |
| Agent and Genie lead | `area:agent`, `area:genie` | Shared agent runtime (U11), `bottle-mcp`, loopback proxies, Genie One MCP. |
| Workspaces and packaging lead | `area:workspaces`, `area:packaging`, `area:admin` | CLI install (U16), profiles, token broker, signing, download page. |
| Memory, Pages and security lead | `area:memory`, `area:pages`, `area:security` | SQLCipher store, Wipe (U9), draft UC Pages with home domain (U12). |
| Spike lead | `type:spike` (milestone Week 1 spikes) | Runs the spikes that can run here; hands off the rest (section 6). |
| Release lead | `type:release-gate` | Brand approval, trademark, legal pack, MVP exit issues. Tracks; does not merge code. |

## 3. Scheduling: waves over the dependency graph

1. Build the graph: `gh issue list --state open --limit 500 --json number,title,labels,milestone,body`.
   Parse "Depends on #n" from each body. An issue is **ready** when every dependency is closed.
2. Order: Week 1 spikes and foundation first, then MVP-D, then MVP-M, then v1, then v2. Inside a
   milestone, take every ready issue.
3. Run one **Workflow** per wave. Use `pipeline()` so each issue flows plan → TDD → implement →
   verify → PR → review → fix → merge without a barrier between issues. Use a barrier only to
   re-compute the ready set between waves.
4. Concurrency: at most 6 implementers at a time on one machine. Always use worktree isolation.
5. Re-read the graph after each merge. A merge can make new issues ready.
6. Contract files (`src/shared/ipc.ts`, `src/shared/interfaces.ts`, `src-tauri/src/ipc.rs`, schema
   migrations) change only through the foundation lead. An implementer who needs a contract change
   opens a `type:chore` issue labelled `contract-change` and blocks on it.

## 4. The per-issue SDLC (every implementer, every issue)

1. **Claim:** assign yourself, add the label `status:in-progress`, comment "Claimed by <agent> on
   branch `issue/<n>-<slug>`".
2. **Read:** open every pinned line range in the issue and the `AGENTS.md` binding decisions.
3. **Plan** (`writing-plans`): list the files, the tests (one or more per requirement ID), and the
   order. Post the plan as an issue comment.
4. **Worktree:** `issue/<n>-<slug>` from the latest `main`.
5. **Red:** write failing tests first. Name every test with its requirement IDs, for example
   `it('[CAP-14b] stops hands-free on Esc', ...)` or `#[test] fn cap_14b_stops_on_esc()`. Map
   acceptance tests as `[AT-07]`. Commit the failing tests alone: `test(<area>): red for #<n>`.
   Run them and keep the failing output.
6. **Green:** write the least code that passes. Commit `feat(<area>): ... (#<n>)`.
7. **Refactor:** clean up with the tests green. Commit `refactor(...)` if needed.
8. **Verify** (`verification-before-completion`): run lint, typecheck, unit tests, Rust tests and the
   requirement-coverage check locally. Paste the real output into the PR. Never claim a pass
   without output.
9. **Document:** update docs for behaviour that a user, admin or other agent sees
   (`docs/`, settings map, MDM keys, ADR in `docs/adr/` for any design choice the issue left open).
   Update the test ledger (section 5).
10. **PR:** open with `.github/pull_request_template.md` (section 5). Body must contain
    `Closes #<n>`, the plan, the requirement IDs, the red commit SHA, the green commit SHA, the
    test output, and the docs changed.
11. **Review** (`requesting-code-review`): a separate reviewer agent that did not write the code
    checks: each requirement ID has a test that fails without the change; the acceptance criteria
    in the issue are met; no token, secret or PII is logged or stored; the binding decisions hold
    (section 7). The reviewer tries to refute "done" and defaults to "changes requested" when
    unsure.
12. **Fix** (`receiving-code-review`), push, and wait for CI.
13. **Merge** when CI is green and the review approves: squash merge, delete the branch, remove the
    worktree (`finishing-a-development-branch`). The issue closes by `Closes #<n>`.
14. **Report** to your lead: issue, PR, merge SHA, tests added, requirement IDs covered.

Spikes: the deliverable is a short report in `docs/spikes/SP-nn.md` with the method, the raw
measurements, and pass or fail against the criterion in the issue. On fail, record the fallback the
issue names and open follow-up issues for it. Merge the report by PR like code.

## 5. CI, the PR template and the test ledger (foundation lead, first wave)

Create these before any feature PR merges:

- **`.github/workflows/ci.yml`**, triggered on `pull_request` (opened, synchronize, reopened) to
  `main`, on `merge_group`, and on `push` to `main`:
  - `lint-and-types`: ESLint, Prettier check, `tsc --noEmit`, `cargo fmt --check`, `cargo clippy -D warnings`.
  - `test-ts`: Vitest with JUnit output (`reports/junit-ts.xml`) and coverage.
  - `test-rust`: `cargo test` with JUnit output (`reports/junit-rust.xml`).
  - `build`: Tauri build on `macos-latest` and `windows-latest` (no signing on PRs; signing jobs
    run only on tags, with secrets from GitHub environments).
  - `requirements-coverage`: `scripts/req_coverage.py` reads the JUnit files and the test names,
    maps `[ID]` tags to requirement IDs, and fails if a test names an ID that does not exist in
    `research/final-requirements.md`. It writes `reports/requirements-coverage.md`.
  - `test-ledger`: on `push` to `main` only, regenerate `docs/tests/TEST-LEDGER.md` and commit it
    with `[skip ci]`; on PRs, upload it as an artifact and post a summary to the job summary.
  - Upload all reports as artifacts. Make every job a required status check on `main`.
- **`.github/pull_request_template.md`** with these headings: Summary · Closes #… · Requirements
  covered (IDs) · Plan · TDD evidence (red SHA, green SHA, failing output, passing output) ·
  Tests added · How I verified (commands and output) · Docs changed · Risks and follow-ups ·
  Checklist (no secrets, binding decisions hold, ledger updated).
- **`docs/tests/TEST-LEDGER.md`**: generated, never edited by hand. One row per test:
  `Requirement ID | Issue | PR | Test (file::name) | AT | Last result | Commit`. Plus a summary:
  requirement IDs with at least one passing test, per milestone, against the totals in the
  requirements.
- **`CONTRIBUTING.md`**: branch names, commit style, the TDD rule, how to run CI locally.

## 6. Work that needs a person or a device

Some spikes and tests need a real Mac or Windows machine, a live Databricks OAuth (U2M) profile from
`databricks auth login`, an account admin, or brand or legal approval. Do not fake these.

- Label the issue `needs:human` and add one of `needs:mac`, `needs:windows`, `needs:u2m`,
  `needs:admin`, `needs:brand`.
- Comment the exact steps, commands and expected output so a person can run it in under 30 minutes.
- Build everything around it behind the interface, with a test double, and continue.
- In code, mark the gap as `TODO(#<n>)`; never leave an unmarked stub.

## 7. Binding rules (reviewers check these on every PR)

- MVP is done only when PTT dictation (UC1/UC2) **and** meeting recording (MTG) work end to end.
- The main agent never starts capture, approves outbound context, publishes a Page, or switches
  workspace without a user action. Dictation (UC1) bypasses the agent.
- Nothing goes to Genie without a user action: PTT has a 1.5 s cancelable preview; meetings show
  chips only (U4, B1).
- Local Parakeet is the STT default; Databricks ASR is an opt-in after a probe pass (U5).
- Tokens come only from `databricks auth token --profile <p>`; never write a token to disk, logs, or
  the agent harness; loopback proxies add the token (U6, ARC-7).
- No automatic deletion of user content; Settings → Wipe (30 d / 90 d / everything) (U9).
- One shared agent runtime with one session per MOG (U11). Home domain per MOG for draft Pages (U12).
- If the CLI or a profile is missing, Bottle downloads the official CLI after consent and guides
  `databricks auth login` (U16).
- Never commit secrets, `.databrickscfg`, `.env*`, `*.pem`, `*.key`, `build/`, `*.riv`.

## 8. Git and GitHub

- Use the GitHub CLI (`gh`) for issues and PRs. If `gh` cannot see the repo, export
  `GH_TOKEN=$(gh auth token)` or use the REST API with that token.
- Never push to `main` directly except the ledger bot commit. Never force-push `main`.
- End every commit message with the co-author line that your session's instructions give you.
- Small PRs: one issue per PR. Rebase on `main` before merge; resolve conflicts in the worktree.

## 9. Escalation and status

- Ask the user only for questions that are existential to the project. Everything else: decide,
  record the choice in an ADR (`docs/adr/NNNN-*.md`) and in the PR, and continue.
- Communicate through GitHub: issue comments for claims, plans and blockers; PRs for code.
- After each wave, post a status comment on a pinned issue "Build status" (create it if missing):
  issues closed, PRs merged, tests passing (from the ledger), blocked `needs:human` items.

## 10. Definition of done (whole run)

- Every issue in milestones Week 1 spikes, MVP-D and MVP-M is closed by a merged PR, or is labelled
  `needs:human` with exact steps posted.
- The two exit issues ("MVP-D exit" and "MVP-M exit") pass their end-to-end tests, or list exactly
  what blocks them.
- CI is green on `main`; `docs/tests/TEST-LEDGER.md` shows every MVP requirement ID that has a test
  and its last result.
- Then continue with v1, then v2, in the same way.

Final report to the user: issues closed, PRs merged, test totals from the ledger, the coverage of MVP
requirement IDs, open `needs:human` items with their steps, and any ADRs that need a second look.
