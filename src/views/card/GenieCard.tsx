/**
 * #573: the Ask Genie surface beside the MOG. One element goes through three states:
 *
 * 1. Working: the status pill. The stage line ("Asking Genie", then the fixed Genie stage) with a
 *    soft light sweep, and after 3 s the timer ("0:03", then "Working… 1:04"). It stays while
 *    Genie works. Only the stage line is live for a screen reader.
 * 2. Answer: the pill grows into the answer card: the question, the answer (safe markdown), the
 *    key numbers and a footer. Failure: "Genie could not answer.", the GL-5 reason and Try again.
 * 3. Gone: an answer stays for T = 4 s + 0.25 s per word (6 s to 20 s), a failure 8 s. The pointer
 *    on the card or on the MOG holds it; after the pointer leaves it stays 2.5 s or more. Then it
 *    fades and tells the brain (`card_done`), which hides the window. No close button, no drag.
 *
 * A double-click opens hub Conversations on this exchange. The card never takes focus: it calls
 * no `focus()`, and the window shows without becoming key (ADR 0102).
 * Design: the designer-mog spec, sections 4 and 5 (ADR 0102), on `tokens.css` only.
 */
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
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
import { emitView, type CardUpdate } from '../view-ipc';
import { SafeMarkdown, type LinkTrust } from './safe-markdown';

/** The card fade at its time; a double-click fades faster. */
export const FADE_MS = 300;
export const DISMISS_FADE_MS = 150;
/** The stage line change: the old line leaves in this time. */
export const STAGE_LEAVE_MS = 120;
/** The failure title (design spec 5). */
export const FAILURE_TITLE = 'Genie could not answer.';
export const FOOTER_HINT = 'Double-click to open in Bottle';

/** No link in an answer is trusted until the card knows the profile host (TODO(#40)). */
const NO_LINKS: LinkTrust = { profileHost: '', allowedAppUrls: [] };

type Vis = 'shown' | 'fading' | 'gone';

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
      {shown.leaving !== undefined && (
        <span className="genie-stage" data-leaving aria-hidden="true">
          {shown.leaving}
        </span>
      )}
      <span key={shown.text} className="genie-stage" data-genie-stage aria-live="polite">
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
  return text === undefined ? null : (
    <span className="genie-timer" data-genie-timer aria-hidden="true">
      {text}
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
  onSize?: (px: number) => void;
}

export function GenieCard({ mogId, card, onSize }: GenieCardProps) {
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
  const timer = useRef<{ handle?: ReturnType<typeof setTimeout>; due: number; left: number }>({
    due: 0,
    left: showMs,
  });

  const end = useCallback(
    (fadeMs: number) => {
      clearTimeout(timer.current.handle);
      setVis('fading');
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
      if (working || vis === 'gone' || was === is) return;
      if (is) {
        clearTimeout(timer.current.handle);
        timer.current.left = Math.max(0, timer.current.due - Date.now());
        // A hold during the fade brings the card back.
        setVis('shown');
      } else arm(Math.max(timer.current.left, AFTER_HOVER_MS));
    },
    [working, vis, arm],
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
    if (!working && vis !== 'gone') end(DISMISS_FADE_MS);
  }, [working, vis, end]);
  useWindowEvent(CARD_DISMISS_EVENT, onDismiss);

  // The window fits the final size first, so the grow motion is never cut.
  useLayoutEffect(() => {
    const frame = ref.current?.parentElement;
    if (frame && onSize) onSize(frame.getBoundingClientRect().height);
  }, [onSize, kind, text, card.sections, card.blurbs, retry]);

  // The pill grows into the card: the same element animates from its old size to the new one.
  const prev = useRef<{ kind: string; w: number; h: number } | undefined>(undefined);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const was = prev.current;
    prev.current = { kind, w: r.width, h: r.height };
    if (!was || was.kind === kind || reducedMotion() || typeof el.animate !== 'function') return;
    el.animate(
      [
        { width: `${was.w}px`, height: `${was.h}px` },
        { width: `${r.width}px`, height: `${r.height}px` },
      ],
      { duration: 220, easing: 'cubic-bezier(0.16, 1, 0.3, 1)' },
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

  return (
    <div
      ref={ref}
      className={`genie genie-${kind}`}
      data-genie={kind}
      data-state={vis}
      onMouseEnter={() => hold('pointer', true)}
      onMouseLeave={() => hold('pointer', false)}
      onDoubleClick={onDoubleClick}
    >
      {kind === 'working' && (
        <>
          <Stage text={line1} />
          <Timer />
        </>
      )}
      {kind !== 'working' && (
        <div className="genie-body" key="body">
          {question !== undefined && (
            <p className="genie-question" data-genie-question title={question}>
              {question}
            </p>
          )}
          {kind === 'answer' && answer && (
            <div className="genie-md" data-answer>
              <SafeMarkdown text={answer.md} trust={NO_LINKS} />
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
            <div role="alert">
              <p className="genie-title">{FAILURE_TITLE}</p>
              <p className="genie-reason">{line1}</p>
            </div>
          )}
          {kind === 'message' && (
            <div className="genie-md" data-answer>
              <SafeMarkdown text={[line1, ...rest].join('\n')} trust={NO_LINKS} />
            </div>
          )}
          <footer className="genie-foot">
            {retry ? (
              <button
                type="button"
                className="popup-action"
                data-chip={retry.id}
                onClick={() =>
                  void emitView(mogId, 'chip_click', { chipId: retry.id }).catch(() => undefined)
                }
              >
                {retry.label}
              </button>
            ) : (
              <span>{FOOTER_HINT}</span>
            )}
            <span className="genie-when">now</span>
          </footer>
        </div>
      )}
    </div>
  );
}
