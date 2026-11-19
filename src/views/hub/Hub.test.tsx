// @vitest-environment jsdom
import { act } from 'react';
import { render, tauri, viewEmits } from '../test-tauri';
import { Hub } from './Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const A = '00000000-0000-4000-8000-00000000000a';
const B = '00000000-0000-4000-8000-00000000000b';
const overlays = { attention: false, recording: false, offline: false, localOnly: false };

describe('V1 hub view', () => {
  beforeEach(() => tauri.reset());

  it('[MOG-05b] [MOG-06d] shows an unshown dictation from the hub Channel with Copy, and sends ready after it subscribes', async () => {
    const { el } = await render(<Hub />);
    const sub = tauri.invoke.mock.calls.find(([cmd]) => cmd === 'view_notice_subscribe');
    const channel = (sub?.[1] as { args: { onNotice: { onmessage?: (m: unknown) => void } } }).args
      .onNotice;
    // The hub sends `ready` (no payload) only after the subscribe call, so the brain resends.
    const order = tauri.invoke.mock.calls.map(([cmd, a]) =>
      cmd === 'view_emit' ? `view_emit:${(a as { args: { kind: string } }).args.kind}` : cmd,
    );
    expect(order.indexOf('view_emit:ready')).toBeGreaterThan(
      order.indexOf('view_notice_subscribe'),
    );
    expect(viewEmits()).toContainEqual({ mogId: 'hub', kind: 'ready' });
    await act(async () =>
      channel.onmessage?.({ kind: 'dictation_unshown', mogId: A, text: 'Send the deck today.' }),
    );
    const notice = el.querySelector('[data-notice="dictation_unshown"]');
    expect(notice?.textContent).toContain('Send the deck today.');
    expect(notice?.querySelector('button[data-action="copy"]')?.textContent).toBe('Copy');
    await act(async () =>
      (notice?.querySelector('button[data-action="dismiss"]') as HTMLElement).click(),
    );
    expect(el.querySelector('[data-notice="dictation_unshown"]')).toBeNull();
  });

  it('[V1] shows the product name and the Labs label', async () => {
    const { el } = await render(<Hub />);
    expect(el.querySelector('h1')?.textContent).toBe('Bottle');
    expect(el.textContent).toContain('Databricks Labs / experimental');
  });

  it('[V1] subscribes to mog:view_state and lists each MOG with its mood', async () => {
    const { el } = await render(<Hub />);
    expect(tauri.listeners('mog:view_state')).toBe(1);
    await tauri.emit('mog:view_state', { mogId: A, mood: 'idle', overlays });
    await tauri.emit('mog:view_state', { mogId: B, mood: 'processing', overlays });
    await tauri.emit('mog:view_state', { mogId: A, mood: 'sleeping', overlays });
    await tauri.emit('mog:view_state', { mogId: 7 });
    const rows = [...el.querySelectorAll('li[data-mog]')].map((li) => [
      li.getAttribute('data-mog'),
      li.getAttribute('data-mood'),
    ]);
    expect(rows).toEqual([
      [A, 'sleeping'],
      [B, 'processing'],
    ]);
  });

  it('[V1] [ARC-2] sends a MOG selection only through view_emit', async () => {
    const { el } = await render(<Hub />);
    await tauri.emit('mog:view_state', { mogId: A, mood: 'idle', overlays });
    await act(async () => (el.querySelector(`li[data-mog="${A}"] button`) as HTMLElement).click());
    // The hub notice (#27, MB-08a) and the MOG list (#26) ask the brain with hub-scope emits
    // (notice.ready, mogs.ready), the CLI step asks with id `cli` (#161), the permission notice
    // with id `permissions` (ADR 0101) and Conversations with id `conversations` (#103); the MOG
    // selection is the rest.
    expect(viewEmits()).toContainEqual({
      mogId: 'hub',
      kind: 'click',
      payload: { action: 'notice.ready' },
    });
    expect(
      viewEmits().filter(
        (e) =>
          !['hub', 'cli', 'permissions', 'conversations'].includes((e as { mogId: string }).mogId),
      ),
    ).toEqual([{ mogId: A, kind: 'click' }]);
  });
});

describe('hub navigation and Conversations (#103)', () => {
  beforeEach(() => tauri.reset());

  const nav = (el: HTMLElement) =>
    [...el.querySelectorAll('nav[aria-label="Hub"] button')] as HTMLElement[];
  const conv = (el: HTMLElement) =>
    el.querySelector('section[aria-label="Conversations"]') as HTMLElement;
  const home = (el: HTMLElement) => el.querySelector('[data-hub-view="home"]') as HTMLElement;

  it('[#103] Conversations is the first navigation item; the hub opens on the MOGs and settings', async () => {
    const { el } = await render(<Hub />);
    expect(nav(el).map((b) => b.textContent)).toEqual(['Conversations', 'MOGs and settings']);
    expect(nav(el)[1]?.getAttribute('aria-current')).toBe('page');
    expect(conv(el).hidden).toBe(true);
    expect(home(el).hidden).toBe(false);
    await act(async () => nav(el)[0]?.click());
    expect(nav(el)[0]?.getAttribute('aria-current')).toBe('page');
    expect(conv(el).hidden).toBe(false);
    expect(home(el).hidden).toBe(true);
    // The product header stays a direct child of main (AT-37).
    expect(el.querySelector('main > h1')?.textContent).toBe('Bottle');
  });

  it('[#103] an open from a double-click shows Conversations', async () => {
    const { el } = await render(<Hub />);
    await tauri.emitTo('hub', 'hub:conversations.open', {});
    expect(conv(el).hidden).toBe(false);
    expect(nav(el)[0]?.getAttribute('aria-current')).toBe('page');
  });
});

describe('V1 hub view: call recording (ADR 0103)', () => {
  beforeEach(() => tauri.reset());

  it('[REC-V15] the hub has a Recordings area and a Call recording settings section', async () => {
    const { el } = await render(<Hub />);
    const nav = [...el.querySelectorAll('nav[aria-label="Hub"] button')].map((b) => b.textContent);
    expect(nav).toContain('Recordings');
    expect(el.querySelector('[data-view="recording-settings"]')).not.toBeNull();
    const rec = [...el.querySelectorAll('nav[aria-label="Hub"] button')].find(
      (b) => b.textContent === 'Recordings',
    );
    await act(async () => (rec as HTMLElement).click());
    expect(el.querySelector('section[aria-label="Recordings"]')?.hasAttribute('hidden')).toBe(false);
    expect(el.querySelector('[data-hub-view="home"]')?.hasAttribute('hidden')).toBe(true);
  });
});
