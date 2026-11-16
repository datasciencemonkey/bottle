# ADR 0030: The naming-rules check (Bottle, the Labs label, the MOG noun)

- Status: Accepted
- Date: 2026-11-16
- Issue: #71
- Owner: shell-lead (implemented by impl-71)

## Context

MB-10a, MB-10b and MB-10c (AT-37) set three naming rules. The product name is "Bottle", and no UI text or package metadata contains a former name of the app. The label "Databricks Labs / experimental" shows in About and in the hub title bar. "MOG" is a UI noun only, and no package or bundle ID contains it. The issue asks for a CI check, so later PRs cannot break these rules. The issue does not say which files are "UI text and package metadata". It also does not say what "MOG only as a UI noun" means for text.

## Decisions

1. **Two checks, one rule set.** The `names` check in `tools/ci_checks.py` runs in the repo-checks workflow today. It checks every rule in decisions 2 to 5 on the source files. The vitest test `src/acceptance/at-37-naming.test.tsx` runs in `pnpm test`. It checks the same rules, and it also renders the hub, MOG and card views. The app CI of #227 will run it. The TS helpers are in `src/test-support/naming-rules.ts`. The banned pattern is made from parts, so the check files do not hold the names they ban. No workflow change is necessary.
2. **Scope of "UI text and package metadata" (MB-10a).** Both checks read the view HTML pages, `package.json`, the Cargo manifests and `THIRD_PARTY_NOTICES.md` (the About screen shows it). They also read every non-test file in `src/`, `src-tauri/src/`, `src-tauri/capabilities/`, `src-tauri/permissions/` and `crates/*/src/`, plus `tauri.conf.json` and `docs/user/`. A space, `_`, `-` or an HTML space entity (`&nbsp;`, `&#160;`) between the two parts of a former name does not hide it. `research/`, `requirements.html`, `AGENTS.md`, ADRs and release notes are agent and project records, not UI text. They can name the former product names as history.
3. **The Labs label (MB-10b).** The native hub window title and the `hub.html` page title are "Bottle (Databricks Labs / experimental)". `Hub.tsx` and `About.tsx` show the label as text. Both checks need all four.
4. **The MOG noun (MB-10c).** In UI text, the noun is spelt "MOG" or "MOGs" only. Both checks report "mog", "Mog" and the other spellings in these places: the page titles, the native window titles, the bundle and npm descriptions, and the prose of `docs/user/`. They also check the JSX text and the `aria-label`, `title`, `alt` and `placeholder` attributes of `src/views/**/*.tsx`. The vitest test also reads each text node and each of these attributes of the rendered views, one at a time. Identifiers and paths (`mog-core`, `mog-<id>`, `mog.html`, `mog:view_state`, `MogView`) and Markdown code are not nouns, so the checks skip them. The noun check does not read the verbatim third-party licence text in About, but the MB-10a scan does. The page titles are now "Bottle MOG" and "Bottle MOG card".
5. **IDs (MB-10c).** ADR 0002 sets the scope of the ID rule: the bundle ID, the product name, the npm package and the app crate. Both checks need the bundle ID `com.databricks.bottle` (B0). They fail if one of these IDs, or `mainBinaryName` when it is set, contains "mog" in any case. The `mog-<id>` window labels (ADR 0008) and the internal crates `mog-core` and `mog-core-ffi` are contracts and code names, not package or bundle IDs. They do not change.

## Not in this ADR

Brand and trademark approval of the names, the mascot and the Labs label for an external release is a human item (#129, #241).
