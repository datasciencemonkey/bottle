# ADR 0012: Tauri and Rive package pins, and the checks that enforce them

- Status: Accepted
- Date: 2026-11-16
- Issue: #169 (depends on #10, SP-01, soft)
- Owner: impl-169 (packaging-lead)
- Rows: NFR-14a, NFR-14b, NFR-16a, NFR-16b, U1, D32, D73, AT-22, AT-37

## Context

NFR-14a says: "The shell must be Tauri 2.12.5 or later. The lockfile shows the version." Tauri has two package sets with separate version lines:

| Package | Registry | Version on main | Latest on the registry (2026-11-16) |
|---|---|---|---|
| `tauri` (Rust crate, the shell) | crates.io | 2.12.5 | 2.12.5 |
| `@tauri-apps/api` (JS bindings) | npm | 2.12.4 | 2.12.4 |
| `@tauri-apps/cli` (build tool) | npm | 2.12.4 | 2.12.4 |

We checked the npm registry through the approved proxy. No `@tauri-apps/api` or `@tauri-apps/cli` 2.12.5 exists. So the literal text "Tauri >= 2.12.5 in the npm lockfile" cannot be met today.

The other Tauri crates in `Cargo.lock` have their own version lines: `tauri-build` 2.7.4, `tauri-codegen` 2.7.4, `tauri-macros` 2.7.4, `tauri-runtime` 2.12.2, `tauri-runtime-wry` 2.12.3, `tauri-utils` 2.10.2. The `tauri` crate pulls them in at compatible versions.

SP-01 (`docs/spikes/SP-01.md`, partial result, #263) chose **`@rive-app/canvas` provisionally** (section 7). Canvas used less CPU and memory than webgl2 and had no render stall. Both runtimes gave the same alpha. The signed rerun is still open (needs:human). SP-01 also found that `macOSPrivateApi` is a no-op on Tauri 2.12.1 and later (section 6.5). SP-01 records no `macOSPrivateApi` fallback.

## Decision

1. **NFR-14a applies to the shell, which is the Rust `tauri` crate.** `Cargo.lock` must have exactly one `tauri` crate, at 2.12.5 or later.
2. **The npm Tauri packages follow the crate line.** `@tauri-apps/api` and `@tauri-apps/cli` must be exact `x.y.z` pins. They must have the same major.minor as the `tauri` crate (now 2.12). They must be 2.12.4 or later. `pnpm-lock.yaml` must resolve the same version. Tauri's own version check compares only major.minor between the crate and `@tauri-apps/api`, so this rule keeps the two sides compatible. When npm publishes 2.12.5 or later, a normal update moves the pins up. The floor `MIN_TAURI_NPM` in `tests/packaging-pins.ts` then goes up too.
3. **The other Tauri crates are not pinned separately.** The `tauri` crate sets their versions. This ADR records their versions on the merge date (table above).
4. **Rive runtime: `@rive-app/canvas` 2.50.0**, exact pin, in `dependencies`. This is the SP-01 provisional choice. It is marked `TODO(#10)`. If the signed rerun picks `@rive-app/webgl2`, swap the package. The checks allow exactly one of the two runtimes. No view imports the runtime in this change. The mascot work (#3) wires it in through the mascot renderer interface (ADR 0002).
5. **No `@rive-app/webgl`** in any `package.json`, `pnpm-lock.yaml`, `package-lock.json` or `yarn.lock` in the repo (the spike folders too), and in no app source file. `@rive-app/webgl2` does not match the ban. No other `@rive-app/*` package can be a direct dependency of the app.
6. **No `macOSPrivateApi`.** No `src-tauri/tauri*.conf.json` file may have a `macOSPrivateApi` key at any depth. No app `Cargo.toml` may enable the `macos-private-api` feature. The one exception: `docs/spikes/SP-01.md` has this exact line on its own:
   `**SP-01 fallback recorded: macOSPrivateApi**`
   SP-01 writes that line only if it reaches fallback step 3. That needs an owner decision, because the flag is a no-op on this Tauri version (SP-01 section 6.5).

## How the checks run

The checks are vitest tests in `tests/packaging-pins.test.ts`. The pure check functions are in `tests/packaging-pins.ts`. `pnpm test` runs them, so the CI job that runs vitest (#266) runs them too. No workflow file changes. Each check also runs on bad fixtures, so a bad lockfile or config provably fails it. The test names carry the requirement IDs for the coverage report (NFR-23a).

## Consequences

- AT-22 and AT-37 inspection items for NFR-14a, NFR-14b, NFR-16a and NFR-16b are automatic.
- The Rive runtime is installed but not used yet. It adds about 6 MB to `node_modules` (JS, source map and two wasm files) and has no dependencies of its own. The bundle size changes only when a view imports it.
- The NFR-14b intent (no private API) is not met by any transparent Tauri 2.12.5 window (SP-01 section 6.5). This check enforces the literal row only. The owner decision stays with #10.
