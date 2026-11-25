/**
 * The quiet "Copied" / "Couldn't copy" note (the owner's "attach Genie's answer to the
 * clipboard" request). The brain sets `turn.copyNote` once its `clipboard_write_text` call for
 * this turn resolves (`src/brain/components/answer-clipboard.ts`); this note shows for
 * `NOTE_MS`, then fades on its own, no further brain round-trip. Thin view (ARC-2): no answer
 * text reaches this file, only the fixed outcome word.
 */
import { motion, useReducedMotion } from 'motion/react';
import { useEffect, useState } from 'react';
import { Check } from 'lucide-react';

export const COPIED_TEXT = 'Copied';
export const FAILED_TEXT = "Couldn't copy";
/** How long the note shows before it goes away on its own (the same shape as `memory-saved.tsx`). */
export const NOTE_MS = 2_000;

/** A small, muted note next to a finished answer. No banner, no toast stack. */
export function CopyAnswerNote({ outcome }: { outcome: 'copied' | 'failed' | undefined }) {
  if (outcome === undefined) return null;
  // Keyed by the outcome: a fresh mount each time it goes from undefined to a value, so the
  // 2 s timer below always starts at `hidden: false` with no effect-driven setState on mount.
  return <CopyAnswerNoteLine key={outcome} outcome={outcome} />;
}

function CopyAnswerNoteLine({ outcome }: { outcome: 'copied' | 'failed' }) {
  const [hidden, setHidden] = useState(false);
  useEffect(() => {
    const t = setTimeout(() => setHidden(true), NOTE_MS);
    return () => clearTimeout(t);
  }, []);
  const reduce = useReducedMotion();
  if (hidden) return null;
  return (
    <motion.p
      role="status"
      className="agent-copy-note"
      data-agent-copy-note={outcome}
      data-tone={outcome === 'failed' ? 'bad' : 'ok'}
      initial={reduce ? false : { opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18, ease: [0.16, 1, 0.3, 1] }}
    >
      {outcome === 'copied' && (
        <Check aria-hidden="true" className="agent-copy-note-icon" size={12} />
      )}
      <span>{outcome === 'copied' ? COPIED_TEXT : FAILED_TEXT}</span>
    </motion.p>
  );
}
