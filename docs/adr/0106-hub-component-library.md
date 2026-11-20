# ADR 0106: The hub adopts shadcn/ui (Radix) and Motion; the MOG and card windows stay plain CSS

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

**Round 2 (PR #602 review).** The first pass rendered the controls wrong in a browser: the
legacy `.hub button` rule still sized a Switch/RadioGroupItem/Checkbox as if they were a
28 px-tall text button, `--input` (a Switch track, a radio/checkbox outline) was the 22% pink
hairline at about 1.6:1 against the surface (WCAG 1.4.11 wants >= 3:1), the sidebar's overlay
z-index let the content column's header paint over its own top-left corner, pinning it covered
the content for good instead of pushing it over, and a real click on the header toggle while
open (pointerdown, then click) closed and immediately reopened it. `sonner` (unused: nothing
calls `toast()`) also injects a `<style>` element at runtime, which the CSP's `default-src
'self'` (no `style-src 'unsafe-inline'`) blocks. Decisions 1, 6 and 8 and the new decision 9
below record the fixes; `#602`'s review comment has the full list.

## Decisions

1. **shadcn/ui (Radix UI primitives) + Motion (the `motion` package) + Tailwind v4, hub only.**
   Copied in as source under `src/views/ui/` (Button, Tooltip, Checkbox, Switch, RadioGroup,
   Collapsible), plus a hand-built `HubSidebar` (shadcn's Sidebar *pattern*, not its copy-in: the
   behaviour below is a specific, non-default spec) and `SettingsGroup`/`SettingsRow` (decision 9:
   also hand-written, not a shadcn copy-in). `cn()` (clsx + tailwind-merge) is the one
   class-merge helper. Icons: `lucide-react`. The shadcn CLI is not a build dependency; every
   file was copied by hand and adapted to this repo's strict TypeScript config
   (`verbatimModuleSyntax`, `exactOptionalPropertyTypes`, `noUnusedLocals`). Exact versions are
   pinned in `package.json` (see the table below); the lockfile is updated with `pnpm`.
   Separator, Tabs, ScrollArea, Dialog and Sonner were copied in during the first pass; the
   review found nothing imported Separator/Tabs/ScrollArea/Dialog and nothing called `sonner`'s
   `toast()`, so all five (and `sonner`'s own dependency and CSP problem, decision 8) were
   removed rather than kept unused (`@radix-ui/react-dialog` itself stays: `WipeDialog.tsx` uses
   it directly, with no `src/views/ui/dialog.tsx` copy-in).
   The legacy `.hub button` rule (`hub.css`) still sized every shadcn control as if it were a
   28 px-tall text button, because it set properties (`min-height`, `padding`) no Tailwind
   utility on the Radix primitives happened to override; it is now `.hub button:not([data-slot])`
   (every `src/views/ui/*` primitive sets `data-slot`), so a Switch, a RadioGroupItem and a
   Checkbox keep their own 16-20px sizing.
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
6. **The hub's nav becomes a hidden-by-default sidebar (course-corrected mid-PR, user request;
   push-vs-overlay and the flush/toggle fixes added in the #602 review round).** The original
   plan (an always-visible, collapsible-to-icons sidebar, remembered open/closed) was
   simplified: the sidebar is hidden until asked for — a toggle button in the header, or
   Cmd+\ / Cmd+B — and Esc or a click outside it hides it again, unless the user pins it open (a
   pin toggle inside the sidebar). Only the pin choice survives a reload
   (`localStorage['bottle.hub.sidebar']`, `'pinned'` or the default `'hidden'`); the icons-only
   collapse is session-only. `src/views/hub/nav.ts` exports one `NAV_ITEMS` array (`{id, label,
   icon}`) and the `HubView` type (`'conversations' | 'mogs' | 'workspaces' | 'recordings' |
   'settings'`); this PR renders only Conversations, MOGs and Settings — `'workspaces'` (the CLI
   path/version, a later PR) and `'recordings'` (rec-lead's area, in flight on another branch
   right now) stay out of the array so the type is ready without an empty nav entry. The hub now
   opens on Conversations (NAV_ITEMS[0]) instead of the old combined "MOGs and settings" default:
   splitting that one area into three areas removes the case the old default was for. A "Setup"
   region (`PermissionNotices`, `CliInstallStep`, `DictationNotices`, `ConnectWorkspace`) renders
   above every area, each already a no-op (or a quiet status line, `CliInstallStep`) when its
   step is not pending.
   - **Push when pinned, overlay otherwise.** The panel is always `position: absolute` within
     `.hub-body`, costing no reflow while it is an unpinned overlay. Pinned, `Hub.tsx` writes
     `--hub-sidebar-width` (the panel's current width, `HubSidebar.sidebarWidth()`) onto
     `.hub-body` as a CSS custom property; `.hub-content` and `.conv` read it in their own
     `padding-left`/`margin-left` (0 while unpinned or hidden), so a pinned panel pushes the
     content column over instead of covering it for good (review finding 3).
   - **Flush with the window's true left edge, full height.** `.hub-body`'s own
     `margin-left: calc(var(--mog-space-6) * -1)` cancels `.hub`'s left padding for that one box
     (`.hub-content`'s matching `padding-left` restores the visual start position), and `.hub`
     is now `display: flex; flex-direction: column; min-height: 100vh` so `.hub-body`'s existing
     `flex: 1` actually stretches to the window's bottom edge outside `.hub--wide` too (review
     polish note).
   - **The header toggle could not close an open sidebar (review finding 2).** A real click is a
     `pointerdown` then a `click`; the panel's outside-click listener saw the `pointerdown` on
     the toggle (outside the panel) and closed the sidebar, and the `click` that followed then
     reopened it (`toggleVisible` flips the now-closed state back open) — net effect, the toggle
     looked like it did nothing. The listener now skips a `pointerdown` whose target is inside
     `[data-sidebar-toggle]`; `Hub.test.tsx` adds a test that dispatches a real `pointerdown`
     then `click`, which `.click()` alone never exercised.
   - **Picking an area closes an unpinned sidebar ("also fix").** `Hub.tsx`'s `pickView` calls
     `requestClose()` when not pinned, the way a native overlay sidebar closes once it knows
     where to go; pinned, it stays (a permanent rail).
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
   Tailwind's own CSS transitions (`transition-colors`, a chevron's `transition-transform`, a
   Button's/Switch's/RadioGroupItem's/Checkbox's own transition) sat in the `utilities` layer,
   which wins over a `hub-legacy` blanket `transition: none` under `prefers-reduced-motion`, so
   every one of those utilities now also carries `motion-reduce:transition-none` (review: "make
   reduced motion win over the Tailwind transitions"); `.hub-content`'s own push-layout
   transition (decision 6) is guarded the plain-CSS way, a `@media (prefers-reduced-motion:
   reduce)` block.
   `sonner` (toasts) was in the first pass (`<Toaster />` in `hub/main.tsx`) but nothing ever
   called `toast()`, and it injects a `<style>` element at runtime — exactly the inline style the
   CSP's `default-src 'self'` (no `style-src 'unsafe-inline'`) blocks, contradicting this
   decision's own claim above. Removed: `<Toaster />`, `src/views/ui/sonner.tsx`, the `sonner`
   npm dependency and its `THIRD_PARTY_NOTICES.md` entry (review finding 6).
9. **Settings reads as a native macOS settings pane, grouped rows in rounded boxes (review of
   #602, "the user cares about them").** `src/views/hub/SettingsGroup.tsx` (hand-written, not a
   shadcn copy-in) has `SettingsGroup` (the rounded, bordered, divided box) and `SettingsRow`
   (one row: a label — and optional helper text — on the left, a control on the right).
   `MicPreference` uses it for its one Switch row; `WipeSettings`'s "Delete data" RadioGroup
   moved from three individually bordered cards into one `SettingsGroup` of lighter divided rows
   (review: "lighter Wipe options"), and each option's helper sentence moved out of the `<label>`
   into its own element, referenced by the radio's `aria-describedby`, so the accessible name
   stays "Older than 30 days" rather than that sentence run together with the helper text
   (`WipeSettings.test.tsx` restores the `value`-order and `radio.labels[0]` assertions a native
   `<input type="radio">` gave for free — Radix's `RadioGroupItem` is a `<button id>`, a
   labelable element, so a `<label htmlFor>` pointed at that `id` still resolves). Every
   collapsible "show more" row in Settings (`WipeSettings`'s "Show how long Bottle keeps data",
   `DataDisclosures`'s "Where your data goes", `CliSettings`'s "Show the Databricks CLI",
   `About`'s "Third-party notices") now shares one visual style — a small chevron plus
   `text-sm text-muted-foreground` — rather than three different weights (two headings, one
   plain-text link); `CliSettings`'s chevron does not rotate, since asking the brain for the CLI
   path is one-way, nothing folds back closed. `DataDisclosures.test.tsx` and `About.test.tsx`
   still find the same `<h2>`/text they always did; only the visual weight changed.
   Contrast (review finding 5): `--input` (a Switch track, a RadioGroupItem/Checkbox outline)
   mapped to `--mog-edge`, the 22% pink hairline, about 1.6:1 against `--mog-surface` — under
   WCAG 1.4.11's >= 3:1 floor for an essential control boundary. A new `--mog-control-border`
   (`tokens.css`, white at 40%, about 3.7:1) replaces it for `--input` only (`--mog-edge` keeps
   doing the decorative-hairline job everywhere else); the Switch thumb became `bg-foreground`
   (`--mog-text`), not `bg-background` (the page's own colour, invisible against an unchecked
   track of the same surface).
   Also fixed: the sr-only toggle label read "⌘\\\\ or ⌘B" (two backslashes) because JSX text
   does not process escapes — `{'⌘\\ or ⌘B'}`, a JS string literal, does, and the toggle now has
   the same `Tooltip` every other sidebar button does; the shortcut handler checks `metaKey`
   only (not `metaKey || ctrlKey`), since Ctrl+B is the Emacs "move back a character" binding
   inside a macOS text field, and it does nothing while a text field, textarea or
   contenteditable region has focus; `MogsArea`/`SettingsArea` stay mounted with `hidden`, like
   `Conversations` already did, rather than unmounting on `view !== ...` (which dropped the Wipe
   scope choice, and closed an open Wipe dialog mid-confirm, on every sidebar nav click).

## Bundle size (`pnpm build`, gzip in parentheses)

| Entry | JS before | JS after (#602 round 2) | CSS before | CSS after (#602 round 2) |
|---|---|---|---|---|
| hub | 68.2 kB (19.0 kB) | 357.2 kB (113.1 kB) | 13.2 kB (3.3 kB) | 42.0 kB (8.4 kB) |
| mog | 358.6 kB (92.5 kB) | 358.6 kB (92.5 kB) | 6.7 kB (1.9 kB) | 6.7 kB (1.9 kB) |
| card | 14.6 kB (5.0 kB) | 14.6 kB (5.0 kB) | 8.3 kB (2.0 kB) | 8.3 kB (2.0 kB) |
| brain | 129.4 kB (46.0 kB) | 129.4 kB (46.0 kB) | — | — |

mog and card move by noise only (a shared-chunk hash), confirming the scoping in decision 5: the
whole cost of Radix + Motion + lucide + Tailwind's generated CSS lands on the hub entry only.
The round-1 PR review measured hub at 388.3 kB JS (121.2 kB gzip) / 44.8 kB CSS (8.9 kB gzip);
removing the unused Separator/Tabs/ScrollArea/Dialog/Sonner copy-ins and the `sonner` dependency
(decisions 1 and 8) brought it back down past the original pre-review number above.

## New third-party dependencies

`@radix-ui/react-{checkbox,collapsible,dialog,radio-group,slot,switch,tooltip}`,
`@tailwindcss/vite`, `class-variance-authority`, `clsx`, `lucide-react`, `motion`,
`tailwind-merge`, `tailwindcss` — all MIT except `class-variance-authority` (Apache-2.0) and
`lucide-react` (ISC, with one Feather-derived MIT sub-licence for the icons it covers); recorded
in `THIRD_PARTY_NOTICES.md` with pinned versions (review finding 7).
`@radix-ui/react-{scroll-area,separator,tabs}` and `sonner` were added, then removed unused in
the #602 review round (decisions 1 and 8).

## Consequences

- A copied-in `src/views/ui/*` file needs hand-adaptation for `exactOptionalPropertyTypes` and
  `verbatimModuleSyntax` on every future shadcn copy-in; there is no CLI to redo this for us.
- Radix's Checkbox/RadioGroup/Switch render as an accessible button
  (`role="checkbox"`/`"radio"`/`"switch"`), not a native `<input>` — but the `<label for>`
  association is real, not lost: each one is a `<button id>`, a labelable HTML element exactly
  like `<input>`, `<select>` or `<textarea>`, so a `<label htmlFor>` pointed at that `id` still
  resolves (`radio.labels[0]` works; confirmed in `WipeSettings.test.tsx`, which restores the
  assertion the round-1 pass had dropped). What is genuinely different is the hidden "bubble
  input" Radix keeps only for native `<form>` submission compatibility: it has no `id`
  and a label pointed at *that* would not resolve — but nothing here points a label at it.
  Tests that asserted on `input[type="radio"|"checkbox"]` now assert on `role`/`aria-checked`
  instead (documented in each test file and the PR body); every other test's DOM/role contract
  is unchanged.
- `ui-1b` (a follow-up) does the licence/version-pin pass to swap the hand-built Motion effects
  for the real Animate UI / Magic UI copy-ins, if still wanted after seeing this PR, and the
  rest of the "Polish" notes from the #602 review that are not in its blocking or "also fix"
  lists (exact pixel alignment, the Wipe radio rows' final visual weight, the content column's
  measure).
