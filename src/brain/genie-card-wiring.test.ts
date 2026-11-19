/** #573: the transient Genie card and the double-click to hub Conversations. */
import { BrainBus } from './bus';
import { wireGenieCard } from './genie-card-wiring';

const A = '00000000-0000-4000-8000-0000000000a1';
const B = '00000000-0000-4000-8000-0000000000b2';
const OP = '00000000-0000-4000-8000-00000000c573';

function setup() {
  const bus = new BrainBus();
  const log: unknown[][] = [];
  const off = wireGenieCard(bus, {
    hideCard: (m) => void log.push(['hide', m]),
    clearAnswer: (m, id) => void log.push(['clear', m, id]),
    sendToCard: (m, e, p) => void log.push(['card', m, e, p]),
    lastOpId: (m) => (m === A ? OP : undefined),
    openConversations: (id) => void log.push(['open', id]),
  });
  const input = (from: string, kind: string, payload?: unknown, mogId = A) =>
    bus.dispatch({ event: 'view:input', payload: { from, input: { mogId, kind, payload } } });
  return { log, input, off };
}

describe('Genie card wiring (#573)', () => {
  it('the pointer on the MOG holds the card of that MOG only', () => {
    const { log, input } = setup();
    input(`mog-${A}`, 'hover', { on: true });
    input(`mog-${A}`, 'hover', { on: false });
    // The old payload-free MOG hover sends nothing to the card.
    input(`mog-${A}`, 'hover');
    expect(log).toEqual([
      ['card', A, 'card:mog_hover', { on: true }],
      ['card', A, 'card:mog_hover', { on: false }],
    ]);
  });

  it('a card that faded by itself hides its window and clears its answer', () => {
    const { log, input } = setup();
    input(`card-${A}`, 'click', { target: 'card_done', requestId: OP });
    input(`card-${A}`, 'click', { target: 'card_done' });
    // Only the card itself can say it is done.
    input(`mog-${A}`, 'click', { target: 'card_done' });
    expect(log).toEqual([
      ['hide', A],
      ['clear', A, OP],
      ['hide', A],
      ['clear', A, undefined],
    ]);
  });

  it('a double-click on the card opens Conversations on its exchange', () => {
    const { log, input } = setup();
    input(`card-${A}`, 'click', { target: 'open_conversations', requestId: 'op-from-card' });
    expect(log).toEqual([['open', 'op-from-card']]);
  });

  it('a double-click on the MOG fades its card and opens Conversations on the newest exchange', () => {
    const { log, input } = setup();
    input(`mog-${A}`, 'click', { target: 'open_conversations' });
    input(`mog-${B}`, 'click', { target: 'open_conversations' }, B);
    expect(log).toEqual([
      ['card', A, 'card:dismiss', {}],
      ['open', OP],
      ['card', B, 'card:dismiss', {}],
      ['open', undefined],
    ]);
  });

  it('drops a bad request ID and input that a window sends for another MOG', () => {
    const { log, input } = setup();
    input(`card-${A}`, 'click', { target: 'open_conversations', requestId: '../x y' });
    input(`card-${B}`, 'click', { target: 'card_done' });
    input('hub', 'click', { target: 'open_conversations' });
    expect(log).toEqual([['open', OP]]);
  });

  it('a plain mascot click is not a double-click', () => {
    const { log, input, off } = setup();
    input(`mog-${A}`, 'click');
    off();
    input(`card-${A}`, 'click', { target: 'card_done' });
    expect(log).toEqual([]);
  });
});
