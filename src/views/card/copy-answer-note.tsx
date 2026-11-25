/**
 * The quiet "Copied" / "Couldn't copy" note (the owner's "attach Genie's answer to the
 * clipboard" request). The brain sets `turn.copyNote` once its `clipboard_write_text` call for
 * this turn resolves (`src/brain/components/answer-clipboard.ts`); this note shows for
 * `NOTE_MS`, then fades out on its own, no further brain round-trip. Thin view (ARC-2): no
 * answer text reaches this file, only the fixed outcome word.
 *
 * The outer `role="status"` line stays mounted at all times (an empty live region announces
 * nothing; a region that only appears with its text is often not announced at all) and reserves
 * its own height, so the note's entrance and exit never move the answer above or below it.
 */
import { AnimatePresence, motion, useReducedMotion } from 'motion/react';
import { useEffect, useState } from 'react';
import { Check } from 'lucide-react';

export const COPIED_TEXT = 'Copied';
export const FAILED_TEXT = "Couldn't copy";
/** How long the note shows before it fades out on its own. */
export const NOTE_MS = 2_000;

/** A small, muted note next to a finished answer. No banner, no toast stack. */
export function CopyAnswerNote({ outcome }: { outcome: 'copied' | 'failed' | undefined }) {
  const [expired, setExpired] = useState(false);
  useEffect(() => {
    if (outcome === undefined) return;
    // Only ever set from this deferred callback, never synchronously in the effect body: a
    // brain-sent `copyNote` is set once per turn and never changes again, so `outcome` itself,
    // not a copy of it in state, is what `shown` below is derived from.
    const t = setTimeout(() => setExpired(true), NOTE_MS);
    return () => clearTimeout(t);
  }, [outcome]);
  const reduce = useReducedMotion();
  const shown = outcome !== undefined && !expired ? outcome : undefined;
  return (
    <p role="status" className="agent-copy-note" data-agent-copy-note={shown ?? ''}>
      <AnimatePresence mode="wait">
        {shown && (
          <motion.span
            key={shown}
            className="agent-copy-note-line"
            data-tone={shown === 'failed' ? 'bad' : 'ok'}
            initial={reduce ? false : { opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: reduce ? 0 : 0.18 }}
          >
            {shown === 'copied' && (
              <Check aria-hidden="true" className="agent-copy-note-icon" size={12} />
            )}
            <span>{shown === 'copied' ? COPIED_TEXT : FAILED_TEXT}</span>
          </motion.span>
        )}
      </AnimatePresence>
    </p>
  );
}
