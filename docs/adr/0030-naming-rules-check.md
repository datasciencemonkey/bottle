# ADR 0030: The naming-rules check (Bottle, the Labs label, the MOG noun)

- Status: Accepted
- Date: 2026-11-16
- Issue: #71
- Owner: shell-lead (implemented by impl-71)

## Context

MB-10a, MB-10b and MB-10c (AT-37) set three naming rules. The product name is "Bottle", and no UI text or package metadata contains a former name of the app. The label "Databricks Labs / experimental" shows in About and in the hub title bar. "MOG" is a UI noun only, and no package or bundle ID contains it. The issue asks for a CI check, so later PRs cannot break these rules. The issue does not say which files are "UI text and package metadata". It also does not say what "MOG only as a UI noun" means for text.

## Decisions

1. **Two checks, one rule set.** The `names` check in `tools/ci_checks.py` runs in the repo-checks workflow today. It scans the files, the page titles, the native window titles and `docs/user/`. The vitest test `src/acceptance/at-37-naming.test.tsx` runs in `pnpm test` and also renders the views. The app CI of #227 will run it. The helpers are in `src/test-support/naming-rules.ts`. The banned pattern is made from parts, so the check files do not hold the names they ban. No workflow change is necessary.
2. **Scope of "UI text and package metadata" (MB-10a).** The scan reads the view HTML pages, `package.json`, the Cargo manifests, `THIRD_PARTY_NOTICES.md` (the About screen shows it), every non-test file in `src/`, `src-tauri/src/`, `src-tauri/capabilities/`, `src-tauri/permissions/` and `crates/*/src/`, `tauri.conf.json` and `docs/user/`. The test also renders the hub, MOG and card views and reads their text and their `aria-label`, `title`, `alt` and `placeholder` attributes. `research/`, `requirements.html`, `AGENTS.md`, ADRs and release notes are agent and project records, not UI text. They can name the former product names as history.
3. **The hub title bar (MB-10b).** The native hub window title and the `hub.html` page title are "Bottle (Databricks Labs / experimental)". The hub page and About also show the label as text.
4. **The MOG noun (MB-10c).** In UI text, the noun is spelt "MOG" or "MOGs" only. The check reports "mog", "Mog" and the other spellings in the page titles, the native window titles, the bundle and npm descriptions, the rendered views and the prose of `docs/user/`. Identifiers and paths (`mog-core`, `mog-<id>`, `mog.html`, `mog:view_state`, `MogView`) and Markdown code are not nouns, so the check skips them. The verbatim third-party licence text in About is out of the noun check, but the MB-10a scan still reads it. The page titles are now "Bottle MOG" and "Bottle MOG card".
5. **IDs (MB-10c).** ADR 0002 keeps the scope of the ID rule: the bundle ID `com.databricks.bottle`, the product name, the npm package and the app crate. The new test also checks `mainBinaryName` if it is set. The `mog-<id>` window labels (ADR 0008) and the internal crates `mog-core` and `mog-core-ffi` are contracts and code names, not package or bundle IDs. They do not change.

## Not in this ADR

Brand and trademark approval of the names, the mascot and the Labs label for an external release is a human item (#129, #241).
