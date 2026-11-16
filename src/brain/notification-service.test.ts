import type { MogId } from '../shared/interfaces';
import { createSimClock } from './clock';
import type { Scheduler } from './card-stack';
import {
  createNotificationService,
  type OsNotification,
  type NotificationHost,
} from './notification-service';

const S = 1000;
const A = '00000000-0000-4000-8000-00000000000a' as MogId;
const B = '00000000-0000-4000-8000-00000000000b' as MogId;

function setup(opts: { visible?: Set<string>; focusAllows?: boolean; failHost?: boolean } = {}) {
  const clock = createSimClock();
  let timers: { at: number; fn: () => void }[] = [];
  const schedule: Scheduler = (at, fn) => {
    const t = { at, fn };
    timers.push(t);
    return () => {
      timers = timers.filter((x) => x !== t);
    };
  };
  const sent: OsNotification[] = [];
  const host: NotificationHost = {
    cardVisible: async (id) => {
      if (opts.failHost) throw new Error('x');
      return opts.visible?.has(id) ?? false;
    },
    focusAllows: async () => opts.focusAllows ?? true,
    notify: async (n) => void sent.push(n),
  };
  const svc = createNotificationService({ clock, schedule, host });
  /** Move the clock, fire due timers, and let the async host calls finish. */
  async function advance(ms: number) {
    clock.advance(ms);
    const due = timers.filter((t) => t.at <= clock.now());
    timers = timers.filter((t) => t.at > clock.now());
    due.forEach((t) => t.fn());
    await new Promise((r) => setTimeout(r, 0));
  }
  return { svc, sent, advance, pending: () => timers.length };
}

const answer = (mogId: MogId, text = 'Revenue is 4.2 M') => ({
  kind: 'answer' as const,
  mogId,
  text,
});

describe('OS notifications', () => {
  it('[UX-26a] a finished answer with the card hidden sends one notification', async () => {
    const t = setup();
    t.svc.report(answer(A));
    await t.advance(5 * S);
    expect(t.sent).toHaveLength(1);
    expect(t.sent[0]).toMatchObject({ mogIds: [A], body: 'Revenue is 4.2 M' });
  });

  it('[UX-26a][AT-09] a chip never sends a notification', async () => {
    const t = setup();
    t.svc.report({ kind: 'chip', mogId: A, text: 'Show chart' });
    expect(t.pending()).toBe(0);
    await t.advance(60 * S);
    expect(t.sent).toEqual([]);
  });

  it('[UX-26a] no notification when the card of the MOG is visible', async () => {
    const t = setup({ visible: new Set([A]) });
    t.svc.report(answer(A));
    await t.advance(5 * S);
    expect(t.sent).toEqual([]);
  });

  it('[UX-26a] the card state is read when the notification is due, so a card that opens meanwhile cancels it', async () => {
    const vis = new Set<string>();
    const t = setup({ visible: vis });
    t.svc.report(answer(A));
    vis.add(A);
    await t.advance(5 * S);
    expect(t.sent).toEqual([]);
  });

  it('[UX-26a] no notification when the OS Focus mode blocks it', async () => {
    const t = setup({ focusAllows: false });
    t.svc.report(answer(A));
    await t.advance(5 * S);
    expect(t.sent).toEqual([]);
  });

  it('[UX-26a] an unreadable card state sends nothing', async () => {
    const t = setup({ failHost: true });
    t.svc.report(answer(A));
    await t.advance(5 * S);
    expect(t.sent).toEqual([]);
  });

  it('[UX-26b][AT-09] a burst of 3 answers of one MOG gives one notification', async () => {
    const t = setup();
    t.svc.report(answer(A, 'one'));
    await t.advance(1 * S);
    t.svc.report(answer(A, 'two'));
    await t.advance(1 * S);
    t.svc.report(answer(A, 'three'));
    await t.advance(10 * S);
    expect(t.sent).toHaveLength(1);
  });

  it('[UX-26b] an answer of the same MOG inside 10 s of a notification sends none', async () => {
    const t = setup();
    t.svc.report(answer(A));
    await t.advance(5 * S); // sent at 5 s
    await t.advance(4 * S); // 9 s
    t.svc.report(answer(A));
    await t.advance(10 * S);
    expect(t.sent).toHaveLength(1);
  });

  it('[UX-26b] the MOG may send again 10 s after its last notification', async () => {
    const t = setup();
    t.svc.report(answer(A));
    await t.advance(5 * S); // sent at 5 s
    await t.advance(10 * S); // 15 s
    t.svc.report(answer(A));
    await t.advance(5 * S);
    expect(t.sent).toHaveLength(2);
  });

  it('[UX-26b] the limit is for each MOG: another MOG is not limited by the first', async () => {
    const t = setup();
    t.svc.report(answer(A));
    await t.advance(5 * S);
    t.svc.report(answer(B));
    await t.advance(5 * S);
    expect(t.sent.map((n) => n.mogIds)).toEqual([[A], [B]]);
  });

  it('[UX-26c][AT-09] two MOGs that finish together give one merged notification', async () => {
    const t = setup();
    t.svc.report(answer(A));
    await t.advance(2 * S);
    t.svc.report(answer(B));
    await t.advance(3 * S);
    expect(t.sent).toHaveLength(1);
    expect(t.sent[0]?.mogIds).toEqual([A, B]);
    expect(t.sent[0]?.body).toBe('2 MOGs have new answers');
  });

  it('[UX-26c] answers more than 5 s apart give separate notifications', async () => {
    const t = setup();
    t.svc.report(answer(A));
    await t.advance(5 * S);
    t.svc.report(answer(B));
    await t.advance(5 * S);
    expect(t.sent).toHaveLength(2);
  });

  it('[UX-26a][UX-26c] a merged notification names only the MOGs with a hidden card', async () => {
    const t = setup({ visible: new Set([B]) });
    t.svc.report(answer(A));
    t.svc.report(answer(B));
    await t.advance(5 * S);
    expect(t.sent).toHaveLength(1);
    expect(t.sent[0]?.mogIds).toEqual([A]);
  });

  it('[UX-26a] dispose cancels a pending notification', async () => {
    const t = setup();
    t.svc.report(answer(A));
    t.svc.dispose();
    expect(t.pending()).toBe(0);
    await t.advance(5 * S);
    expect(t.sent).toEqual([]);
  });
});
