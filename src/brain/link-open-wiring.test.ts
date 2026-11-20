/**
 * #523: view -> brain -> Rust. A click on a link in the card or in hub Conversations reaches the
 * `link_open` command only if the brain finds the workspace itself and the URL passes the trust
 * rule for it. The Rust opener is mocked (`open`). Fake hosts only.
 */
import { BrainBus } from './bus';
import { wireLinkOpen, type LinkOpenDeps } from './link-open-wiring';

const HOST = 'https://acme-prod.cloud.databricks.test';
const MOG = '00000000-0000-4000-8000-0000000000a1';
const OTHER = '00000000-0000-4000-8000-0000000000b2';
const DEEP = `${HOST}/one/chat/threads/c1?o=1234&utm_source=genie_chat_mcp`;

function setup(over: Partial<LinkOpenDeps> = {}) {
  const bus = new BrainBus();
  const opened: { profile: string; url: string }[] = [];
  const resolved: { from: string; requestId: string | undefined }[] = [];
  const deps: LinkOpenDeps = {
    resolve: async (input, requestId) => {
      resolved.push({ from: input.from, requestId });
      return input.from === 'hub'
        ? requestId === 'op-1'
          ? { profile: 'DEMO', host: HOST }
          : undefined
        : input.mogId === MOG
          ? { profile: 'DEMO', host: HOST }
          : undefined;
    },
    open: async (a) => void opened.push(a),
    ...over,
  };
  const off = wireLinkOpen(bus, deps);
  const send = (from: string, mogId: string, payload: unknown, kind = 'click') =>
    bus.dispatch({ event: 'view:input', payload: { from, input: { mogId, kind, payload } } });
  return { opened, resolved, send, off };
}
const flush = () => new Promise((r) => setTimeout(r, 0));

describe('link open wiring (#523)', () => {
  it('a card click on a trusted URL goes to link_open with the MOG profile', async () => {
    const t = setup();
    t.send(`card-${MOG}`, MOG, { target: 'link_open', url: DEEP });
    await flush();
    expect(t.opened).toEqual([{ profile: 'DEMO', url: DEEP }]);
  });

  it('a hub click resolves the workspace from the exchange the brain lists, not from the click', async () => {
    const t = setup();
    t.send('hub', 'conversations', { target: 'link_open', url: DEEP, requestId: 'op-1' });
    await flush();
    expect(t.opened).toEqual([{ profile: 'DEMO', url: DEEP }]);
    expect(t.resolved).toEqual([{ from: 'hub', requestId: 'op-1' }]);
  });

  it('opens a normalised URL', async () => {
    const t = setup();
    t.send('hub', 'conversations', {
      target: 'link_open',
      url: `HTTPS://ACME-PROD.cloud.databricks.test/a`,
      requestId: 'op-1',
    });
    await flush();
    expect(t.opened.map((o) => o.url)).toEqual([`${HOST}/a`]);
  });

  it.each([
    'https://evil.test/x',
    'https://acme-prod.cloud.databricks.test.evil.test/x',
    'https://evil.test/@acme-prod.cloud.databricks.test',
    'https://acme-prod.cloud.databricks.test@evil.test/x',
    'http://acme-prod.cloud.databricks.test/x',
    'javascript:alert(1)',
    'file:///etc/passwd',
    '//acme-prod.cloud.databricks.test/x',
  ])('a forged click with %s never reaches Rust', async (url) => {
    const t = setup();
    t.send(`card-${MOG}`, MOG, { target: 'link_open', url });
    t.send('hub', 'conversations', { target: 'link_open', url, requestId: 'op-1' });
    await flush();
    expect(t.opened).toEqual([]);
  });

  it('an unknown exchange or an unknown MOG has no workspace, so nothing opens', async () => {
    const t = setup();
    t.send('hub', 'conversations', { target: 'link_open', url: DEEP, requestId: 'op-9' });
    t.send('hub', 'conversations', { target: 'link_open', url: DEEP });
    t.send(`card-${OTHER}`, OTHER, { target: 'link_open', url: DEEP });
    await flush();
    expect(t.opened).toEqual([]);
  });

  it('a window cannot speak for another MOG', async () => {
    const t = setup();
    t.send(`card-${OTHER}`, MOG, { target: 'link_open', url: DEEP });
    t.send(`mog-${OTHER}`, MOG, { target: 'link_open', url: DEEP });
    await flush();
    expect(t.opened).toEqual([]);
  });

  it('ignores other clicks, other kinds and a malformed payload', async () => {
    const t = setup();
    t.send(`card-${MOG}`, MOG, { target: 'open_conversations' });
    t.send(`card-${MOG}`, MOG, { target: 'link_open' });
    t.send(`card-${MOG}`, MOG, { target: 'link_open', url: 5 });
    t.send(`card-${MOG}`, MOG, 'link_open');
    t.send(`card-${MOG}`, MOG, { target: 'link_open', url: DEEP }, 'hover');
    await flush();
    expect(t.opened).toEqual([]);
    expect(t.resolved).toEqual([]);
  });

  it('a failing resolve or a refusing Rust never throws and opens nothing', async () => {
    const a = setup({ resolve: async () => Promise.reject(new Error('profiles')) });
    a.send(`card-${MOG}`, MOG, { target: 'link_open', url: DEEP });
    const b = setup({ open: async () => Promise.reject({ error: 'host_failed' }) });
    b.send(`card-${MOG}`, MOG, { target: 'link_open', url: DEEP });
    await flush();
    expect(a.opened).toEqual([]);
    expect(b.opened).toEqual([]);
  });

  it('stops after off()', async () => {
    const t = setup();
    t.off();
    t.send(`card-${MOG}`, MOG, { target: 'link_open', url: DEEP });
    await flush();
    expect(t.opened).toEqual([]);
  });
});
