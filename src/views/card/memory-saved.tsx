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
import { MEMORY_UNDO_MS } from '../../shared/hub-memory';
import { EVENTS } from '../../shared/ipc';
import { emitView } from '../view-ipc';

export const SAVED_LINE = 'Saved to today’s note';
/** The one secret sentence, on the card, the pop-up and in the hub. */
export const SECRET_LINE = 'Not saved: this looks like a key or password.';
export const QUARANTINE_LINE =
  'Saved. Genie and the agent will not see it until you release it in Memory.';
export const FAILED_LINE = 'Could not save this. Try again.';
export const UNDONE_LINE = 'Removed from today’s note.';
export const UNDO_LABEL = 'Undo';
/** How long a refusal, a quarantine note, a failure or "Removed" shows. */
export const NOTE_MS = 6_000;
export const UNDONE_MS = 2_500;

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
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
    ...(typeof o['itemId'] === 'string' && UUID.test(o['itemId']) ? { itemId: o['itemId'] } : {}),
  };
}

export interface SavedLine {
  outcome: MemorySaved['outcome'] | 'undone';
  /** Set while the Undo control shows. */
  itemId?: string;
  /** How long the line shows (ms). */
  ms: number;
}

export interface MemorySavedApi {
  line: SavedLine | undefined;
  /** Counts the lines shown, so a view can start over for each one. 0 until the first. */
  seq: number;
  undo: () => void;
}

/**
 * The state of the line for one MOG. `undo` sends `memory.undo`. One listener per card window:
 * CardView owns it, so a voice remember with no pop-up blurb still shows its line.
 */
/** No line, for a view that has no `memory:saved` listener (a test of the card alone). */
export const NO_SAVED: MemorySavedApi = { line: undefined, seq: 0, undo: () => undefined };

export function useMemorySaved(mogId: MogId): MemorySavedApi {
  const [line, setLine] = useState<SavedLine | undefined>(undefined);
  const [seq, setSeq] = useState(0);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const start = useCallback((next: Omit<SavedLine, 'ms'>, ms: number) => {
    clearTimeout(timer.current);
    setLine({ ...next, ms });
    setSeq((n) => n + 1);
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
          start({ outcome: 'saved', itemId: m.itemId }, m.undoMs || MEMORY_UNDO_MS);
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

  return { line, seq, undo };
}

const TEXT: Readonly<Record<SavedLine['outcome'], string>> = {
  saved: SAVED_LINE,
  quarantined: QUARANTINE_LINE,
  secret_refused: SECRET_LINE,
  failed: FAILED_LINE,
  undone: UNDONE_LINE,
};

/**
 * The line inside a live region that stays mounted, so a screen reader reads each new outcome
 * (a status region that appears together with its text is often not announced).
 */
export function MemorySavedLive({
  line,
  onUndo,
}: {
  line: SavedLine | undefined;
  onUndo: () => void;
}) {
  return (
    <div role="status" className="memory-saved-live">
      {line && <MemorySavedLine line={line} onUndo={onUndo} />}
    </div>
  );
}

export function MemorySavedLine({ line, onUndo }: { line: SavedLine; onUndo: () => void }) {
  const reduce = useReducedMotion();
  const bad = line.outcome === 'secret_refused' || line.outcome === 'failed';
  return (
    <motion.div
      className="memory-saved"
      data-memory-saved={line.outcome}
      data-tone={bad ? 'bad' : 'ok'}
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
