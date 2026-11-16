/** V3 card view (thin): blurbs, chips and the expanded answer from `card.update`. */
import { useCallback, useState, type KeyboardEvent } from 'react';
import type { MogId } from '../../shared/interfaces';
import { asCardUpdate, emitView, useViewEvent, type CardUpdate } from '../view-ipc';

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

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Escape') void emitView(mogId, 'cancel');
  };

  // TODO(#85, #86, #89): blurb stack, footer and chip rules. TODO(#210): safe markdown.
  return (
    <section aria-label="MOG card" onKeyDown={onKeyDown}>
      <ul>
        {card?.blurbs.map((b) => (
          <li key={b.id}>{b.text}</li>
        ))}
      </ul>
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
    </section>
  );
}
