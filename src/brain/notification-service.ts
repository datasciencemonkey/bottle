/**
 * OS notifications (UX-26a, UX-26b, UX-26c, ADR 0032). The brain decides; the host only shows.
 *
 * - UX-26a: only an answer that finishes with the card hidden and the OS Focus mode open sends
 *   a notification. A chip never does.
 * - UX-26b: 1 notification or less for each MOG in each 10 s.
 * - UX-26c: reports of different MOGs inside one 5 s window leave as one merged notification.
 *
 * The first report opens the 5 s window. The window closes with one check of the card state of
 * each MOG and of the Focus mode, so a card that opens meanwhile cancels its notification.
 *
 * TODO(#95): the host calls (`cardVisible`, `focusAllows`, `notify`) have no IPC command yet.
 * The wiring in `main.ts` waits for a contract change (see the PR).
 */
import type { MogId } from '../shared/interfaces';
import type { Scheduler } from './card-stack';
import type { MonotonicClock } from './clock';

export const MOG_RATE_LIMIT_MS = 10_000;
export const MERGE_WINDOW_MS = 5_000;

export interface NotificationReport {
  /** Only an `answer` can notify. A `chip` never does (UX-26a). */
  kind: 'answer' | 'chip';
  mogId: MogId;
  /** The one-line text of the answer, shown when this MOG is the only one in the window. */
  text: string;
}

export interface OsNotification {
  title: string;
  body: string;
  mogIds: MogId[];
}

/** What the brain needs from the host. A rejected call means "unknown", and nothing is sent. */
export interface NotificationHost {
  cardVisible(mogId: MogId): Promise<boolean>;
  /** False when the OS Focus mode (or Do Not Disturb) blocks notifications. */
  focusAllows(): Promise<boolean>;
  notify(n: OsNotification): Promise<void>;
}

export interface NotificationServiceDeps {
  clock: MonotonicClock;
  schedule: Scheduler;
  host: NotificationHost;
}

export interface NotificationService {
  report(r: NotificationReport): void;
  dispose(): void;
}

export function createNotificationService(deps: NotificationServiceDeps): NotificationService {
  const { clock, schedule, host } = deps;
  const lastSentAt = new Map<MogId, number>();
  /** The reports of the open window, one for each MOG, in arrival order. */
  let pending = new Map<MogId, string>();
  let cancelTimer: (() => void) | undefined;

  const limited = (id: MogId): boolean => {
    const at = lastSentAt.get(id);
    return at !== undefined && clock.now() - at < MOG_RATE_LIMIT_MS;
  };

  async function flush(batch: Map<MogId, string>): Promise<void> {
    try {
      if (!(await host.focusAllows())) return;
      const hidden: MogId[] = [];
      for (const id of batch.keys())
        if (!(await host.cardVisible(id)) && !limited(id)) hidden.push(id);
      if (hidden.length === 0) return;
      const first = hidden[0] as MogId;
      await host.notify(
        hidden.length === 1
          ? { title: 'MOG', body: batch.get(first) ?? '', mogIds: hidden }
          : { title: 'MOG', body: `${hidden.length} MOGs have new answers`, mogIds: hidden },
      );
      const now = clock.now();
      for (const id of hidden) lastSentAt.set(id, now);
    } catch {
      // The host state is unknown or the call failed: send nothing.
    }
  }

  return {
    report(r) {
      if (r.kind !== 'answer' || pending.has(r.mogId) || limited(r.mogId)) return;
      pending.set(r.mogId, r.text);
      if (cancelTimer) return;
      cancelTimer = schedule(clock.now() + MERGE_WINDOW_MS, () => {
        const batch = pending;
        pending = new Map();
        cancelTimer = undefined;
        void flush(batch);
      });
    },
    dispose() {
      cancelTimer?.();
      cancelTimer = undefined;
      pending = new Map();
    },
  };
}
