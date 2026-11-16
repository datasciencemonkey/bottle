# ADR 0027: The Rive mascot view, the level meter and the PNG fallback

- Status: Accepted
- Date: 2026-11-16
- Issue: #12 (depends on #9, #11, and the soft spike #10 / SP-01)
- Owner: impl-12 (shell-lead)
- Rows: UX-02a, UX-02b, UX-03, UX-06a, UX-06b, UX-08a, UX-08b, UX-09a, UX-09b, SEC-20a, Table UX-LEVEL, D73

## Context

Issue #12 wires the mascot art of #3 (`mog-rive/scene.rml`) into the MOG view. The binding rules forbid a committed `.riv` file. The `rive` CLI builds the file only after `rive login` (SP-01, section 5). ADR 0012 pins `@rive-app/canvas` 2.50.0. ADR 0021 makes `rive-loader.ts` the only place that builds Rive. The issue left these points open: how the app ships the `.riv`, where the level meter gets its input, and what the view does when the art does not match.

## Decisions

1. **Ship the `.riv` as a built file, not a committed file.** The Vite plugin `bottle-mog-riv` (`vite.config.ts`) reads `mog-rive/build/mog-rive.riv`, the gitignored output of `rive mog-rive --publish`. In dev it serves the file at `/mascot/mog.riv`. At build time it emits `dist/mascot/mog.riv`. If the file is missing, the build warns and continues, and the view shows the PNG fallback. The view loads `/mascot/mog.riv` through `createOfflineRive`, so the asset CDN stays off and the CSP `connect-src 'self'` covers the fetch.
2. **No asset is inlined.** `build.assetsInlineLimit` is 0, because the CSP has no `data:` source.
3. **One `RiveMascot` per MOG view** (`src/views/mog/rive-mascot.ts`). It sets `autoBind: true`, `shouldDisableRiveListeners: true` (UX-08b) and `drawingOptions: DrawOnChanged` (UX-09a). It binds the `Mog` view model: `mood`, `level`, `hover`, `demo`, `attention` and the `poke` trigger. On load it sets `demo=false`, `level=0`, `hover=false`, `mood=idle` (UX-02b). Then it applies the state that the brain sent before the load. It never sets `demo=true`, and in the art only `demo` lets a poke cycle the moods (UX-02a).
4. **A write only on a change.** The renderer writes a view-model value only when it differs from the current value. With `DrawOnChanged`, an unchanged MOG causes no redraw from the view.
5. **The art must have exactly the 7 moods.** The `mood` enum must have 7 entries that match the 7 moods without regard to case. The runtime can report the keys (`idle`) or the display names (`Idle`), so the renderer stores the entry that the runtime reports. Any other enum, a missing property, a load error or a runtime throw gives the PNG fallback.
6. **The PNG fallback** (`PngMascot`, UX-09b) uses transparent PNGs in `mog-rive/fallback/`: `mood-<mood>.png` for the 7 moods, and `attention.png` with only the "?" bubble (UX-03), so the cue shows over any mood without hiding it. The `mog-rive/shots/` files cannot serve: the Rive CLI paints them over an opaque dark background, and the attention shot is a full idle mascot. `tools/render_mog_fallback.py` renders each frame over a black and over a white backdrop in a temporary copy of the project and computes the alpha by difference matting. The cue is the difference between frames with and without the `Attention` node. The CLI does not need `rive login` for this. `tests/mog-fallback-png.test.ts` decodes the PNGs and checks the alpha. Level, hover and poke do nothing in the fallback.
7. **The level meter** (`src/views/mog/level-meter.ts`) is a pure class. `level = clamp((dBFS + 60) / 50, 0, 1)`. A one-pole smoother uses a 30 ms time constant when the level rises and 150 ms when it falls. The meter shows a new value at most 30 times per second and only on a change. A frame with another owner, or with no owner, sets 0 at once (UX-06b). The view no longer uses `mog.view_state.level`.
8. **Mic frames come through `MicLevelSource`.** A frame is `{ owner, dbfs, t }`. When the mic closes or moves, the source must send one frame with the new owner (`null` when closed); the meter sets 0 only on such a frame. The default source sends nothing, so the level stays 0. `TODO(#4)`: the CaptureService level stream (`capture_subscribe` `onLevel`) feeds the source. The brain-to-view event `mog.level` carries the frames to the `mog-<id>` window (#285, ADR 0040).
9. **Moods stay in the brain.** The view renders `mood` and `overlays.attention` from `mog.view_state`. The brain computes them with `mapMood` and `holdMood` (ADR 0018). The view sends DOM hover to the Rive `hover` input and a click to the `poke` trigger. Both are visual only. The brain still decides what the input means (`TODO(#83)`).

## Consequences

- A build without `rive login` ships the PNG fallback. The release build must run `rive mog-rive --publish` first.
- The `mog` bundle now holds the Rive runtime (about 350 kB JS) and `rive.wasm` (2.3 MB).
- The device checks are on #347 (needs:human, needs:mac): rendering in the transparent panel, the first frame, the poke, the idle frame count, the real enum names and the offline run.
- Risk: the art has idle float and blink layers. If they animate all the time, Rive keeps drawing in `DrawOnChanged` mode. #347 measures this. A fix is an art change in `tools/gen_mog_rml.py`, not a view change.
