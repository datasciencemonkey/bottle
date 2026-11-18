/**
 * #559: the small pop-up beside the owner MOG after a dictation or an ask. It shows one line,
 * the newest blurb, and one action: Ask Genie, or Copy when the paste was refused (#498).
 *
 * - An ignored pop-up fades after `FADE_AFTER_MS` and then sends `cancel`, so the brain hides
 *   the card. Pointer hover or keyboard focus keeps it open.
 * - A Copy pop-up (MOG-05b, MOG-06d) and a held preview (B1) never fade: the user copies,
 *   dismisses or answers the preview, so no text goes away unseen.
 * - The brain keeps the blurb order, pins and History (UX-11a..UX-12). This surface shows only
 *   the newest blurb; the older ones stay in the brain for the hub History (TODO(#103)).
 */
import { useEffect, useId, useRef, useState } from 'react';
import type { MogId } from '../../shared/interfaces';
import { emitView, type CardUpdate } from '../view-ipc';

/** An ignored pop-up starts to fade after this time. */
export const FADE_AFTER_MS = 5000;
/** The fade itself. The card hides at its end. */
export const FADE_OUT_MS = 300;

/** Chip IDs from the brain: `ask_genie:<utterance>` (ask-gate.ts), `copy-<utterance>` (#498). */
const ASK = 'ask_genie:';
const COPY = 'copy-';

type Chip = { id: string; label: string };
type Phase = 'shown' | 'fading' | 'gone';

/** The chip of `prefix` for the newest blurb, else the newest chip of that kind. */
function pick(chips: Chip[], prefix: string, owner: string | undefined): Chip | undefined {
  const all = chips.filter((c) => c.id.startsWith(prefix));
  return all.find((c) => c.id.slice(prefix.length) === owner) ?? all[all.length - 1];
}

export interface PopupProps {
  mogId: MogId;
  card: CardUpdate;
  /** The user copied or dismissed the pop-up, or it faded. */
  onClose?: () => void;
}

export function Popup({ mogId, card, onClose }: PopupProps) {
  const newest = card.blurbs[0];
  const [phase, setPhase] = useState<Phase>('shown');
  const [held, setHeld] = useState({ pointer: false, focus: false });
  const ref = useRef<HTMLDivElement>(null);
  const whyId = useId();

  const utterance = newest?.id.replace(/^ask:/, '');
  const copy = pick(card.chips, COPY, newest?.id);
  const ask = copy ? undefined : pick(card.chips, ASK, utterance);
  // Any other chip is a held preview's control or a warning (ask-gate.ts): show it as it is.
  const others = card.chips.filter((c) => !c.id.startsWith(ASK) && !c.id.startsWith(COPY));
  const sticky = copy !== undefined || others.length > 0;
  const hovered = held.pointer || held.focus;

  const close = () => {
    setPhase('gone');
    onClose?.();
    void emitView(mogId, 'cancel');
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

  if (!newest || phase === 'gone') return null;

  // `hold` keeps the pop-up open. `report` is the UX-12 blurb hover for the brain: the pointer
  // on the text, or the keyboard focus in the pop-up. A hover on a chip alone sends nothing.
  const hold = (via: 'pointer' | 'focus', on: boolean) => {
    if (held[via] === on) return;
    // A hover or focus during the fade brings the pop-up back.
    if (on && phase === 'fading') setPhase('shown');
    setHeld((h) => ({ ...h, [via]: on }));
  };
  const report = (via: 'pointer' | 'focus', on: boolean) =>
    void emitView(mogId, 'hover', { blurbId: newest.id, on, via });
  const [line, ...rest] = newest.text.split('\n');
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
      role="status"
      data-popup
      data-variant={copy ? 'copy' : 'ask'}
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
        onMouseEnter={() => report('pointer', true)}
        onMouseLeave={() => report('pointer', false)}
      >
        {newest.parts
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
      {copy && chip(copy, close)}
      {ask && chip(ask)}
      {others.map((c) => chip(c))}
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
