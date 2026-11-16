# ADR 0001: The foundation skeleton proceeds before spike SP-01

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
| How a MOG window becomes transparent on macOS | `transparency::apply` in `src-tauri/src/windows.rs` | No transparency on macOS (Tauri gates it behind `macos-private-api`); `transparent(true)` on Windows and Linux. |
| `macOSPrivateApi` | `tauri.conf.json` and the Cargo `tauri` features | Off. No `macos-private-api` feature. |
| The Rive runtime package | The MOG view renders through a mascot renderer interface (`src/views/mog/mascot.ts`) | A DOM placeholder. No `@rive-app/*` package is installed. |
| Non-activating panel and hover delivery (UX-08a) | `WindowSpec.focusable` and #11, #80, #81 | `focusable(false)` on MOG windows. |

The window model (labels `brain`, `hub`, `mog-<id>`, `card-<id>`), the CSP, the per-window capabilities and the crate split do not depend on the SP-01 result.

## Consequences

- On macOS the MOG window is not transparent until SP-01 or #11 applies its result. This is a known gap, marked `TODO(#10)` in code.
- #169 (pin the Tauri shell and Rive runtime) stays open. It adds the Rive package and the lockfile checks after SP-01. The skeleton does not contradict #169: Tauri is pinned at `2.12.5` or later, and no `@rive-app/webgl` exists.
- If SP-01 fails all the way to the `macOSPrivateApi` fallback, the change is one Cargo feature, one config key and the body of `transparency::apply`.
