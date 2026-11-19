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
 * A single click on [Open in Bottle] or on the card body (not a link, a button, or a text
 * selection drag), and a double-click anywhere on the card, open hub Conversations on this
 * exchange. The close control fades the card at once, the same as a double-click. The card never
 * takes focus: it calls no `focus()`, and the window shows without becoming key (ADR 0102).
 * Design: the designer-mog spec, sections 4 and 5 (ADR 0102), on `tokens.css` only.
 */
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent as ReactMouseEvent,
} from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { MogId } from '../../shared/interfaces';
import {
  AFTER_HOVER_MS,
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
import { NO_LINKS, SafeMarkdown } from './safe-markdown';

/** The card fade at its time; a double-click fades faster. */
export const FADE_MS = 300;
export const DISMISS_FADE_MS = 150;
/** The stage line change: the old line leaves in this time. */
export const STAGE_LEAVE_MS = 120;
/** The failure title (design spec 5). */
export const FAILURE_TITLE = 'Genie could not answer.';
/** The footer action that opens hub Conversations on this exchange (also a single click on the card). */
export const OPEN_IN_BOTTLE_LABEL = 'Open in Bottle';
export const SHOW_MORE_LABEL = 'Show more';
export const SHOW_LESS_LABEL = 'Show less';
export const CLOSE_LABEL = 'Close';
/** The answer clamps to about this height (roughly 4 lines) until the user asks to see more. */
export const ANSWER_CLAMP_PX = 96;

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
function Stage({ text }: { text: string }) {
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
        {shown.text}
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
}

export function GenieCard({ mogId, card, onSize, identity }: GenieCardProps) {
  const accent = identity && paletteColorOf(identity.colorVariant, mogId).hex;
  const newest = card.blurbs[0];
  const blurbId = newest?.id ?? '';
  const opId = opIdOfGenieBlurb(blurbId);
  const working = newest !== undefined && !newest.completed;
  const answer = card.sections?.find((s) => s.id === 'answer');
  const keys = card.sections?.find((s) => s.id === 'keyNumbers');
  const retry = card.chips.find((c) => c.id === `retry:${blurbId}`);
  const kind = working ? 'working' : answer ? 'answer' : retry ? 'failure' : 'message';
  const text = newest?.text ?? '';
  const showMs = kind === 'failure' ? FAILURE_SHOW_MS : answerShowMs(answer?.md ?? text);

  const [vis, setVis] = useState<Vis>('shown');
  const ref = useRef<HTMLDivElement>(null);
  const holds = useRef({ pointer: false, mog: false });
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

  // The answer clamps to a short summary by default (ANSWER_CLAMP_PX); "Show more" expands it to
  // the full scroll region (`.genie-answer`'s own cap), "Show less" clamps it again. A new answer
  // always starts collapsed.
  const [expanded, setExpanded] = useState(false);
  const [canExpand, setCanExpand] = useState(false);
  const answerRef = useRef<HTMLDivElement>(null);
  // Reset during render (not in an effect), the documented way to clear state on a prop change:
  // https://react.dev/learn/you-might-not-need-an-effect#resetting-all-state-when-a-prop-changes
  const prevBlurbId = useRef(blurbId);
  if (prevBlurbId.current !== blurbId) {
    prevBlurbId.current = blurbId;
    if (expanded) setExpanded(false);
  }
  useLayoutEffect(() => {
    const el = answerRef.current;
    // `scrollHeight` is the content's natural height, unclamped by this element's own
    // `max-height`, so this never flips once the user expands it.
    setCanExpand(el ? el.scrollHeight > ANSWER_CLAMP_PX + 1 : false);
  }, [kind, text, answer?.md]);

  const end = useCallback(
    (fadeMs: number) => {
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
      if (workingRef.current || visRef.current === 'gone' || was === is) return;
      if (is) {
        clearTimeout(timer.current.handle);
        timer.current.left = Math.max(0, timer.current.due - Date.now());
        // A hold during the fade brings the card back.
        setVis('shown');
      } else arm(Math.max(timer.current.left, AFTER_HOVER_MS));
    },
    [arm],
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

  // A single click on the card body or on [Open in Bottle] opens hub Conversations, the same as
  // a double-click (ADR 0102, #103). A link stays inert (TODO(#523)); a text-selection drag never
  // navigates, though `.genie` already disables text selection.
  const openInBottle = useCallback(
    (e: ReactMouseEvent) => {
      if (workingRef.current) return;
      if ((e.target as Element | null)?.closest('a')) return;
      if ((document.getSelection()?.toString() ?? '') !== '') return;
      void emitView(mogId, 'click', {
        target: OPEN_CONVERSATIONS,
        ...(opId ? { requestId: opId } : {}),
      }).catch(() => undefined);
      if (visRef.current !== 'gone') end(DISMISS_FADE_MS);
    },
    [mogId, opId, end],
  );

  // The window fits the final size first, so the grow motion is never cut. A card that is gone
  // asks for one line, so a later show never gets the old, taller window back.
  const fitted = useRef<Promise<void>>(Promise.resolve());
  useLayoutEffect(() => {
    if (!onSize) return;
    const frame = ref.current?.parentElement;
    fitted.current = Promise.resolve(onSize(frame ? frame.getBoundingClientRect().height : 0));
  }, [onSize, kind, text, card.sections, card.blurbs, retry, vis, expanded]);

  // The pill grows into the card: the same element animates from its old size and corners to
  // the new ones. The motion waits until the window has its new height.
  const prev = useRef<{ kind: string; w: number; h: number; radius: string } | undefined>(
    undefined,
  );
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const radius = getComputedStyle(el).borderRadius;
    const was = prev.current;
    prev.current = { kind, w: r.width, h: r.height, radius };
    if (!was || was.kind === kind || reducedMotion() || typeof el.animate !== 'function') return;
    const from = was.kind === 'working' ? `${was.h / 2}px` : was.radius;
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
    void emitView(mogId, 'click', {
      target: OPEN_CONVERSATIONS,
      ...(opId ? { requestId: opId } : {}),
    }).catch(() => undefined);
    if (!working) end(DISMISS_FADE_MS);
  };
  const question = questionOf(card);
  const [line1 = '', ...rest] = text.split('\n');

  const toggleExpanded = (e: ReactMouseEvent) => {
    e.stopPropagation();
    setExpanded((x) => !x);
  };
  // The safe-markdown answer, clamped to a short summary. "Show more"/"Show less" (rendered once,
  // beside it) lift or reapply the clamp when it overflows ANSWER_CLAMP_PX. Shared by the
  // 'answer' and 'message' kinds (UX-14a, #103).
  const answerBody = (md: string) => (
    <div
      ref={answerRef}
      className="genie-answer-body"
      data-expanded={expanded}
      data-overflowing={canExpand}
    >
      <SafeMarkdown text={md} trust={NO_LINKS} />
    </div>
  );

  return (
    <div
      ref={ref}
      className={`genie genie-${kind}`}
      data-genie={kind}
      data-state={vis}
      style={accent ? ({ '--mog-id-color': accent } as CSSProperties) : undefined}
      onMouseEnter={() => hold('pointer', true)}
      onMouseLeave={() => hold('pointer', false)}
      onClick={openInBottle}
      onDoubleClick={onDoubleClick}
    >
      {/* #ui-4: this card is already target-only (MOG-13a, MOG-13b); this only names the MOG. */}
      {identity && <span className="genie-identity">{identity.name}</span>}
      {kind === 'working' && (
        <>
          <Stage text={line1} />
          <Timer />
        </>
      )}
      {kind !== 'working' && (
        <button
          type="button"
          className="popup-dismiss genie-dismiss"
          data-action="dismiss"
          aria-label={CLOSE_LABEL}
          onDoubleClick={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            if (!workingRef.current && visRef.current !== 'gone') end(DISMISS_FADE_MS);
          }}
        >
          <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
            <path d="M2 2l6 6M8 2l-6 6" />
          </svg>
        </button>
      )}
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
          {(kind === 'answer' || kind === 'message') && canExpand && (
            <button
              type="button"
              className="popup-action genie-toggle"
              data-action={expanded ? 'show-less' : 'show-more'}
              onDoubleClick={(e) => e.stopPropagation()}
              onClick={toggleExpanded}
            >
              {expanded ? SHOW_LESS_LABEL : SHOW_MORE_LABEL}
            </button>
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
            <div role="alert">
              <p className="genie-title">{FAILURE_TITLE}</p>
              <p className="genie-reason">{line1}</p>
            </div>
          )}
          <footer className="genie-foot">
            {retry ? (
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
            ) : (
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
            )}
            <span className="genie-when">now</span>
          </footer>
        </div>
      )}
    </div>
  );
}
