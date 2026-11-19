# ADR 0104: The hub adopts shadcn/ui (Radix) and Motion; the MOG and card windows stay plain CSS

- Status: accepted
- Issue: refs #97, #103
- Date: 2026-11-19
- Owner: ui-1

## Context

The hub's Settings pane used a raw `<fieldset>`/`<legend>` for "Delete data" (Wipe), bare
`<input>`/`<label>` pairs with no gap, a disabled primary button that looked broken rather than
"pick an option first," and two long outline buttons ("Show how long Bottle keeps data," "Show
the Databricks CLI") standing in for what is really inline disclosure content. The top nav was
two always-visible buttons ("Conversations," "MOGs and settings"). A pre-PR UI audit
(`/tmp/ui-audit.md`, screenshots in `/tmp/ui-audit/*before*.png`) confirmed these and added: no
`.hub-section` on Wipe, CLI settings or Connect a workspace (the exact cause of the "Local speech
has big gaps, Wipe/CLI are cramped" complaint); `h1`/`h2`/the legal notice `h2` used literal
`20px`/`15px`/`17px` instead of a token; the Conversations loading state was a blank `aria-busy`
div.

Hand-rolling every control (a styled radio row, a focus-trapped dialog, a collapsible disclosure,
a tooltip) in plain CSS/JS is exactly the kind of work shadcn/ui plus Radix exists to remove, and
the user asked for a "fluid, animated UI" the current plain-CSS-only approach cannot give without
a lot of bespoke keyframe work. Building one window's worth of this (the hub) first, instead of
all three, matches "hub first" (another agent owns `src/views/card/**` on another branch in this
same cycle; the MOG window is deliberately kept light, #561).

## Decisions

1. **shadcn/ui (Radix UI primitives) + Motion (the `motion` package) + Tailwind v4, hub only.**
   Copied in as source under `src/views/ui/` (Button, Separator, Tooltip, Checkbox, Switch, Tabs,
   RadioGroup, ScrollArea, Collapsible, Dialog, Sonner), plus a hand-built `HubSidebar` (shadcn's
   Sidebar *pattern*, not its copy-in: the behaviour below is a specific, non-default spec).
   `cn()` (clsx + tailwind-merge) is the one class-merge helper. Icons: `lucide-react`. The
   shadcn CLI is not a build dependency; every file was copied by hand and adapted to this repo's
   strict TypeScript config (`verbatimModuleSyntax`, `exactOptionalPropertyTypes`,
   `noUnusedLocals`). Exact versions are pinned in `package.json` (see the table below); the
   lockfile is updated with `pnpm`.
2. **Feedback effects are hand-built on Motion, not copied from Animate UI or Magic UI (ui-lead
   ruling, mid-PR).** The plan going in was to adopt Animate UI (animate-ui.com) and Magic UI
   (magicui.design) components for the animated extras. Mid-implementation, ui-lead scoped that
   down: ship ui-1 on shadcn/ui + Radix + Motion only, with the handful of feedback effects
   hand-built on Motion and *named* after the library primitive they stand in for, so a follow-up
   PR (ui-1b) can swap in a licensed, pinned copy-in without renaming call sites. `src/views/ui/
   motion-effects.tsx` has: `BlurFade` (section/card/detail entry), `Shimmer` /
   `ConversationsSkeleton` (a loading state, replacing the blank `aria-busy` div — only renders
   while active, so an idle view has no running animation), `NumberTicker` (counts up once on
   mount, no idle loop), and the sidebar's active-item highlight via a shared Motion `layoutId`.
   The Conversations list rows use `layout="position"` (stands in for an "AnimatedList"). None of
   this is a dependency; it is ~90 lines in one file, documented as "hand-built on Motion;
   Animate UI/Magic UI copy-in deferred to ui-1b" here and in the PR body.
3. **One source of truth stays `src/views/tokens.css`.** It gained: a spacing scale
   (`--mog-space-1..7`: 4/8/12/16/24/32/48px), two new type steps (`--mog-text-lg: 15px`,
   `-xl: 20px`) plus line-height and font-weight tokens, a neutral "note" pair
   (`--mog-note-bg`/`-text`, for a missing-feature state that is not an error — contrast 7.0:1 on
   its own background, 9.5:1 on the hub surface) and a real destructive pair distinct from the
   lamp-red brand accent (`--mog-destructive` white-on-fill is 6.5:1, the hover shade 8.9:1; a
   text-only `--mog-destructive-text` on the hub surface is 5.6:1 — all WCAG AA, >= 4.5:1; ratios
   computed and left as comments next to each token, following the file's existing style).
   `hub.css`'s own `h1`/`h2`/legal-notice-`h2` literals became `var(--mog-text-xl/-lg)`, and the
   main structural rhythm (`.hub` padding, `.hub-section`, `.hub-title`, `.hub-permissions`,
   `.hub-live`) moved onto the spacing scale. This pass is not a byte-for-byte sweep of every
   decorative 2px/4px/6px value in `hub.css` (e.g. icon sizes, hairline offsets) — it targets the
   rhythm declarations the audit named, per "replace ad-hoc px ... where trivially safe."
4. **shadcn's CSS variables map onto the `--mog-*` tokens, in a hub-only file, not in
   `tokens.css` itself.** `src/views/hub/tailwind.css` defines `--background`, `--foreground`,
   `--primary`, `--destructive`, `--sidebar-*`, etc. as `var(--mog-surface)`,
   `var(--mog-text)`, `var(--mog-accent)`, `var(--mog-destructive)`, ... — there is no second
   colour system — and exposes them to Tailwind's utilities via `@theme inline`. `@theme` is
   Tailwind-only syntax; keeping it out of `tokens.css` means the MOG and card windows, which
   import `tokens.css` directly and never load Tailwind, never see a block their plain-CSS
   pipeline would not understand. `hub.css` itself now opens with
   `@layer theme, base, hub-legacy, components, utilities;` and wraps its own rules in
   `@layer hub-legacy { … }`. Cascade layers give priority by layer first, selector specificity
   second: unlayered CSS always beats layered CSS regardless of specificity, so without this,
   `hub.css`'s existing generic selectors (`.hub button`, `.hub input[type=...]`) would keep
   winning over a new shadcn component's Tailwind utility classes. Declaring the five-layer order
   inside `hub.css`, before Tailwind loads (`main.tsx` imports `tokens.css`, then `hub.css`, then
   `tailwind.css`, in that order), keeps `hub.css` winning over Tailwind's own preflight (`base`)
   exactly as before, while a new component's utility classes (`components`/`utilities`) now win
   over any `hub.css` generic selector that happens to also match it. `hub.css` works unchanged in
   a hypothetical build that never loads Tailwind (an empty, unused layer name is valid, inert
   CSS), so this is not a one-way migration.
5. **Scope: hub first, then card (a later PR); the MOG window stays plain CSS.** `mog.html` and
   `card.html` never import `tailwind.css` (only `hub.html` does, through `hub/main.tsx`), so
   neither window ever runs Tailwind's preflight or pays for Radix/Motion/lucide in its bundle.
   `src/views/card/**` was explicitly out of scope: another agent is editing it on another branch
   in this cycle. `src/views/mog/**` is untouched, by design (#561): the mascot window stays on
   hand-written CSS.
6. **The hub's nav becomes a hidden-by-default, overlay sidebar (course-corrected mid-PR, user
   request).** The original plan (an always-visible, collapsible-to-icons sidebar, remembered
   open/closed) was simplified: the sidebar is hidden until asked for — a toggle button in the
   header, or Cmd+\ / Cmd+B — and Esc or a click outside it hides it again, unless the user pins
   it open (a pin toggle inside the sidebar). Only the pin choice survives a reload
   (`localStorage['bottle.hub.sidebar']`, `'pinned'` or the default `'hidden'`); the icons-only
   collapse is session-only. It is an overlay (`position: absolute`), not a push layout, so the
   content column's width never changes under it. `src/views/hub/nav.ts` exports one
   `NAV_ITEMS` array (`{id, label, icon}`) and the `HubView` type (`'conversations' | 'mogs' |
   'workspaces' | 'recordings' | 'settings'`); this PR renders only Conversations, MOGs and
   Settings — `'workspaces'` (the CLI path/version, a later PR) and `'recordings'` (rec-lead's
   area, in flight on another branch right now) stay out of the array so the type is ready
   without an empty nav entry. The hub now opens on Conversations (NAV_ITEMS[0]) instead of the
   old combined "MOGs and settings" default: splitting that one area into three areas removes the
   case the old default was for. A "Setup" region (`PermissionNotices`, `CliInstallStep`,
   `DictationNotices`, `ConnectWorkspace`) renders above every area, each already a no-op (or a
   quiet status line, `CliInstallStep`) when its step is not pending.
7. **The Conversations composer slot is layout only.** `<Conversations composer?={(thread: {
   requestId, profile }) => ReactNode}>` fills `<div data-slot="thread-composer">` at the bottom
   of the detail pane; with no `composer` prop it shows one muted hint line, never a dead input.
   impl-followup owns the real composer; this PR owns only the slot and its fallback.
8. **Reduced motion and CSP/offline.** `MotionConfig reducedMotion="user"` wraps the hub's React
   tree (`hub/main.tsx`); `useReducedMotion()` additionally turns the sidebar's width/slide
   animation into an instant show/hide. Motion sets animated values through the CSSOM (element
   `style` properties via JS), not an injected `<style>` element or `style=` attribute text, so it
   needs no `style-src 'unsafe-inline'` — the CSP in `src-tauri/tauri.conf.json`
   (`script-src 'self' 'wasm-unsafe-eval'`) is unchanged and still holds. Every new dependency is
   an npm package bundled by Vite; nothing is loaded from a CDN (no Google Fonts, no CDN icon
   sprite): `lucide-react` icons are inline SVG, Tailwind's generated CSS is a local file.

## Bundle size (`pnpm build`, gzip in parentheses)

| Entry | JS before | JS after | CSS before | CSS after |
|---|---|---|---|---|
| hub | 68.2 kB (19.0 kB) | 371.4 kB (118.4 kB) | 13.2 kB (3.3 kB) | 44.5 kB (8.9 kB) |
| mog | 358.6 kB (92.5 kB) | 358.6 kB (92.5 kB) | 6.7 kB (1.9 kB) | 6.7 kB (1.9 kB) |
| card | 14.6 kB (5.0 kB) | 14.6 kB (5.0 kB) | 8.3 kB (2.0 kB) | 8.3 kB (2.0 kB) |
| brain | 129.4 kB (46.0 kB) | 129.4 kB (46.0 kB) | — | — |

mog and card move by noise only (a shared-chunk hash), confirming the scoping in decision 5: the
whole cost of Radix + Motion + lucide + Tailwind's generated CSS lands on the hub entry only.

## New third-party dependencies

`@radix-ui/react-{checkbox,collapsible,dialog,radio-group,scroll-area,separator,slot,switch,tabs,
tooltip}`, `@tailwindcss/vite`, `class-variance-authority`, `clsx`, `lucide-react`, `motion`,
`sonner`, `tailwind-merge`, `tailwindcss` — all MIT except `class-variance-authority` (Apache-2.0)
and `lucide-react` (ISC); recorded in `THIRD_PARTY_NOTICES.md`.

## Consequences

- A copied-in `src/views/ui/*` file needs hand-adaptation for `exactOptionalPropertyTypes` and
  `verbatimModuleSyntax` on every future shadcn copy-in; there is no CLI to redo this for us.
- Radix's Checkbox/RadioGroup render an accessible button (`role="checkbox"`/`"radio"`), not a
  native `<input>` with a `<label for>` association (its hidden "bubble input," kept only for
  native `<form>` submission, has no `id` and is not what a label targets). Two WipeSettings
  tests that asserted on `input[type="radio"|"checkbox"]` now assert on `role`/`aria-checked`
  instead (documented in WipeSettings.test.tsx and the PR body); every other test's DOM/role
  contract is unchanged.
- `ui-1b` (a follow-up) does the licence/version-pin pass to swap the hand-built Motion effects
  for the real Animate UI / Magic UI copy-ins, if still wanted after seeing this PR.
