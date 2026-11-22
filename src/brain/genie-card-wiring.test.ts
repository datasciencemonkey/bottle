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
    clearAnswer: (m) => void log.push(['clear', m]),
    forget: (m, id) => void log.push(['forget', m, id]),
    sendToCard: (m, e, p) => void log.push(['card', m, e, p]),
    lastOpId: (m) => (m === A ? OP : undefined),
    openConversations: (id) => void log.push(['open', id]),
    closeMog: (m) => void log.push(['close', m]),
    changeHome: (m) => void log.push(['change-home', m]),
    remember: (m, id, t) => void log.push(['remember', m, id, t]),
    undoRemember: (m) => void log.push(['undo', m]),
  });
  const input = (from: string, kind: string, payload?: unknown, mogId = A, actionTicket?: string) =>
    bus.dispatch({
      event: 'view:input',
      payload: { from, input: { mogId, kind, payload }, actionTicket },
    });
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
      ['clear', A],
      ['forget', A, OP],
      ['hide', A],
      ['clear', A],
    ]);
  });

  it('a late card_done of an older request only forgets it: the newer card stays', () => {
    const { log, input } = setup();
    input(`card-${A}`, 'click', { target: 'card_done', requestId: 'op-older' });
    expect(log).toEqual([['forget', A, 'op-older']]);
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

  it('[ui-4] the MOG right-click menu Closes the MOG or opens the hub on Change home', () => {
    const { log, input } = setup();
    input(`mog-${A}`, 'click', { target: 'mog_menu_close' });
    input(`mog-${A}`, 'click', { target: 'mog_menu_change_home' });
    // Only the MOG's own menu can act on it: the card has no such menu.
    input(`card-${A}`, 'click', { target: 'mog_menu_close' });
    expect(log).toEqual([
      ['close', A],
      ['change-home', A],
    ]);
  });

  it('[ADR 0113] a card remember click passes the host ticket and the exchange on', () => {
    const { log, input } = setup();
    input(`card-${A}`, 'click', { action: 'memory.remember', requestId: OP }, A, 'tkt');
    input(`card-${A}`, 'click', { action: 'memory.remember' }, A, 'tkt2');
    // No ticket: the callee gets none, and the brain makes none.
    input(`card-${A}`, 'click', { action: 'memory.remember', requestId: OP });
    expect(log).toEqual([
      ['remember', A, OP, 'tkt'],
      ['remember', A, OP, 'tkt2'],
      ['remember', A, OP, undefined],
    ]);
  });

  it('[ADR 0113] remember and undo come only from the card of the same MOG', () => {
    const { log, input } = setup();
    input(`mog-${A}`, 'click', { action: 'memory.remember' }, A, 'tkt');
    input(`mog-${A}`, 'click', { action: 'memory.undo' });
    input('hub', 'click', { action: 'memory.undo' });
    input(`card-${B}`, 'click', { action: 'memory.remember' }, A, 'tkt');
    input(`card-${A}`, 'click', { action: 'memory.undo' });
    expect(log).toEqual([['undo', A]]);
  });
});
