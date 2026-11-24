/**
 * V3 card view (thin) from `card.update`. A small card is the #559 pop-up (`Popup.tsx`): one line
 * and one action. A card with an answer, sections, a Page domain or a checklist is the panel.
 */
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { PIN_ENABLED } from '../../shared/feature-flags';
import { isGenieBlurbId } from '../../shared/genie-status';
import type { MogId } from '../../shared/interfaces';
import { DomainPicker } from '../DomainPicker';
import {
  asCardUpdate,
  emitView,
  sendViewReady,
  useCardIdentity,
  useViewEvent,
  type CardSection,
  type CardUpdate,
} from '../view-ipc';
import { applyCardSide, createWindowFit, tauriCardWindow, widthMayChangeIn } from './fit-window';
import { GenieCard } from './GenieCard';
import { Popup } from './Popup';
import { useMemorySaved } from './memory-saved';
import { NO_LINKS, SafeMarkdown } from './safe-markdown';

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

/** A chip is a native button: it acts on a click, Enter or Space and never on hover (UX-20c). */
function Chip({ mogId, chip }: { mogId: MogId; chip: { id: string; label: string } }) {
  return (
    <button
      type="button"
      data-chip={chip.id}
      onClick={() => void emitView(mogId, 'chip_click', { chipId: chip.id })}
    >
      {chip.label}
    </button>
  );
}

/** One expanded section. The brain sends the sections in the UX-EXP order (UX-18). */
function Section({
  mogId,
  s,
  stt,
  overflow,
}: {
  mogId: MogId;
  s: CardSection;
  stt?: string | undefined;
  overflow: { id: string; label: string }[];
}) {
  const click = (payload: object) => void emitView(mogId, 'click', payload);
  switch (s.id) {
    case 'answer':
      return (
        <section data-section="answer" aria-label="Full answer">
          <div data-answer>
            <SafeMarkdown text={s.md} trust={NO_LINKS} />
          </div>
        </section>
      );
    case 'keyNumbers':
      return (
        <section data-section="keyNumbers" aria-label="Key numbers">
          <ul>
            {s.items.map((n, i) => (
              <li key={i}>{n}</li>
            ))}
          </ul>
        </section>
      );
    case 'sql':
      return (
        <section data-section="sql" aria-label="SQL">
          <details>
            <summary>SQL</summary>
            <pre>{s.sql}</pre>
          </details>
        </section>
      );
    case 'tablePreview':
      return (
        <section data-section="tablePreview" aria-label="Table preview">
          <table>
            <thead>
              <tr>
                {s.columns.map((c, i) => (
                  <th key={i}>{c}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {s.rows.map((r, i) => (
                <tr key={i}>
                  {r.map((c, j) => (
                    <td key={j}>{c}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
          {s.moreRows > 0 && <p>{`${s.moreRows} more rows`}</p>}
          {(s.moreColumns ?? 0) > 0 && <p>{`${s.moreColumns} more columns`}</p>}
        </section>
      );
    case 'sentToGenie':
      return (
        <section data-section="sentToGenie" aria-label="Sent to Genie">
          <h3>Sent to Genie</h3>
          <p>{s.text}</p>
        </section>
      );
    case 'resend':
      return (
        <section data-section="resend">
          <button type="button" data-action="resend" onClick={() => click({ target: 'resend' })}>
            Resend without context
          </button>
        </section>
      );
    case 'copy':
      return (
        <section data-section="copy" aria-label="Copy">
          {s.formats.map((f) => (
            <button
              key={f}
              type="button"
              data-copy={f}
              onClick={() => click({ target: 'copy', format: f })}
            >
              {f === 'csv' ? 'Copy as CSV' : 'Copy as Markdown'}
            </button>
          ))}
        </section>
      );
    case 'footer':
      return (
        <footer data-section="footer">
          {(s.sttBadge ?? stt) !== undefined && <span data-badge="stt">{s.sttBadge ?? stt}</span>}
          {s.deepLink !== undefined && <span data-deeplink>{s.deepLink}</span>}
          {overflow.map((c) => (
            <Chip key={c.id} mogId={mogId} chip={c} />
          ))}
        </footer>
      );
    case 'dataFlow':
      return (
        <section data-section="dataFlow" aria-label="Where your data goes">
          <h3>Where your data goes</h3>
          <p>{s.text}</p>
        </section>
      );
  }
}

export function CardView({ mogId }: { mogId: MogId }) {
  const [card, setCard] = useState<CardUpdate | undefined>(undefined);
  // #ui-4: the owner MOG's name and colour (`mog:target`, `card.identity_ready`). The card is
  // already target-only (MOG-13a, MOG-13b): this only labels it.
  const identity = useCardIdentity(mogId);
  // Memory slice 1: one `memory:saved` listener per card window. A voice remember can show the
  // card with no blurb, and a Popup that remounts must not lose the line.
  const saved = useMemorySaved(mogId);
  // #559, MOG-06d: the last Copy pop-up stays until the user copies or dismisses it, even when
  // the brain drops its chip (30 s) or its blurb (UX-12). A newer blurb replaces it.
  const [kept, setKept] = useState<CardUpdate | undefined>(undefined);

  const onUpdate = useCallback(
    (p: unknown) => {
      const u = asCardUpdate(p);
      if (!u || u.mogId !== mogId) return;
      setCard(u);
      const first = u.blurbs[0];
      if (first && u.chips.some((c) => c.id === `copy-${first.id}`)) setKept(u);
      // A newer text replaces the kept Copy pop-up for good.
      else if (first) setKept((k) => (k && k.blurbs[0]?.id !== first.id ? undefined : k));
    },
    [mogId],
  );
  // #507, #498: `ready` after the listener is set, so the brain sends the kept card state again
  // (a new card window misses the updates sent before it listens).
  const onListening = useCallback(() => sendViewReady(mogId), [mogId]);
  useViewEvent('card.update', onUpdate, onListening);

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
  // TODO(#86): blurb lines. TODO(#210 follow-up): the deep link becomes a trusted link.
  const hover = (blurbId: string, via: 'pointer' | 'focus', on: boolean) =>
    void emitView(mogId, 'hover', { blurbId, on, via });
  const historyCount = card?.historyCount ?? 0;
  const probe = import.meta.env.DEV && mogId === DEMO_ID && (
    <textarea data-probe aria-label="Typing probe (dev demo card)" />
  );
  // B2 (review round 1): the brain ages a completed Genie blurb out of `card.blurbs` on its own
  // timer (UX-12), independently of the view's own, usually shorter fade. If that already
  // happened, the sections it left behind are orphaned: never treat them as the panel's answer
  // (no clamp, no close, no fade, no refit on that path), so the card never falls back to a raw,
  // unbounded legacy panel for a Genie answer with no Genie blurb backing it any more.
  const liveGenieBlurbId = card?.blurbs.find((b) => isGenieBlurbId(b.id))?.id;
  const hasAnswer =
    liveGenieBlurbId !== undefined &&
    (card?.sections !== undefined || card?.answerMd !== undefined);
  const panel =
    card !== undefined &&
    (hasAnswer || card.pageDomain !== undefined || card.pagesChecklist !== undefined);
  const newest = card?.blurbs[0];
  const shown = kept && (!newest || newest.id === kept.blurbs[0]?.id) ? kept : card;
  // #573: the window takes the height of the pop-up or the Genie card (`fit-window.ts`).
  const genie = card !== undefined && isGenieBlurbId(newest?.id);
  // UX slice 1: a pending Genie pill stays when a newer pop-up (a dictation) comes. The pill is
  // on top and the pop-up under it, in the one card window. A closed stacked pop-up leaves the pill.
  const [closedPopup, setClosedPopup] = useState<string | undefined>(undefined);
  const pending = useMemo(
    () => card?.blurbs.find((b) => isGenieBlurbId(b.id) && !b.completed),
    [card],
  );
  const pillCard = useMemo(
    () =>
      card && pending && !genie
        ? { ...card, blurbs: [pending, ...card.blurbs.filter((b) => b !== pending)] }
        : card,
    [card, pending, genie],
  );
  // lamp-alive: the card sits close to its MOG on either side; `data-side` on the page mirrors
  // the tail (card.css) toward the MOG.
  const fit = useMemo(() => createWindowFit(tauriCardWindow, { onSide: applyCardSide }), []);
  const lastPx = useRef<number | undefined>(undefined);
  const onSize = useCallback(
    (px: number) => {
      lastPx.current = px;
      return fit(px, window.innerHeight);
    },
    [fit],
  );
  const frame = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    if (genie || panel || !frame.current) return;
    void onSize(frame.current.getBoundingClientRect().height);
  }, [genie, panel, shown, onSize]);
  useEffect(() => {
    // R11 sets a hidden card back to one line when it shows it again: fit once more.
    const onResize = () => {
      if (lastPx.current !== undefined) void fit(lastPx.current, window.innerHeight);
    };
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, [fit]);
  // lamp-alive: the pill changes width after the first fit (the timer comes in after 3 s, the stage
  // line changes): fit the window again, so no clear window lies between the pill and the MOG.
  useEffect(() => {
    if (typeof MutationObserver === 'undefined') return;
    let raf = 0;
    const mo = new MutationObserver((records) => {
      const widthMayChange = widthMayChangeIn(records);
      if (!widthMayChange) return;
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        if (lastPx.current !== undefined) void fit(lastPx.current, window.innerHeight);
      });
    });
    mo.observe(document.getElementById('root') ?? document.body, {
      childList: true,
      subtree: true,
      characterData: true,
      characterDataOldValue: true,
    });
    return () => {
      mo.disconnect();
      cancelAnimationFrame(raf);
    };
  }, [fit]);
  // #573: the echo of an ⌥G question (`ask:<utterance>`, one line, no Ask Genie chip) comes in
  // the same task as the Genie status pill, so it shows nothing: no flash before the pill.
  const echo =
    newest !== undefined &&
    newest.id.startsWith('ask:') &&
    !newest.text.includes('\n') &&
    !card?.chips.some((c) => c.id === `ask_genie:${newest.id.slice(4)}`);
  const stack = !genie && !panel && pending !== undefined && pillCard !== undefined;
  const popup = stack ? shown?.blurbs[0] : undefined;
  const popupOn =
    popup !== undefined &&
    newest !== undefined &&
    popup.id !== closedPopup &&
    !(echo && shown === card);
  // A stacked pop-up that fades or closes leaves the pill alone: fit the window to the new height.
  const stackFrame = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    if (!stack || !stackFrame.current) return;
    void onSize(stackFrame.current.getBoundingClientRect().height);
  }, [stack, popupOn, onSize]);
  if ((genie && newest) || (stack && pending && pillCard))
    return (
      <section ref={stackFrame} aria-label="MOG card" className="card card-genie">
        <GenieCard
          key={(genie ? newest : pending)?.id ?? ''}
          mogId={mogId}
          card={(genie ? card : pillCard) as CardUpdate}
          onSize={onSize}
          identity={identity}
          saved={saved}
        />
        {!genie && popupOn && shown && popup && (
          <Popup
            key={popup.id}
            mogId={mogId}
            card={shown}
            saved={saved}
            stacked
            onClose={() => setClosedPopup(popup.id)}
          />
        )}
        {probe}
      </section>
    );
  if (!panel)
    return (
      <section ref={frame} aria-label="MOG card" className="card card-popup">
        {shown?.blurbs[0] && !(echo && shown === card) && (
          <Popup
            key={shown.blurbs[0].id}
            mogId={mogId}
            card={shown}
            saved={saved}
            onClose={() => setKept(undefined)}
          />
        )}
        {/* A voice remember shows the card with no blurb: the line is all there is. */}
        {!shown?.blurbs[0] && saved.line !== undefined && (
          <Popup
            key={`memory-saved-${saved.seq}`}
            mogId={mogId}
            card={{ mogId, blurbs: [], chips: [] }}
            saved={saved}
            onClose={() => setKept(undefined)}
          />
        )}
        {probe}
      </section>
    );
  return (
    <section aria-label="MOG card" className="card card-panel">
      <ul aria-label="Answers">
        {card?.blurbs
          // [LOW, round 2/3] `hasAnswer` means the `Section` below already shows this exact
          // Genie answer in full, with key numbers: showing it a second time here, as a raw
          // blurb line, is the "answer shows twice" finding. An empty `<li>` kept the B2 hover
          // hold alive but left a bare, empty bullet showing (round 3) — drop the `<li>`
          // entirely instead, and move its hover hold onto the `Section` wrapper below, which
          // shows this same blurb's answer.
          .filter((b) => !(isGenieBlurbId(b.id) && hasAnswer))
          .map((b) => (
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
              ) : isGenieBlurbId(b.id) ? (
                // B1 (review round 1): a Genie blurb's text is its full markdown answer (a
                // completed outcome blurb's text is the raw `end.md`), even when it is not the
                // newest entry (for example a dictation blurb landed after it). Reached only when
                // `hasAnswer` is false (no live `Section` duplicates it) — for example a page
                // domain or checklist panel with no answer section at all.
                <SafeMarkdown text={b.text} trust={NO_LINKS} />
              ) : (
                b.text
              )}
              {/* #574: the host refuses to persist a pin until the pin API ships. */}
              {PIN_ENABLED && (
                <button
                  type="button"
                  data-action="pin"
                  aria-pressed={b.pinned === true}
                  onClick={() => void emitView(mogId, 'click', { target: 'pin', blurbId: b.id })}
                >
                  Pin
                </button>
              )}
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
          <Chip key={c.id} mogId={mogId} chip={c} />
        ))}
      </div>
      {card && (
        <button type="button" data-action="expand" onClick={() => void emitView(mogId, 'expand')}>
          Expand
        </button>
      )}
      {hasAnswer && liveGenieBlurbId !== undefined && (
        // The Genie blurb's own `<li>` is gone from the list above (round 3): its hover hold
        // (B2, review round 1) moves here, onto the answer it actually shows.
        <div
          data-blurb={liveGenieBlurbId}
          onMouseEnter={() => hover(liveGenieBlurbId, 'pointer', true)}
          onMouseLeave={() => hover(liveGenieBlurbId, 'pointer', false)}
          onFocus={() => hover(liveGenieBlurbId, 'focus', true)}
          onBlur={() => hover(liveGenieBlurbId, 'focus', false)}
        >
          {card?.sections?.map((s) => (
            <Section
              key={s.id}
              mogId={mogId}
              s={s}
              stt={card.sttBadge}
              overflow={card.overflowChips ?? []}
            />
          ))}
          {card?.sections === undefined && card?.answerMd !== undefined && (
            <div data-answer>
              <SafeMarkdown text={card.answerMd} trust={NO_LINKS} />
            </div>
          )}
        </div>
      )}
      {card?.pageDomain && <DomainPicker mogId={mogId} picker={card.pageDomain} />}
      {card?.pagesChecklist && (
        <div data-pages-checklist role="alert">
          <p>{card.pagesChecklist.text}</p>
          <ol aria-label="Admin checklist">
            {card.pagesChecklist.items.map((i) => (
              <li key={i}>{i}</li>
            ))}
          </ol>
        </div>
      )}
      {card?.sections === undefined && card?.sttBadge !== undefined && (
        <footer>
          <span data-badge="stt">{card.sttBadge}</span>
        </footer>
      )}
      {/* Dev builds only: a field for the typing and IME check of #78 (tools/mac/card-check.swift). */}
      {probe}
    </section>
  );
}
