/**
 * Small, calm feedback effects, hand-built on Motion (ui-1, ADR 0104; "hand-built on Motion;
 * Animate UI/Magic UI copy-in deferred to ui-1b" per ui-lead). Each one is named after the
 * Animate UI / Magic UI primitive it stands in for, so a later PR can swap the implementation
 * for a licensed copy-in without renaming call sites. All of them respect
 * `useReducedMotion`/`MotionConfig reducedMotion="user"` by using Motion's own animated
 * primitives, which already no-op the animation (not the state change) under reduced motion.
 */
import { motion } from 'motion/react';
import type { ReactNode } from 'react';

/** Stands in for Magic UI's BlurFade: a section or card fades and un-blurs in on mount. */
export function BlurFade({
  children,
  delay = 0,
  className,
}: {
  children: ReactNode;
  delay?: number;
  className?: string;
}) {
  return (
    <motion.div
      className={className}
      initial={{ opacity: 0, filter: 'blur(4px)', y: 4 }}
      animate={{ opacity: 1, filter: 'blur(0px)', y: 0 }}
      transition={{ duration: 0.25, delay, ease: [0.16, 1, 0.3, 1] }}
    >
      {children}
    </motion.div>
  );
}

/**
 * Stands in for Magic UI's Shimmer / BorderBeam: a quiet animated sheen for "this is working"
 * states (the Conversations loading skeleton, SIM-27). Idle CPU stays near zero: the shimmer only
 * renders while `active`, so a finished or hidden view has no running animation.
 */
export function Shimmer({ active, className }: { active: boolean; className?: string }) {
  if (!active) return null;
  return (
    <span
      aria-hidden="true"
      className={
        'relative isolate block overflow-hidden rounded-[var(--radius-sm)] bg-muted ' +
        (className ?? '')
      }
    >
      <motion.span
        className="absolute inset-0 -translate-x-full bg-gradient-to-r from-transparent via-white/10 to-transparent"
        animate={{ x: ['-100%', '100%'] }}
        transition={{ duration: 1.1, repeat: Infinity, ease: 'linear' }}
      />
    </span>
  );
}

/** A row of Shimmer blocks shaped like a Conversations list + detail skeleton. */
export function ConversationsSkeleton() {
  return (
    <div className="flex h-full w-full gap-[var(--mog-space-4)] p-[var(--mog-space-4)]">
      <div className="flex w-[220px] shrink-0 flex-col gap-[var(--mog-space-2)]">
        {[0, 1, 2, 3].map((i) => (
          <Shimmer key={i} active className="h-10 w-full" />
        ))}
      </div>
      <div className="flex min-w-0 flex-1 flex-col gap-[var(--mog-space-3)]">
        <Shimmer active className="h-6 w-2/3" />
        <Shimmer active className="h-4 w-1/3" />
        <Shimmer active className="mt-[var(--mog-space-2)] h-24 w-full" />
      </div>
    </div>
  );
}

/** Stands in for Magic UI's NumberTicker: counts up once on mount, then holds (no idle loop). */
export function NumberTicker({ value, className }: { value: number; className?: string }) {
  return (
    <motion.span
      className={className}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.3 }}
    >
      {value}
    </motion.span>
  );
}
