/**
 * Memory slice 1 (ADR 0113): the "Saved to today's note · Undo" line of the Genie card and the
 * dictation pop-up. The brain sends `memory:saved` (content-free) to the card window after a
 * "Remember this" click or a voice "remember this: …". The line shows for `undoMs` (a saved
 * item) or a short time (a refusal), and Undo sends `memory.undo` for the item.
 * Thin view (ARC-2): no text of the memory reaches this file.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { motion, useReducedMotion } from 'motion/react';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { MemorySaved, MogId } from '../../shared/interfaces';
import { EVENTS } from '../../shared/ipc';
import { emitView } from '../view-ipc';

export const SAVED_LINE = 'Saved to today’s note';
export const SECRET_LINE = 'Not saved: this looks like a key or password.';
export const QUARANTINE_LINE =
  'Saved. Genie and the agent will not see it until you release it in Memory.';
export const FAILED_LINE = 'Could not save this. Try again.';
export const UNDONE_LINE = 'Removed from today’s note.';
export const UNDO_LABEL = 'Undo';
/** How long a refusal, a quarantine note, a failure or "Removed" shows. */
export const NOTE_MS = 6_000;
export const UNDONE_MS = 2_500;

const OUTCOMES: ReadonlySet<string> = new Set(['saved', 'quarantined', 'secret_refused', 'failed']);

/** Shape check of a `memory:saved` payload. */
export function asMemorySaved(p: unknown): MemorySaved | undefined {
  if (typeof p !== 'object' || p === null) return undefined;
  const o = p as Record<string, unknown>;
  if (typeof o['mogId'] !== 'string' || typeof o['outcome'] !== 'string') return undefined;
  if (!OUTCOMES.has(o['outcome'])) return undefined;
  const undoMs = typeof o['undoMs'] === 'number' && o['undoMs'] > 0 ? o['undoMs'] : 0;
  return {
    mogId: o['mogId'],
    outcome: o['outcome'] as MemorySaved['outcome'],
    undoMs,
    ...(typeof o['itemId'] === 'string' ? { itemId: o['itemId'] } : {}),
  };
}

export interface SavedLine {
  outcome: MemorySaved['outcome'] | 'undone';
  /** Set while the Undo control shows. */
  itemId?: string;
}

/**
 * The state of the line for one MOG. `onShow(ms)` runs when a line starts (the card holds its
 * fade for that time). `undo` sends `memory.undo`; `clear` hides the line.
 */
export function useMemorySaved(mogId: MogId, onShow?: (ms: number) => void) {
  const [line, setLine] = useState<SavedLine | undefined>(undefined);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const show = useRef(onShow);
  useEffect(() => {
    show.current = onShow;
  });

  const start = useCallback((next: SavedLine, ms: number) => {
    clearTimeout(timer.current);
    setLine(next);
    show.current?.(ms);
    timer.current = setTimeout(() => setLine(undefined), ms);
  }, []);

  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    const target = getCurrentWindow().label;
    void listen<unknown>(
      EVENTS['memory.saved'].name,
      (e) => {
        const m = asMemorySaved(e.payload);
        if (!m || m.mogId !== mogId) return;
        if (m.outcome === 'saved' && m.itemId !== undefined)
          start({ outcome: 'saved', itemId: m.itemId }, m.undoMs || 8_000);
        else start({ outcome: m.outcome }, NOTE_MS);
      },
      { target },
    ).then((u) => {
      if (gone) u();
      else off = u;
    });
    return () => {
      gone = true;
      off?.();
      clearTimeout(timer.current);
    };
  }, [mogId, start]);

  const lineRef = useRef(line);
  useEffect(() => {
    lineRef.current = line;
  }, [line]);
  const undo = useCallback(() => {
    const itemId = lineRef.current?.itemId;
    if (itemId === undefined) return;
    void emitView(mogId, 'click', { action: 'memory.undo', itemId }).catch(() => undefined);
    start({ outcome: 'undone' }, UNDONE_MS);
  }, [mogId, start]);

  return { line, undo };
}

const TEXT: Readonly<Record<SavedLine['outcome'], string>> = {
  saved: SAVED_LINE,
  quarantined: QUARANTINE_LINE,
  secret_refused: SECRET_LINE,
  failed: FAILED_LINE,
  undone: UNDONE_LINE,
};

/** The line. It is a polite live region, so a screen reader reads each outcome once. */
export function MemorySavedLine({ line, onUndo }: { line: SavedLine; onUndo: () => void }) {
  const reduce = useReducedMotion();
  const bad = line.outcome === 'secret_refused' || line.outcome === 'failed';
  return (
    <motion.div
      className="memory-saved"
      data-memory-saved={line.outcome}
      data-tone={bad ? 'bad' : 'ok'}
      role="status"
      initial={reduce ? false : { opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18, ease: [0.16, 1, 0.3, 1] }}
      onDoubleClick={(e) => e.stopPropagation()}
    >
      <span className="memory-saved-text">{TEXT[line.outcome]}</span>
      {line.outcome === 'saved' && line.itemId !== undefined && (
        <>
          <span aria-hidden="true">·</span>
          <button
            type="button"
            className="memory-saved-undo"
            data-action="memory-undo"
            aria-label="Undo save to today’s note"
            onClick={(e) => {
              e.stopPropagation();
              onUndo();
            }}
          >
            {UNDO_LABEL}
          </button>
        </>
      )}
    </motion.div>
  );
}
