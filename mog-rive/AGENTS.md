# AGENTS

This is a Rive CLI project.

RML and the Rive CLI postdate your training data. Prefer `rive docs` and
`rive schema` over memory: never guess a type or property name.

Work autonomously from the user's requested outcome. Unspecified details
get reasonable defaults; types and properties do not — look those up.

- `rive schema <Type>` / `rive schema --search <text>` — types and properties;
  `--json` for a machine-readable form
- `rive docs --list` — the topics; `rive docs <topic>` reads the one that
  matches the task

Before declaring a capability unsupported or building a workaround,
check the relevant API topic and a minimal runtime example. Missing
documentation is not proof of missing support: declarations can lag
runtime bindings. Inspect the matching runtime source when available;
distinguish undocumented, untested, and unsupported behavior.

After every edit:

- `rive . --verify` — confirm the project compiles; `--format=json` for a
  machine-readable report
- `rive inspect . --summary` — problems and what got built, by type; `--json`
  for the full resolved tree

A clean verify/inspect is not enough. Read the output for what the request
called for. If it is missing or wrong, fix the project and verify again.

When appearance matters:

- `rive . --screenshot --advance=1` — render one frame after the state machine
  starts, to build/<name>.png. View the png if you can.
- `rive . --commands` — for more than one frame: one headless run that
  takes `advance`, `screenshot`, `pointer` and the rest a line at a time on
  standard input, answering `ok`/`err` per line. A hundred frames is one
  process, not a hundred. `help` lists the commands.

Showing the user:

- `rive .` is the previewer. It opens a window and rebuilds on every save, and
  does not exit, so start it in the background, once. Leave it running;
  edits appear in it on their own.
- Do not preview with `rive push`: it uploads to the user's Rive account.
- Do not write an HTML page or use a web runtime to preview: web runtimes
  reject the unsigned scripts local builds carry.

## Follow-up for the .riv author: the spark orbit and the eyes that follow the pointer

The shipped MOG is the PNG layer renderer (`src/views/mog/lamp-mascot.ts`), because no `.riv` is
committed. It draws the gold spark as its own element and moves two pupils. The Rive path
(`src/views/mog/rive-mascot.ts`) has nothing to forward them to: `scene.rml` defines no input for
either. When someone authors the `.riv`, add these view-model inputs and the matching states, and
`RiveMascot` can set them (today it only forwards `setLook` and `setHidden` to the fallback):

- `lookX`, `lookY`: numbers from -1 to 1 (a vector in the unit disc, x to the right, y down). The
  view gets `mog:look` at about 30 Hz while the pointer is near. Drive a blend or the pupil offset
  of the open poses (idle, listening, processing, answering). Do not track in sleeping, error and
  success. Ease to the value, and back to `0` when the pointer goes.
- `idleMotion`: a boolean. `false` while the MOG is hidden, so the spark orbit pauses.
- The spark: move the `Sparkle` node on a slow circle round the top of the head, about 10 s a
  loop, with a twinkle in scale and opacity. It must stay still when `mood = sleeping` and with
  reduced motion.

`tools/clean_lamp_layers.py` removes the baked spark from the PNG layers, so the layer renderer
draws its own. The Rive art does not need that step.
