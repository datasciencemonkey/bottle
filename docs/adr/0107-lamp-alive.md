# ADR 0107: The lamp MOG is alive: an orbiting spark, eyes that follow the pointer, a name row, a close card

- Status: accepted
- Issue: refs #286
- Date: 2026-11-19
- Owner: lamp-alive

## Context

The user asked for four things on the MOG:

1. The gold four-point spark over the head should move slowly in a circle.
2. The eyes should follow the mouse when it moves around the MOG.
3. The name of the MOG should sit right under the picture, not on the lamp.
4. The card (the "Asking Genie" pill, and the answer) should sit close to the MOG, on either side.

The user thought items 1 and 2 were Rive changes. The build does not ship Rive art: the `.riv` is
not committed (ADR 0027, `*.riv` is never committed), so the MOG shows the PNG layer renderer
`LampMascot` (`src/views/mog/lamp-mascot.ts`, #286). The features are built there. The Rive path
(`RiveMascot`) forwards the new calls to that fallback, and has no look input, because the `.riv`
the repo can build has none (see `mog-rive/AGENTS.md`).

## Decisions

### The spark is a separate element

The spark was baked into every `genie-<mood>.png` layer (the `Sparkle` node of `scene.rml`). The
view now draws its own spark (inline SVG, four nested boxes) and `tools/clean_lamp_layers.py`
removes the baked one from the layers. The script is reproducible and idempotent, and
`render_mog_fallback.py` runs it after each render. It writes `layers/SPARK-CLEAN.sha256` (the hash
of the script and of each cleaned layer), and `tests/mog-fallback-png.test.ts` checks it. The
mood PNGs (`mood-*.png`, the static fallback of `PngMascot`) keep their baked spark.

The spark orbits the top of the head of each pose in 10 s, with a 5.3 s breathing radius, a 2.9 s
twinkle (scale and opacity) and a counter-turn that keeps it upright. The three periods share no
factor, so the pattern does not repeat exactly. Each loop is a CSS animation on `transform` or
`opacity` only, so it runs on the compositor.

**This replaces the old rule "an idle MOG runs no animation".** The spark is the one thing that is
always alive. It pauses (`animation-play-state`) while the MOG is hidden (the Hide menu item, or
an invisible page) and while it sleeps. With `prefers-reduced-motion` it is still, at the place of
the old baked spark. Measured cost (headless Chromium, software compositing, so an upper bound for
a Mac with a GPU), in `docs/shots/lamp-alive/cpu.json`: the orbit adds about 2 to 3.5 % of one core
over the still spark (two runs: +2.2 % and +3.4 %). The main thread does no work for it (script and layout time, and layout
count, do not change). Hidden, the cost is the same as with reduced motion.

### Eyes follow the pointer

A MOG window is a non-activating panel and gets no mouse move while the pointer is outside it, so
the host reads the global pointer:

- `src-tauri/src/mog_look_host.rs` reads the pointer with the Tauri cursor read (`NSEvent`
  `mouseLocation` on macOS, `GetCursorPos` on Windows). It needs no new permission, no new crate
  and no `unsafe`.
- `crates/mog-core/src/mog_look.rs` is the pure part: a look vector from the eye centre, clamped
  to the unit disc, with a dead zone of 14 px; a poll period of about 30 Hz while the pointer is
  within 400 px of a visible MOG, and about 4 Hz when it is further or no MOG is visible; and a
  send-on-change rule (a still pointer sends nothing; a far pointer sends one reset to `0, 0`).
- Only the vector leaves Rust. A pointer position is never logged, stored or sent.
- ARC-5 bans Tauri events in the app crate, so the host sends `mog.look` `{ mogId, dx, dy }` to
  the brain on the brain Channel (`events::MOG_LOOK`, `EVENTS['mog.look']`, core to brain). The
  brain (`src/brain/mog-look-relay.ts`) sends `{ dx, dy }` to that one view as `mog:look`
  (`emitTo`). The view only listens (ARC-4); its capability does not change.

In the view, `LampMascot.setLook` moves a pupil over each baked eye of the open poses (idle,
listening, processing, answering), from the `EYES` geometry the lids use: a patch in the screen
colour covers the baked eye and a pupil in the eye colour moves over it by a few px, with a short
`translate` transition (170 ms), so it lags a little and glides back to the centre. Sleeping,
error and success keep their eyes. A blink still shows on top (the lids, and the pupil scales
away with the same keyframes). Hidden, sleeping and reduced motion turn the tracking off.

### The name row

The MOG window is 160 x 192 (`MOG_WIDTH`, `MOG_HEIGHT`): the 160 px art square on top, and a 32 px
row under it for the target marker and the name pill. The pill no longer covers the lamp.

### The card sits close to the MOG

The card window keeps a 10 px gap between the lamp art and the visible edge of the pill on both
sides. It is as wide as its content (a compact pill) or 340 px (the answer card), so no clear
window lies between the MOG and the card and takes the user's clicks. The edge that faces the MOG
never moves. On the left the tail is on the right edge and the content drifts toward the MOG. The
placement is in `windows::card_beside_sized` (Rust, at show time) and `card-placement.ts` (the
view, when it fits the window to its content). To read its MOG window the card view gets two
more window permissions (`core:window:allow-outer-size`, `core:window:allow-get-all-windows`);
`ipc_capabilities.rs` checks them.

## Consequences

- The contract files change by one event (`mog.look`).
- `render_mog_fallback.py` needs `numpy` for the clean step.
- A `.riv` author can add the matching inputs later. See `mog-rive/AGENTS.md`.
