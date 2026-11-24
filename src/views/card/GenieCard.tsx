/**
 * #573: the Ask Genie surface beside the MOG. One element goes through three states:
 *
 * 1. Working: the status pill. The stage line ("Asking Genie", then the fixed Genie stage) with a
 *    soft light sweep, and after 3 s the timer ("0:03", then "Working… 1:04"). It stays while
 *    Genie works. Only the stage line is live for a screen reader.
 * 2. Answer: the pill grows into the answer card: the question, a short summary of the answer
 *    (safe markdown, clamped to a few lines with "Show more" to read the rest), the key numbers
 *    and a footer with [Open in Bottle] and a close control. Failure: "Genie could not answer.",
 *    the GL-5 reason and Try again.
 * 3. Gone: an answer stays for T = 4 s + 0.25 s per word (6 s to 20 s), a failure 8 s. The pointer
 *    on the card or on the MOG holds it; after the pointer leaves it stays 2.5 s or more. Then it
 *    fades and tells the brain (`card_done`), which hides the window.
 *
 * A single click on [Open in Bottle] or on the card body (not a link, a button, or a press-drag
 * release), and a double-click anywhere on the card, open hub Conversations on this exchange. The
 * close control fades the card at once, the same as a double-click. The card never
 * takes focus: it calls no `focus()`, and the window shows without becoming key (ADR 0102).
 * Design: the designer-mog spec, sections 4 and 5 (ADR 0102), on `tokens.css` only.
 */
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent as ReactMouseEvent,
} from 'react';
import { listen } from '@tauri-apps/api/event';
import { ExternalLink } from 'lucide-react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { MogId } from '../../shared/interfaces';
import {
  AFTER_HOVER_MS,
  ANSWER_READY,
  CARD_DISMISS_EVENT,
  CARD_DONE,
  CARD_MOG_HOVER_EVENT,
  FAILURE_SHOW_MS,
  OPEN_CONVERSATIONS,
  answerShowMs,
  opIdOfGenieBlurb,
  timerText,
} from '../../shared/genie-status';
import { paletteColorOf } from '../../shared/mog-palette';
import { emitView, type CardUpdate } from '../view-ipc';
import { LINK_OPEN, openInDatabricksUrl } from '../../shared/link-trust';
import { SafeMarkdown } from './safe-markdown';
import { MemorySavedLive, NO_SAVED, type MemorySavedApi } from './memory-saved';

/** The card fade at its time; a double-click fades faster. */
export const FADE_MS = 300;
export const DISMISS_FADE_MS = 150;
/** The stage line change: the old line leaves in this time. */
export const STAGE_LEAVE_MS = 120;
/** The failure title (design spec 5). */
export const FAILURE_TITLE = 'Genie could not answer.';
/** The footer action that opens hub Conversations on this exchange (also a single click on the card). */
export const OPEN_IN_BOTTLE_LABEL = 'Open in Bottle';
/** #523: the footer link to the same conversation in the workspace (the brain opens it). */
export const OPEN_IN_DATABRICKS_LABEL = 'Open in Databricks';
export const OPEN_IN_DATABRICKS_ARIA = 'Open in Databricks (opens in browser)';
export const TABLE_MORE_HINT = 'Open in Bottle to see them all';
export const SHOW_MORE_LABEL = 'Show more';
export const SHOW_LESS_LABEL = 'Show less';
export const CLOSE_LABEL = 'Close';
/** Memory slice 1 (S1): one click keeps this answer in today's note of the MOG. */
export const REMEMBER_LABEL = 'Remember this';
export const REMEMBERED_LABEL = 'In today’s note';
/** A click that gets no answer from the brain frees the button after this time. */
export const REMEMBER_WAIT_MS = 10_000;
/** A press that moves more than this before release is a drag, not a click (review round 2). */
export const DRAG_SLOP_PX = 4;
/** `var(--mog-leading)` is 1.45 (`tokens.css`); 14px is `var(--mog-text-md)`'s usual size. */
export const ANSWER_CLAMP_LINES = 4;
const APPROX_LINE_PX = 14 * 1.45;
/** The one clamp height. It goes to the CSS as `--genie-clamp-px` (`genie-card.css`), so the
 * real, CSS-driven clamp and the overflow check below can never drift apart. */
export const ANSWER_CLAMP_PX = ANSWER_CLAMP_LINES * APPROX_LINE_PX;
/** The whole answer card's own hard cap (`.genie-answer`/`.genie-failure`/`.genie-message` in
 * genie-card.css). The expanded body's target height is always derived from this minus the
 * chrome around it (question, key numbers, footer), never a constant (review round 2, finding
 * 1): a constant does not know whether key numbers show, so it either clips the footer or wastes
 * space, depending on content. The card is a flex column (genie-card.css) as the safety net in
 * case this estimate is ever wrong, so the footer and the close control can never clip either way.
 */
export const ANSWER_CARD_MAX_PX = 280;
/** The expanded body's height before anything has been measured (first render, or a measurement
 * that comes back zero, e.g. in a test with no real layout). `toggleExpanded` replaces this with
 * a chrome-aware value on every Show more. */
export const ANSWER_EXPANDED_MAX_PX = 180;

type Vis = 'shown' | 'fading' | 'dismissing' | 'gone';

const reducedMotion = () =>
  typeof window.matchMedia === 'function' &&
  window.matchMedia('(prefers-reduced-motion: reduce)').matches;

/** Listen to one brain event sent to this window only (ARC-5). `fn` must be stable. */
function useWindowEvent(name: string, fn: (payload: unknown) => void) {
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    const target = getCurrentWindow().label;
    void listen<unknown>(name, (e) => fn(e.payload), { target }).then((u) => {
      if (gone) u();
      else off = u;
    });
    return () => {
      gone = true;
      off?.();
    };
  }, [name, fn]);
}

/** The stage line with its change motion: the old line leaves while the new one comes in. */
function Stage({ text, live }: { text: string; live?: string | undefined }) {
  const [shown, setShown] = useState({ text, leaving: undefined as string | undefined });
  if (text !== shown.text) setShown({ text, leaving: shown.text });
  useEffect(() => {
    if (shown.leaving === undefined) return;
    const t = setTimeout(() => setShown((s) => ({ ...s, leaving: undefined })), STAGE_LEAVE_MS);
    return () => clearTimeout(t);
  }, [shown.leaving]);
  return (
    <span className="genie-stage-slot">
      {/* One stable live region, so a screen reader reads each new stage once. */}
      <span className="genie-sr" data-genie-live aria-live="polite">
        {live ?? shown.text}
      </span>
      {shown.leaving !== undefined && (
        <span className="genie-stage" data-leaving aria-hidden="true">
          {shown.leaving}
        </span>
      )}
      <span key={shown.text} className="genie-stage" data-genie-stage aria-hidden="true">
        {shown.text}
      </span>
    </span>
  );
}

/** The status timer, from the time this request first showed in this window. */
function Timer() {
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    const start = Date.now();
    const t = setInterval(() => setElapsed(Date.now() - start), 1000);
    return () => clearInterval(t);
  }, []);
  const text = timerText(elapsed);
  // The slot is kept before 3 s, so the pill does not grow when the timer shows.
  return (
    <span
      className="genie-timer"
      data-genie-timer={text === undefined ? undefined : ''}
      data-waiting={text === undefined ? '' : undefined}
      aria-hidden="true"
    >
      {text ?? '0:00'}
    </span>
  );
}

/** "8,800 tables" -> value "8,800", label "tables". A number with no label keeps it all. */
function splitKeyNumber(k: string): { value: string; label: string } {
  const m = /^(\S*\d\S*)\s+(.+)$/.exec(k.trim());
  return m ? { value: m[1] ?? k, label: m[2] ?? '' } : { value: k, label: '' };
}

/** The question of the request: the ask gate's quoted blurb (`ask:<utterance>`). */
function questionOf(card: CardUpdate): string | undefined {
  const b = card.blurbs.find((x) => x.id.startsWith('ask:'));
  const line = b?.text.split('\n')[0]?.trim();
  if (!line) return undefined;
  return line.replace(/^"([\s\S]*)"$/, '$1');
}

/**
 * The quiet "Open in Databricks" link of the pill and the footer. It sends the same `link_open`
 * click; the brain and the host check the URL again (#523).
 */
function DatabricksLink({
  url,
  onOpen,
  className = '',
}: {
  url: string;
  onOpen: (url: string) => void;
  className?: string;
}) {
  return (
    <a
      className={`genie-toggle genie-databricks ${className}`.trim()}
      data-action="open-in-databricks"
      href={url}
      rel="noopener noreferrer"
      title={url}
      aria-label={OPEN_IN_DATABRICKS_ARIA}
      onDoubleClick={(e) => e.stopPropagation()}
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        if (e.detail > 1) return;
        onOpen(url);
      }}
    >
      {OPEN_IN_DATABRICKS_LABEL}
      <ExternalLink size={11} strokeWidth={2} aria-hidden="true" />
    </a>
  );
}

export interface GenieCardProps {
  mogId: MogId;
  card: CardUpdate;
  /**
   * The final height of the card surface with its frame (logical px), after each change of its
   * content and before any size motion, so the window can fit it (`fit-window.ts`).
   */
  onSize?: (px: number) => Promise<void> | void;
  /**
   * #ui-4: the target MOG's name and colour (`CardView.tsx`, `useCardIdentity`). The card is
   * already target-only (MOG-13a, MOG-13b: the owner of the request is the MOG that captured
   * it), so this only labels it; undefined before the brain answers shows no label.
   */
  identity?: { name: string; colorVariant?: string } | undefined;
  /** Memory slice 1: the `memory:saved` line of this window (`useMemorySaved` in CardView). */
  saved?: MemorySavedApi;
  /** The pill sits over a pop-up in the one card window (`CardView.tsx`). */
  stacked?: boolean;
  /** The user closed the working or ready pill: the view keeps it out of sight. */
  onClosePill?: () => void;
}

export function GenieCard({
  mogId,
  card,
  onSize,
  identity,
  saved = NO_SAVED,
  stacked = false,
  onClosePill,
}: GenieCardProps) {
  const accent = identity && paletteColorOf(identity.colorVariant, mogId).hex;
  const newest = card.blurbs[0];
  const blurbId = newest?.id ?? '';
  const opId = opIdOfGenieBlurb(blurbId);
  const working = newest !== undefined && !newest.completed;
  const answer = card.sections?.find((s) => s.id === 'answer');
  const keys = card.sections?.find((s) => s.id === 'keyNumbers');
  const footer = card.sections?.find((s) => s.id === 'footer');
  // #523: links are live only for the workspace the answer came from. The brain checks the click
  // again; a link to any other host is text.
  const linkTrust = useMemo(
    () => ({ profileHost: footer?.workspaceHost ?? '', allowedAppUrls: [] as string[] }),
    [footer?.workspaceHost],
  );
  const databricksUrl = openInDatabricksUrl(footer?.deepLink, footer?.workspaceHost);
  const openLink = useCallback(
    (url: string) => {
      void emitView(mogId, 'click', { target: LINK_OPEN, url }).catch(() => undefined);
    },
    [mogId],
  );
  const retry = card.chips.find((c) => c.id === `retry:${blurbId}`);
  const changeWorkspace = card.chips.find((c) => c.id === `workspace:${blurbId}`);
  // Slice 2a, R-8: the chip that sends this question to the agent too. Its id names the request.
  const askOthers = card.chips.find((c) => c.id === `agent-more:${opId}`);
  const question = questionOf(card);
  const kind = working ? 'working' : answer ? 'answer' : retry ? 'failure' : 'message';
  const text = newest?.text ?? '';
  // UX slice 1: an answer is waiting for the talk-key hold to end. The pill says so, with no timer.
  const ready = working && text.split('\n')[0] === ANSWER_READY;
  const showMs = kind === 'failure' ? FAILURE_SHOW_MS : answerShowMs(answer?.md ?? text);

  const [vis, setVis] = useState<Vis>('shown');
  const ref = useRef<HTMLDivElement>(null);
  const holds = useRef({ pointer: false, mog: false });
  // UX slice 1: the width of the working pill, so the ready pill keeps it (the timer was in it).
  const workWidth = useRef(0);
  // The point of the last `mousedown` on the card, so `openInBottle` can tell a click from a
  // press-drag-release (review round 2, finding "also fix if cheap"): a drag never opens the hub.
  const pressPos = useRef<{ x: number; y: number } | null>(null);
  // `hold` and the event handlers read these refs, so they stay stable and never miss an event.
  const visRef = useRef<Vis>('shown');
  const workingRef = useRef(working);
  useLayoutEffect(() => {
    visRef.current = vis;
    workingRef.current = working;
  });
  const timer = useRef<{ handle?: ReturnType<typeof setTimeout>; due: number; left: number }>({
    due: 0,
    left: showMs,
  });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || kind !== 'working') return;
    if (!ready) workWidth.current = el.getBoundingClientRect().width;
    el.style.minWidth = ready && workWidth.current > 0 ? `${workWidth.current}px` : '';
    if (ready || typeof ResizeObserver === 'undefined') return;
    // The timer comes in after 3 s and grows the pill without a render of this component.
    const ro = new ResizeObserver(() => {
      workWidth.current = el.getBoundingClientRect().width;
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  // The window fits the content's final size first, so the grow motion is never cut (used below,
  // and by the Show more/less settle effect, finding 3, review round 1).
  const fitted = useRef<Promise<void>>(Promise.resolve());
  const measure = useCallback(() => {
    const frame = ref.current?.parentElement;
    return onSize ? onSize(frame ? frame.getBoundingClientRect().height : 0) : undefined;
  }, [onSize]);

  // The answer clamps to a short summary by default (ANSWER_CLAMP_LINES); "Show more" expands it
  // to its own bounded scroll region (genie-card.css), "Show less" clamps it again. A new answer
  // always starts collapsed.
  // Memory S1: idle, a click waiting for the brain, or kept (the answer is in today's note).
  const [remember, setRemember] = useState<'idle' | 'sending' | 'kept'>('idle');
  const remembering = remember === 'sending';
  const [expanded, setExpanded] = useState(false);
  const [canExpand, setCanExpand] = useState(false);
  const [moreBelow, setMoreBelow] = useState(false);
  const [scrolledTop, setScrolledTop] = useState(false);
  // The expanded body's target height, recomputed on every Show more from the chrome actually
  // measured at that moment (review round 2, finding 1): see `toggleExpanded`.
  const [expandedCapPx, setExpandedCapPx] = useState(ANSWER_EXPANDED_MAX_PX);
  const answerRef = useRef<HTMLDivElement>(null);
  const answerBodyId = useId();
  // Reset during render (not in an effect), the documented way to clear state on a prop change:
  // https://react.dev/learn/you-might-not-need-an-effect#resetting-all-state-when-a-prop-changes
  const prevBlurbId = useRef(blurbId);
  if (prevBlurbId.current !== blurbId) {
    prevBlurbId.current = blurbId;
    if (expanded) setExpanded(false);
    if (remember !== 'idle') setRemember('idle');
  }
  // A new memory:saved line settles the click: kept after a save, back to idle otherwise.
  const [seenLine, setSeenLine] = useState(saved.line);
  if (seenLine !== saved.line) {
    setSeenLine(saved.line);
    const o = saved.line?.outcome;
    // Only a click of this card marks it: a voice save leaves the answer card alone.
    if (remember === 'sending') {
      if (o === 'saved' || o === 'quarantined') setRemember('kept');
      else if (o !== undefined) setRemember('idle');
    }
  }
  useLayoutEffect(() => {
    const el = answerRef.current;
    // `scrollHeight` is the content's natural height, unclamped by this element's own
    // `max-height`, so this never flips once the user expands it.
    setCanExpand(el ? el.scrollHeight > ANSWER_CLAMP_PX + 1 : false);
  }, [kind, text, answer?.md]);
  // B3: a bottom fade shows only while the expanded, scrolled answer still has more below it.
  // [LOW, round 3] A top fade shows the same way once the user has scrolled past the top, to
  // match: the content was cut hard under the question with no warning that it had scrolled.
  const updateMoreBelow = useCallback(() => {
    const el = answerRef.current;
    setMoreBelow(el ? el.scrollHeight - el.scrollTop - el.clientHeight > 1 : false);
    setScrolledTop(el ? el.scrollTop > 0 : false);
  }, []);
  useLayoutEffect(updateMoreBelow, [expanded, canExpand, updateMoreBelow]);
  // B5/finding 3 (review round 1): the answer's own `max-height` animates over 200 ms, so a size
  // read on the frame that toggles `expanded` is still the old, pre-transition size (the window
  // would not grow on Show more, or would stay tall on Show less). Wait for the real
  // `transitionend` and measure once the layout has actually settled, so the window moves once,
  // with no jump. Reduced motion never transitions (genie-card.css), so there measure at once.
  useEffect(() => {
    const el = answerRef.current;
    if (!el) return;
    const settle = () => {
      void measure();
      updateMoreBelow();
    };
    if (reducedMotion()) {
      settle();
      return;
    }
    const onTransitionEnd = (e: TransitionEvent) => {
      if (e.target !== el || e.propertyName !== 'max-height') return;
      settle();
    };
    el.addEventListener('transitionend', onTransitionEnd);
    return () => el.removeEventListener('transitionend', onTransitionEnd);
  }, [expanded, measure, updateMoreBelow]);

  const end = useCallback(
    (fadeMs: number) => {
      // A card that is gone sends no second `card_done`.
      if (visRef.current === 'gone') return;
      clearTimeout(timer.current.handle);
      setVis(fadeMs === DISMISS_FADE_MS ? 'dismissing' : 'fading');
      timer.current.handle = setTimeout(() => {
        setVis('gone');
        void emitView(mogId, 'click', {
          target: CARD_DONE,
          ...(opId ? { requestId: opId } : {}),
        }).catch(() => undefined);
      }, fadeMs);
    },
    [mogId, opId],
  );

  const arm = useCallback(
    (ms: number) => {
      clearTimeout(timer.current.handle);
      timer.current.due = Date.now() + ms;
      timer.current.handle = setTimeout(() => end(FADE_MS), ms);
    },
    [end],
  );

  // Memory S1: "Saved to today's note · Undo". The card stays for the Undo time, so the user can
  // still click Undo. A pointer or MOG hold keeps its own say (`arm` runs only when none holds).
  const line = saved.line;
  useEffect(() => {
    if (!line || workingRef.current || visRef.current === 'gone') return;
    timer.current.left = Math.max(timer.current.left, line.ms + AFTER_HOVER_MS);
    if (!holds.current.pointer && !holds.current.mog) arm(timer.current.left);
  }, [line, arm]);
  // A click the brain never answers does not lock the button for good.
  useEffect(() => {
    if (remember !== 'sending') return;
    const t = setTimeout(() => setRemember('idle'), REMEMBER_WAIT_MS);
    return () => clearTimeout(t);
  }, [remember]);
  const doRemember = useCallback(() => {
    // The brain links the memory to the exchange by this id: no op id, no click.
    if (remembering || workingRef.current || !opId) return;
    setRemember('sending');
    void emitView(mogId, 'click', { action: 'memory.remember', requestId: opId }).catch(() =>
      setRemember('idle'),
    );
  }, [mogId, opId, remembering]);

  // The show time starts when the outcome shows; the working pill never fades.
  useEffect(() => {
    if (working) return;
    timer.current.left = showMs;
    if (!holds.current.pointer && !holds.current.mog) arm(showMs);
    const t = timer.current;
    return () => clearTimeout(t.handle);
  }, [working, showMs, arm]);

  const hold = useCallback(
    (via: 'pointer' | 'mog', on: boolean) => {
      const h = holds.current;
      const was = h.pointer || h.mog;
      h[via] = on;
      const is = h.pointer || h.mog;
      // The brain's own card stack ages a completed blurb out after its own, longer idle time
      // (UX-12), on a timer that never saw this hold. Report it the same way the panel already
      // does (`hover`), so the brain never ages this answer out while the user still reads it.
      if (blurbId)
        void emitView(mogId, 'hover', { blurbId, on: is, via: 'pointer' }).catch(() => undefined);
      if (workingRef.current || visRef.current === 'gone' || was === is) return;
      if (is) {
        clearTimeout(timer.current.handle);
        timer.current.left = Math.max(0, timer.current.due - Date.now());
        // A hold during the fade brings the card back.
        setVis('shown');
      } else arm(Math.max(timer.current.left, AFTER_HOVER_MS));
    },
    [arm, mogId, blurbId],
  );

  const onMogHover = useCallback(
    (p: unknown) => {
      const on = typeof p === 'object' && p !== null && (p as { on?: unknown }).on;
      if (typeof on === 'boolean') hold('mog', on);
    },
    [hold],
  );
  useWindowEvent(CARD_MOG_HOVER_EVENT, onMogHover);
  const onDismiss = useCallback(() => {
    if (!workingRef.current && visRef.current !== 'gone') end(DISMISS_FADE_MS);
  }, [end]);
  useWindowEvent(CARD_DISMISS_EVENT, onDismiss);

  const onPress = (e: ReactMouseEvent) => {
    pressPos.current = { x: e.clientX, y: e.clientY };
  };

  // A single click on the card body or on [Open in Bottle] opens hub Conversations, the same as
  // a double-click (ADR 0102, #103). A link never does: it asks the brain to open it (#523).
  const openInBottle = useCallback(
    (e: ReactMouseEvent) => {
      // Only a true single click, on a card that is still fully shown: the second click of a
      // double-click, and the dblclick after it, send nothing more (cheap item from review).
      if (workingRef.current || visRef.current !== 'shown' || e.detail > 1) return;
      if ((e.target as Element | null)?.closest('a')) return;
      // `.genie` is `user-select:none` (genie-card.css), so there is never a text selection to
      // check here; a press-drag-release is caught by comparing against the press point instead
      // (review round 2, "also fix if cheap" — the old selection check did nothing).
      const start = pressPos.current;
      pressPos.current = null;
      if (
        start &&
        (Math.abs(e.clientX - start.x) > DRAG_SLOP_PX ||
          Math.abs(e.clientY - start.y) > DRAG_SLOP_PX)
      )
        return;
      void emitView(mogId, 'click', {
        target: OPEN_CONVERSATIONS,
        ...(opId ? { requestId: opId } : {}),
      }).catch(() => undefined);
      end(DISMISS_FADE_MS);
    },
    [mogId, opId, end],
  );

  // A card that is gone asks for one line, so a later show never gets the old, taller window
  // back. `expanded` is not a dependency here: Show more/less resize goes through the dedicated
  // effect above, which waits for the answer's own `max-height` transition to settle before it
  // measures (finding 3, review round 1) instead of measuring this frame's pre-transition size.
  useLayoutEffect(() => {
    if (!onSize) return;
    fitted.current = Promise.resolve(measure());
  }, [onSize, measure, kind, text, card.sections, card.blurbs, retry, vis, saved.line]);

  // M3: a very long question scrolls inside its own capped area (the status row and the link row
  // stay outside it). A bottom fade shows only while it overflows.
  const pillQuestionRef = useRef<HTMLParagraphElement>(null);
  const [questionOverflows, setQuestionOverflows] = useState(false);
  // The fade shows while more of the question lies below: it goes at the scroll end.
  const updateQuestionFade = useCallback(() => {
    const el = pillQuestionRef.current;
    setQuestionOverflows(el ? el.scrollHeight - el.scrollTop - el.clientHeight > 1 : false);
  }, []);
  useLayoutEffect(updateQuestionFade, [kind, question, updateQuestionFade]);
  // A resize or a fit of the window changes the width, so the line breaks: measure again.
  useEffect(() => {
    window.addEventListener('resize', updateQuestionFade);
    return () => window.removeEventListener('resize', updateQuestionFade);
  }, [updateQuestionFade]);
  // The window fit changes the pill's size after the first layout: check the fade once more.
  useEffect(() => {
    void fitted.current.then(updateQuestionFade, updateQuestionFade);
  }, [kind, text, question, updateQuestionFade]);

  // The pill grows into the card: the same element animates from its old size and corners to
  // the new ones. The motion waits until the window has its new height.
  const prev = useRef<
    { kind: string; w: number; h: number; radius: string; capsule: boolean } | undefined
  >(undefined);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const radius = getComputedStyle(el).borderRadius;
    const was = prev.current;
    // A one-line pill is a capsule (999px token): its real corner is half its height. A pill with
    // a question has a card-like radius, so its computed radius is the real start.
    const capsule = kind === 'working' && el.querySelector('.genie-pill-question') === null;
    prev.current = { kind, w: r.width, h: r.height, radius, capsule };
    if (!was || was.kind === kind || reducedMotion() || typeof el.animate !== 'function') return;
    const from = was.capsule ? `${was.h / 2}px` : was.radius;
    const anim = el.animate(
      [
        { width: `${was.w}px`, height: `${was.h}px`, borderRadius: from, overflow: 'hidden' },
        {
          width: `${r.width}px`,
          height: `${r.height}px`,
          borderRadius: radius,
          overflow: 'hidden',
        },
      ],
      { duration: 220, easing: 'cubic-bezier(0.16, 1, 0.3, 1)' },
    );
    anim.pause();
    void fitted.current.then(
      () => anim.play(),
      () => anim.play(),
    );
  }, [kind]);

  // A faded answer that the brain cleared (empty text, no answer) shows nothing.
  if (!newest || vis === 'gone' || (!working && text === '' && !answer)) return null;

  const onDoubleClick = () => {
    // The card's own single click already sent this and started the fade (cheap item from
    // review): a double-click never sends it a second time.
    if (visRef.current !== 'shown') return;
    void emitView(mogId, 'click', {
      target: OPEN_CONVERSATIONS,
      ...(opId ? { requestId: opId } : {}),
    }).catch(() => undefined);
    if (!working) end(DISMISS_FADE_MS);
  };
  const [line1 = '', ...rest] = text.split('\n');

  const toggleExpanded = (e: ReactMouseEvent) => {
    e.stopPropagation();
    if (expanded) {
      // Collapsing: reset the scroll position first (round 3): otherwise the collapsed summary
      // keeps showing wherever the user last scrolled to, cut at the top instead of starting at
      // the heading. Then shrink the body (the CSS transition below), then shrink the window
      // once it settles (the `transitionend` effect above). That order is unchanged from round 1.
      if (answerRef.current) answerRef.current.scrollTop = 0;
      setExpanded(false);
      return;
    }
    // Expanding (review round 2, finding 2): grow the window to the target size *before* the
    // body's own `max-height` transition runs, so the footer is never below the window edge
    // mid-transition and the window never snaps once it ends ("the window fits the final size
    // first", the same pattern this file already uses for the pill-to-card grow motion).
    const el = answerRef.current;
    const genieEl = ref.current;
    const frame = genieEl?.parentElement;
    const bodyNow = el?.getBoundingClientRect().height ?? 0;
    const genieNow = genieEl?.getBoundingClientRect().height ?? 0;
    const frameNow = frame?.getBoundingClientRect().height ?? 0;
    // Everything in the card besides the body: question, key numbers, footer, padding, border.
    const chrome = Math.max(0, genieNow - bodyNow);
    const available = Math.max(ANSWER_CLAMP_PX, ANSWER_CARD_MAX_PX - chrome);
    const targetBody = el ? Math.min(el.scrollHeight, available) : ANSWER_EXPANDED_MAX_PX;
    setExpandedCapPx(targetBody);
    const targetFrame = frameNow + Math.max(0, targetBody - bodyNow);
    const grown = Promise.resolve(onSize?.(targetFrame));
    void grown.then(() => setExpanded(true));
  };
  const clampStyle = {
    '--genie-clamp-px': `${ANSWER_CLAMP_PX}px`,
    '--genie-expanded-max-px': `${expandedCapPx}px`,
  } as CSSProperties;
  // The safe-markdown answer, clamped to a short summary. "Show more"/"Show less" (rendered once,
  // in the footer) lift or reapply the clamp when it overflows ANSWER_CLAMP_LINES. Shared by the
  // 'answer' and 'message' kinds (UX-14a, #103).
  const answerBody = (md: string) => (
    <div
      ref={answerRef}
      id={answerBodyId}
      className="genie-answer-body"
      style={clampStyle}
      data-expanded={expanded}
      data-overflowing={canExpand}
      data-more-below={moreBelow}
      data-scrolled-top={scrolledTop}
      onScroll={updateMoreBelow}
    >
      <SafeMarkdown
        text={md}
        trust={linkTrust}
        onOpenLink={openLink}
        overflowHint={TABLE_MORE_HINT}
      />
    </div>
  );

  return (
    <div
      ref={ref}
      className={`genie genie-${kind}`}
      data-genie={kind}
      data-ready={ready ? '' : undefined}
      data-state={vis}
      style={accent ? ({ '--mog-id-color': accent } as CSSProperties) : undefined}
      onMouseEnter={() => hold('pointer', true)}
      onMouseLeave={() => hold('pointer', false)}
      onMouseDown={onPress}
      onClick={openInBottle}
      onDoubleClick={onDoubleClick}
    >
      {/* #ui-4: this card is already target-only (MOG-13a, MOG-13b); this only names the MOG.
          Review #603, finding 5: the name is neutral text (AA on this surface); the dot, not the
          text, carries the accent colour. */}
      {identity && (
        <span className="genie-identity">
          <span className="genie-identity-dot" aria-hidden="true" />
          {identity.name}
        </span>
      )}
      {kind === 'working' && (
        <>
          <Stage
            text={line1}
            live={ready && identity ? `${identity.name}: ${ANSWER_READY}` : undefined}
          />
          {ready ? <span className="genie-ready-dot" aria-hidden="true" /> : <Timer />}
          {question !== undefined && (
            <p
              ref={pillQuestionRef}
              className="genie-pill-question"
              data-genie-pill-question
              data-overflow={questionOverflows ? '' : undefined}
              onScroll={updateQuestionFade}
            >
              {question}
            </p>
          )}
          {/* The link row has its height from the first frame, so "Answer ready" never changes the
              size of the pill (the window would move near the screen bottom). */}
          {(question !== undefined || (ready && databricksUrl !== undefined)) && (
            <div className="genie-pill-linkrow">
              {ready && databricksUrl !== undefined ? (
                <DatabricksLink url={databricksUrl} onOpen={openLink} className="genie-pill-link" />
              ) : (
                <span
                  className="genie-toggle genie-databricks genie-pill-link genie-pill-link-ph"
                  data-pill-link-placeholder
                  aria-hidden="true"
                >
                  {OPEN_IN_DATABRICKS_LABEL}
                  <ExternalLink size={11} strokeWidth={2} aria-hidden="true" />
                </span>
              )}
            </div>
          )}
        </>
      )}
      <button
        type="button"
        className="popup-dismiss genie-dismiss"
        data-action="dismiss"
        aria-label={CLOSE_LABEL}
        onDoubleClick={(e) => e.stopPropagation()}
        onClick={(e) => {
          e.stopPropagation();
          if (visRef.current === 'gone') return;
          if (workingRef.current) {
            // The user closed the pill. The view keeps it out of sight at once; the brain hides
            // the window (a working request runs on and shows its answer when it lands). Over a
            // pop-up, the pop-up stays: only a ready pill tells the brain, to keep it closed.
            onClosePill?.();
            if (!stacked)
              void emitView(mogId, 'click', {
                target: CARD_DONE,
                ...(opId ? { requestId: opId } : {}),
              }).catch(() => undefined);
            else if (ready)
              void emitView(mogId, 'click', {
                target: CARD_DONE,
                pillOnly: true,
                ...(opId ? { requestId: opId } : {}),
              }).catch(() => undefined);
            return;
          }
          end(DISMISS_FADE_MS);
        }}
      >
        <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
          <path d="M2 2l6 6M8 2l-6 6" />
        </svg>
      </button>
      {kind !== 'working' && (
        <div className="genie-body" key="body">
          {question !== undefined && (
            <p className="genie-question" data-genie-question title={question}>
              {question}
            </p>
          )}
          {kind === 'answer' && answer && (
            <div className="genie-md" data-answer role="status">
              {answerBody(answer.md)}
            </div>
          )}
          {kind === 'message' && (
            <div className="genie-md" data-answer>
              {answerBody([line1, ...rest].join('\n'))}
            </div>
          )}
          {kind === 'answer' && keys && keys.items.length > 0 && (
            <ul className="genie-keys" aria-label="Key numbers">
              {keys.items.map((k, i) => {
                const { value, label } = splitKeyNumber(k);
                return (
                  <li key={i}>
                    <span className="genie-key-value">{value}</span>
                    {label && <span className="genie-key-label">{label}</span>}
                  </li>
                );
              })}
            </ul>
          )}
          {kind === 'failure' && (
            <div className="genie-failure-body" role="alert">
              <p className="genie-title">{FAILURE_TITLE}</p>
              <p className="genie-reason">{line1}</p>
            </div>
          )}
          <footer className="genie-foot">
            {retry ? (
              <>
                <button
                  type="button"
                  className="popup-action"
                  data-chip={retry.id}
                  onDoubleClick={(e) => e.stopPropagation()}
                  onClick={(e) => {
                    e.stopPropagation();
                    // The second click of a double-click sends nothing more.
                    if (e.detail > 1) return;
                    void emitView(mogId, 'chip_click', { chipId: retry.id }).catch(() => undefined);
                  }}
                >
                  {retry.label}
                </button>
                {changeWorkspace && (
                  <button
                    type="button"
                    className="genie-toggle genie-workspace"
                    data-chip={changeWorkspace.id}
                    onDoubleClick={(e) => e.stopPropagation()}
                    onClick={(e) => {
                      e.stopPropagation();
                      if (e.detail > 1) return;
                      void emitView(mogId, 'chip_click', { chipId: changeWorkspace.id }).catch(
                        () => undefined,
                      );
                    }}
                  >
                    {changeWorkspace.label}
                  </button>
                )}
              </>
            ) : (
              <>
                <button
                  type="button"
                  className="popup-action genie-open"
                  data-action="open-in-bottle"
                  onDoubleClick={(e) => e.stopPropagation()}
                  onClick={(e) => {
                    e.stopPropagation();
                    openInBottle(e);
                  }}
                >
                  {OPEN_IN_BOTTLE_LABEL}
                </button>
                {kind === 'answer' && databricksUrl !== undefined && (
                  <DatabricksLink url={databricksUrl} onOpen={openLink} />
                )}
                {kind === 'answer' && askOthers && (
                  <button
                    type="button"
                    className="genie-toggle genie-ask-others"
                    data-chip={askOthers.id}
                    onDoubleClick={(e) => e.stopPropagation()}
                    onClick={(e) => {
                      e.stopPropagation();
                      if (e.detail > 1) return;
                      void emitView(mogId, 'chip_click', { chipId: askOthers.id }).catch(
                        () => undefined,
                      );
                    }}
                  >
                    {askOthers.label}
                  </button>
                )}
                {kind === 'answer' &&
                  (remember === 'kept' ? (
                    <span className="genie-remembered" data-remembered>
                      {REMEMBERED_LABEL}
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="genie-remember"
                      data-action="remember"
                      aria-label="Remember this answer in today’s note"
                      disabled={remembering}
                      onDoubleClick={(e) => e.stopPropagation()}
                      onClick={(e) => {
                        e.stopPropagation();
                        if (e.detail > 1) return;
                        doRemember();
                      }}
                    >
                      {REMEMBER_LABEL}
                    </button>
                  ))}
                {(kind === 'answer' || kind === 'message') && canExpand && (
                  <button
                    type="button"
                    className="genie-toggle"
                    data-action={expanded ? 'show-less' : 'show-more'}
                    aria-expanded={expanded}
                    aria-controls={answerBodyId}
                    onDoubleClick={(e) => e.stopPropagation()}
                    onClick={toggleExpanded}
                  >
                    {expanded ? SHOW_LESS_LABEL : SHOW_MORE_LABEL}
                  </button>
                )}
              </>
            )}
            <span className="genie-when">now</span>
          </footer>
          <MemorySavedLive line={saved.line} onUndo={saved.undo} />
        </div>
      )}
    </div>
  );
}
