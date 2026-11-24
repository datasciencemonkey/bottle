/**
 * #559: the small pop-up beside the owner MOG after a dictation or an ask. It shows one line,
 * the newest blurb, and one action: Ask Genie, or Copy when the paste was refused (#498). A
 * refused paste shows Copy and Ask Genie of the same text (#557).
 *
 * - ADR 0098: Ask Genie is the fallback for text that was already dictated. Next to it, the
 *   hint `POPUP_ASK_HINT` tells the user that ⌥G during the hold asks Genie directly.
 * - An ignored pop-up fades after `FADE_AFTER_MS` and then sends `cancel`, so the brain hides
 *   the card. Pointer hover or keyboard focus keeps it open.
 * - A Copy pop-up (MOG-05b, MOG-06d) and a warning (RTR-14) never fade: the user copies or
 *   dismisses them, so no text goes away unseen.
 * - The brain keeps the blurb order, pins and History (UX-11a..UX-12). This surface shows only
 *   the newest blurb; the older ones stay in the brain for the hub History (TODO(#103)).
 */
import { useEffect, useId, useRef, useState } from 'react';
import type { MogId } from '../../shared/interfaces';
import { emitView, type CardUpdate } from '../view-ipc';
import { MemorySavedLive, type MemorySavedApi } from './memory-saved';

/** An ignored pop-up starts to fade after this time. */
export const FADE_AFTER_MS = 5000;
/** The fade itself. The card hides at its end. */
export const FADE_OUT_MS = 300;
/** RTR-17a (ADR 0098): the hint beside Ask Genie. The text is fixed by the user decision. */
export const POPUP_ASK_HINT = 'Hold ⌥G to ask Genie';

/**
 * Chip IDs from the brain: `ask_genie:<utterance>` (ask-gate.ts), `copy-<utterance>` (#498),
 * `perm:<permission>` (the OS permission notice, ADR 0101).
 */
const ASK = 'ask_genie:';
const COPY = 'copy-';
const PERM = 'perm:';

type Chip = { id: string; label: string };
type Phase = 'shown' | 'fading' | 'gone';

/** The chip of `prefix` for exactly this utterance. A chip of another text is never shown. */
function pick(chips: Chip[], prefix: string, utterance: string | undefined): Chip | undefined {
  return utterance === undefined ? undefined : chips.find((c) => c.id === prefix + utterance);
}

export interface PopupProps {
  /** Memory slice 1: the `memory:saved` line (`useMemorySaved` in CardView). */
  saved: MemorySavedApi;
  mogId: MogId;
  card: CardUpdate;
  /** The user copied or dismissed the pop-up, or it faded. */
  onClose?: () => void;
  /**
   * UX slice 1: the pop-up shows under a pending Genie pill. Closing it must not hide the card
   * window, because the pill still needs it.
   */
  stacked?: boolean;
}

export function Popup({ mogId, card, onClose, saved, stacked = false }: PopupProps) {
  const newest = card.blurbs[0];
  const [phase, setPhase] = useState<Phase>('shown');
  const [held, setHeld] = useState({ pointer: false, focus: false });
  const ref = useRef<HTMLDivElement>(null);
  const whyId = useId();
  // Memory S2: a voice "remember this: …" shows "Saved to today's note · Undo" for the Undo time.

  // The ask gate's blurb is `ask:<utterance>` (ask-gate.ts); the #498 Copy blurb is `<utterance>`.
  const copy = pick(card.chips, COPY, newest?.id);
  // #557: a refused paste of a question shows Copy and the Ask Genie of the same utterance.
  const ask = pick(card.chips, ASK, copy ? newest?.id : newest?.id.match(/^ask:(.+)$/)?.[1]);
  // Any other chip is a warning (ask-gate.ts, RTR-14): show it as it is.
  const others = card.chips.filter((c) => !c.id.startsWith(ASK) && !c.id.startsWith(COPY));
  const sticky = copy !== undefined || others.length > 0;
  // ADR 0101: a permission notice wraps, so its whole text shows; its one control is a chip.
  const perm = newest?.id.startsWith(PERM) === true && others.some((c) => c.id === newest.id);
  const hovered = held.pointer || held.focus || saved.line !== undefined;

  const close = () => {
    setPhase('gone');
    onClose?.();
    if (!stacked) void emitView(mogId, 'cancel');
  };

  useEffect(() => {
    if (phase === 'gone' || sticky || hovered) return;
    const t =
      phase === 'shown'
        ? setTimeout(() => setPhase('fading'), FADE_AFTER_MS)
        : setTimeout(close, FADE_OUT_MS);
    return () => clearTimeout(t);
    // `close` only reads `mogId` and `onClose`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase, sticky, hovered, mogId]);

  if ((!newest && !saved.line) || phase === 'gone') return null;

  // `hold` keeps the pop-up open. `report` is the UX-12 blurb hover for the brain: the pointer
  // on the text, or the keyboard focus in the pop-up. A hover on a chip alone sends nothing.
  const hold = (via: 'pointer' | 'focus', on: boolean) => {
    if (held[via] === on) return;
    // A hover or focus during the fade brings the pop-up back.
    if (on && phase === 'fading') setPhase('shown');
    setHeld((h) => ({ ...h, [via]: on }));
  };
  const report = (via: 'pointer' | 'focus', on: boolean) => {
    if (newest) void emitView(mogId, 'hover', { blurbId: newest.id, on, via });
  };
  const [line = '', ...rest] = (newest?.text ?? '').split('\n');
  const why = rest.join(' ').trim();
  const chip = (c: Chip, after?: () => void) => (
    <button
      key={c.id}
      type="button"
      className="popup-action"
      data-chip={c.id}
      aria-describedby={why ? whyId : undefined}
      onClick={() => {
        void emitView(mogId, 'chip_click', { chipId: c.id });
        after?.();
      }}
    >
      {c.label}
    </button>
  );

  return (
    <div
      ref={ref}
      className="popup"
      data-popup
      data-variant={copy ? 'copy' : perm ? 'perm' : 'ask'}
      data-state={phase}
      title={why ? `${line}\n${why}` : line}
      onMouseEnter={() => hold('pointer', true)}
      onMouseLeave={() => hold('pointer', false)}
      onFocus={() => {
        if (!held.focus) report('focus', true);
        hold('focus', true);
      }}
      onBlur={(e) => {
        if (ref.current?.contains(e.relatedTarget as Node | null)) return;
        hold('focus', false);
        report('focus', false);
      }}
    >
      <p
        className="popup-text"
        data-popup-text
        aria-live="polite"
        onMouseEnter={() => report('pointer', true)}
        onMouseLeave={() => report('pointer', false)}
      >
        {newest?.parts
          ? newest.parts.map((p) => (
              <span key={p.id} data-part={p.id}>
                {p.text}
              </span>
            ))
          : line}
      </p>
      {why && (
        <span id={whyId} className="popup-sr">
          {why}
        </span>
      )}
      {/* One wrapping row for all controls (1, 2 or 3 chips): it drops under the text and wraps
          by itself, so a chip is never clipped by the window. */}
      <div className="popup-actions" data-popup-actions>
        {copy && chip(copy, close)}
        {ask && chip(ask)}
        {others.map((c) => chip(c))}
        {ask && (
          <span className="popup-hint" data-popup-hint>
            {POPUP_ASK_HINT}
          </span>
        )}
      </div>
      <MemorySavedLive line={saved.line} onUndo={saved.undo} />
      {copy && (
        <button
          type="button"
          className="popup-dismiss"
          data-action="dismiss"
          aria-label="Dismiss"
          onClick={close}
        >
          <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
            <path d="M2 2l6 6M8 2l-6 6" />
          </svg>
        </button>
      )}
    </div>
  );
}
