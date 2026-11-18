/** The Copy card sink of the utterance handler (#498): no dictation text is dropped silently. */
import type { MogId } from '../shared/interfaces';
import { routerDecision } from '../test-support/model-invoke';
import { createCardStack, type CardStackUpdate } from './card-stack';
import { copyReasonText, createDictationSink, type DictationSinkDeps } from './dictation-copy';

const A = '00000000-0000-4000-8000-0000000000a1';
const SENTENCE = 'Hi Sam, the deck is attached and I will send the numbers tomorrow.';
const owned = { utteranceId: 'utt-1', mogId: A };

/**
 * A fake card window. Like the host (MOG-18a), the window exists only after a show. Like the
 * view (`useViewEvent`), it gets a `card.update` only while it listens; it listens after `ready`.
 */
function setup(shows: ('ok' | 'fail')[] = [], hub: boolean[] = []) {
  const log: string[] = [];
  const seen: CardStackUpdate[] = [];
  const sent: CardStackUpdate[] = [];
  const unshown: [MogId, string][] = [];
  let exists = false;
  let listening = false;
  const stacks = new Map<MogId, ReturnType<typeof createCardStack>>();
  const deps: DictationSinkDeps = {
    stackFor: (mogId) => {
      let s = stacks.get(mogId);
      if (!s) {
        s = createCardStack(mogId, {
          clock: { now: () => 0 },
          schedule: () => () => undefined,
          send: (u) => {
            log.push('card.update');
            sent.push(u);
            if (exists && listening) seen.push(u);
          },
          openHistory: () => undefined,
        });
        stacks.set(mogId, s);
      }
      return s;
    },
    showCard: async () => {
      log.push('show');
      if ((shows.shift() ?? 'ok') === 'fail') throw new Error('closing');
      exists = true;
    },
    sendHubNotice: async (mogId, text) => {
      const ok = hub.shift() ?? true;
      if (ok) unshown.push([mogId, text]);
      return ok;
    },
  };
  const sink = createDictationSink(deps);
  const ready = () => {
    listening = true;
    sink.cardReady(A);
  };
  return { sink, log, seen, sent, unshown, ready };
}

const texts = (u: CardStackUpdate | undefined) => u?.blurbs.map((b) => b.text) ?? [];
const labels = (u: CardStackUpdate | undefined) => u?.chips.map((c) => c.label) ?? [];

describe('dictation Copy card sink', () => {
  it('[MOG-05b] [RTR-01c] a copy route shows the card, then puts the text and Copy on it', async () => {
    const { sink, log, sent } = setup();
    await sink.onDone({ route: 'copy', text: SENTENCE }, owned);
    expect(log[0]).toBe('show');
    expect(texts(sent.at(-1))).toEqual([SENTENCE]);
    expect(labels(sent.at(-1))).toEqual(['Copy']);
  });

  it('[MOG-06d] [RTR-01c] each insert_failed reason puts the text, the reason and Copy on the card', async () => {
    for (const reason of ['no_permission', 'no_focus', 'secure_field', 'host_failed'] as const) {
      const { sink, sent } = setup();
      await sink.onDone({ route: 'insert_failed', reason, text: SENTENCE }, owned);
      const [text] = texts(sent.at(-1));
      expect(text?.startsWith(SENTENCE)).toBe(true);
      expect(text).toContain(`Could not insert: ${copyReasonText(reason)}`);
      expect(labels(sent.at(-1))).toEqual(['Copy']);
    }
  });

  it('[MOG-06d] each failure reason has its own text', () => {
    const all = (['no_permission', 'no_focus', 'secure_field', 'host_failed'] as const).map(
      copyReasonText,
    );
    expect(new Set(all).size).toBe(4);
    for (const t of all) expect(t.trim()).not.toBe('');
  });

  it('[RTR-01c] an inserted or other route adds no Copy card', async () => {
    const { sink, log } = setup();
    const decision = routerDecision({ intent: 'noise' });
    await sink.onDone({ route: 'inserted', decision }, owned);
    await sink.onDone({ route: 'other', decision }, owned);
    expect(log).toEqual([]);
  });

  it('[MOG-05b] the first Copy card of a MOG is not empty: the kept state goes again on ready', async () => {
    // Regression (PR #504 review): the new card window listens only after its view mounts, so
    // the updates sent before that are lost. The card `ready` (#507) sends the kept state again.
    const { sink, seen, ready } = setup();
    await sink.onDone({ route: 'copy', text: SENTENCE }, owned);
    expect(seen).toEqual([]);
    ready();
    expect(texts(seen.at(-1))).toEqual([SENTENCE]);
    expect(labels(seen.at(-1))).toEqual(['Copy']);
  });

  it('[MOG-05b] a failed show is tried once more, and the next show sends the kept state', async () => {
    const { sink, log, sent, unshown } = setup(['fail', 'ok']);
    await sink.onDone({ route: 'copy', text: SENTENCE }, owned);
    expect(log.filter((l) => l === 'show')).toEqual(['show', 'show']);
    expect(log.indexOf('card.update')).toBeGreaterThan(log.lastIndexOf('show'));
    expect(texts(sent.at(-1))).toEqual([SENTENCE]);
    expect(unshown).toEqual([]);
  });

  it('[MOG-05b] [MOG-06d] when the retry also fails, the text goes to the hub fallback and stays on the card', async () => {
    const { sink, log, sent, unshown } = setup(['fail', 'fail']);
    await sink.onDone(
      { route: 'insert_failed', reason: 'no_focus', text: SENTENCE },
      { utteranceId: 'utt-2', mogId: A },
    );
    expect(log.filter((l) => l === 'show')).toHaveLength(2);
    expect(unshown).toEqual([[A, SENTENCE]]);
    // The card stack keeps the text, so a later show or `ready` still shows it.
    expect(texts(sent.at(-1))[0]?.startsWith(SENTENCE)).toBe(true);
  });

  it('[MOG-05b] [MOG-06d] a refused hub notice keeps the text and sends it again on hub ready', async () => {
    // No hub listens yet: `hub_notice_send` gives false (ADR 0091). The hub's `ready` resends.
    const { sink, unshown } = setup(['fail', 'fail'], [false]);
    await sink.onDone({ route: 'copy', text: SENTENCE }, owned);
    expect(unshown).toEqual([]);
    await sink.hubReady();
    expect(unshown).toEqual([[A, SENTENCE]]);
    // Idempotent: a second ready does not send a second copy.
    await sink.hubReady();
    expect(unshown).toEqual([[A, SENTENCE]]);
  });

  it('[MOG-05b] ready for a MOG with no dictation sends nothing', () => {
    const { sink, log } = setup();
    sink.cardReady(A);
    expect(log).toEqual([]);
  });
});
