/** V3 card view (thin): blurbs, chips and the expanded answer from `card.update`. */
import { useCallback, useEffect, useState } from 'react';
import type { MogId } from '../../shared/interfaces';
import { asCardUpdate, emitView, useViewEvent, type CardUpdate } from '../view-ipc';

/** The MOG ID of the debug demo card (`DEMO_ID` in `src-tauri/src/lib.rs`). */
const DEMO_ID = 'demo';

/**
 * UX-19b: one Esc press sends one `cancel`; the brain then cancels the request or hides the card
 * (`src/brain/card-esc.ts`). A key repeat sends nothing more. An Esc while the IME composes
 * (MOG-03a) only ends the composition, so it never closes the card under the user's text.
 */
function isCardEsc(e: KeyboardEvent): boolean {
  return e.key === 'Escape' && !e.repeat && !e.isComposing && e.keyCode !== 229;
}

export function CardView({ mogId }: { mogId: MogId }) {
  const [card, setCard] = useState<CardUpdate | undefined>(undefined);

  const onUpdate = useCallback(
    (p: unknown) => {
      const u = asCardUpdate(p);
      if (u && u.mogId === mogId) setCard(u);
    },
    [mogId],
  );
  useViewEvent('card.update', onUpdate);

  // The card is the only key window (UX-19a), so every key of the card reaches this window.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!isCardEsc(e)) return;
      e.preventDefault();
      void emitView(mogId, 'cancel');
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [mogId]);

  // The brain orders, limits, pins and collapses the blurbs (UX-11a..UX-12, ADR 0028); the view
  // renders them in the order it gets and reports pointer hover and keyboard focus.
  // TODO(#86, #89): blurb lines, footer and chip rules. TODO(#210): safe markdown (plain text now).
  const hover = (blurbId: string, via: 'pointer' | 'focus', on: boolean) =>
    void emitView(mogId, 'hover', { blurbId, on, via });
  const historyCount = card?.historyCount ?? 0;
  return (
    <section aria-label="MOG card">
      <ul aria-label="Answers">
        {card?.blurbs.map((b) => (
          <li
            key={b.id}
            data-blurb={b.id}
            onMouseEnter={() => hover(b.id, 'pointer', true)}
            onMouseLeave={() => hover(b.id, 'pointer', false)}
            onFocus={() => hover(b.id, 'focus', true)}
            onBlur={() => hover(b.id, 'focus', false)}
          >
            {b.parts ? (
              <ul aria-label="Answer parts">
                {b.parts.map((p) => (
                  <li key={p.id} data-part={p.id}>
                    {p.text}
                  </li>
                ))}
              </ul>
            ) : (
              b.text
            )}
            <button
              type="button"
              data-action="pin"
              aria-pressed={b.pinned === true}
              onClick={() => void emitView(mogId, 'click', { target: 'pin', blurbId: b.id })}
            >
              Pin
            </button>
          </li>
        ))}
      </ul>
      {historyCount > 0 && (
        <button
          type="button"
          data-action="history"
          onClick={() => void emitView(mogId, 'click', { target: 'history' })}
        >
          {`${historyCount} more in History`}
        </button>
      )}
      <div>
        {card?.chips.map((c) => (
          <button
            key={c.id}
            type="button"
            data-chip={c.id}
            onClick={() => void emitView(mogId, 'chip_click', { chipId: c.id })}
          >
            {c.label}
          </button>
        ))}
      </div>
      {card && (
        <button type="button" data-action="expand" onClick={() => void emitView(mogId, 'expand')}>
          Expand
        </button>
      )}
      {card?.answerMd !== undefined && <div data-answer>{card.answerMd}</div>}
      {card?.sttBadge !== undefined && (
        <footer>
          <span data-badge="stt">{card.sttBadge}</span>
        </footer>
      )}
      {/* Dev builds only: a field for the typing and IME check of #78 (tools/mac/card-check.swift). */}
      {import.meta.env.DEV && mogId === DEMO_ID && (
        <textarea data-probe aria-label="Typing probe (dev demo card)" />
      )}
    </section>
  );
}
