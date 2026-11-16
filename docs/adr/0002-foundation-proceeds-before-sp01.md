# ADR 0002: The foundation skeleton proceeds before spike SP-01

- Status: Accepted
- Date: 2026-11-16
- Issue: #221 (depends on #10, SP-01)
- Owner: foundation-lead

## Context

Issue #221 (repo and Tauri v2 skeleton) lists #10 (SP-01) as a dependency. SP-01 tests a Rive mascot in a transparent, frameless, non-activating Tauri window that a signed and notarized DMG installs. SP-01 picks the Rive runtime (`@rive-app/webgl2` or `@rive-app/canvas`, NFR-16a). It also tells us if the app needs `macOSPrivateApi` (NFR-14b, D73).

The signed part of SP-01 needs an Apple Developer ID. The build Mac does not have one, so SP-01 cannot finish soon. All other wave-1 work depends on #221.

## Decision

We treat #10 as a soft dependency of #221. The skeleton lands now. Each skeleton choice that SP-01 can change stays behind one seam:

| SP-01 can change | Seam in the skeleton | Default until SP-01 ends |
|---|---|---|
| How a MOG window becomes transparent, and any white-flash or halo fix | `transparency::apply` in `src-tauri/src/windows.rs` | `transparent(true)` on every desktop OS. Tauri 2.12 clears the window and WebView background without `macOSPrivateApi`. SP-01 measures if this passes in a signed DMG. |
| `macOSPrivateApi` | `tauri.conf.json` and the Cargo `tauri` features | Off. No `macos-private-api` feature. |
| The Rive runtime package | The MOG view renders through a mascot renderer interface (`src/views/mog/mascot.ts`) | A DOM placeholder. No `@rive-app/*` package is installed. |
| Non-activating panel and hover delivery (UX-08a) | `WindowSpec.focusable` and #11, #80, #81 | `focusable(false)` on MOG windows. |

The window model (labels `brain`, `hub`, `mog-<id>`, `card-<id>`), the CSP, the per-window capabilities and the crate split do not depend on the SP-01 result.

## Consequences

- The MOG window is transparent in the skeleton, but nobody has measured it in a signed DMG (alpha, white flash, halo). SP-01 does that. The gap is marked `TODO(#10)` in code.
- #169 (pin the Tauri shell and Rive runtime) stays open. It adds the Rive package and the lockfile checks after SP-01. The skeleton does not contradict #169: Tauri is pinned at `2.12.5` or later, and no `@rive-app/webgl` exists.
- If SP-01 fails all the way to the `macOSPrivateApi` fallback, the change is one Cargo feature, one config key and the body of `transparency::apply`.

## Also decided in the skeleton (#221)

- **Bundle ID.** The bundle ID is `com.databricks.bottle`. It equals the macOS MDM domain of B0 and Appendix C, so managed preferences and the app use one identifier.
- **MB-10c scope.** MB-10c applies to the bundle ID, the product name, the npm package (`bottle`) and the app crate (`bottle`). The internal Rust crates `mog-core` and `mog-core-ffi` keep the names that ARC-1 gives them. They are `publish = false` and never ship as a package or bundle ID, so they are out of MB-10c scope. The test `mb_10c_no_mog_in_bundle_or_package_ids` checks both rules.
- **Windows 11.** This Mac cannot build or start the Windows app. The Windows part of the #221 acceptance criteria moves to a `needs:windows` follow-up issue with the steps.
