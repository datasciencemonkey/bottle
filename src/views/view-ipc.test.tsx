// @vitest-environment jsdom
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { tauri } from './test-tauri';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  asCardUpdate,
  asMicLevelFrame,
  asMogViewState,
  emitView,
  mogIdFromLabel,
  ownMogId,
  useViewEvent,
  viewMicLevels,
} from './view-ipc';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('./test-tauri')).windowModule);

const ID = '00000000-0000-4000-8000-000000000001';

describe('view IPC helpers', () => {
  beforeEach(() => tauri.reset());

  it('[V2] reads the MOG ID from a mog-<id> window label', () => {
    expect(mogIdFromLabel(`mog-${ID}`, 'mog')).toBe(ID);
    expect(mogIdFromLabel(`card-${ID}`, 'mog')).toBeUndefined();
    expect(mogIdFromLabel('mog-', 'mog')).toBeUndefined();
  });

  it('[V3] reads the MOG ID from a card-<id> window label', () => {
    expect(mogIdFromLabel(`card-${ID}`, 'card')).toBe(ID);
    expect(mogIdFromLabel('hub', 'card')).toBeUndefined();
  });

  it('[V2] [V3] ownMogId uses the current window label', () => {
    tauri.label = `card-${ID}`;
    expect(ownMogId('card')).toBe(ID);
    expect(ownMogId('mog')).toBeUndefined();
  });

  it('[V1] [ARC-2] emitView sends only view_emit with the args envelope', async () => {
    await emitView(ID, 'click');
    await emitView(ID, 'chip_click', { chipId: 'c1' });
    expect(tauri.invoke.mock.calls).toEqual([
      ['view_emit', { args: { mogId: ID, kind: 'click' } }],
      ['view_emit', { args: { mogId: ID, kind: 'chip_click', payload: { chipId: 'c1' } } }],
    ]);
  });

  it('[MOG-08a] [ARC-5] a view listens only to events sent to its own window label', async () => {
    tauri.label = `mog-${ID}`;
    const onPayload = () => undefined;
    function Probe() {
      useViewEvent('mog.view_state', onPayload);
      return null;
    }
    const root = createRoot(document.createElement('div'));
    await act(async () => root.render(<Probe />));
    expect(tauri.listenOptions.get('mog:view_state')).toEqual([{ target: `mog-${ID}` }]);
    await act(async () => root.unmount());
  });

  async function probe(label: string, key: 'mog.view_state' | 'card.update') {
    tauri.label = label;
    const got: unknown[] = [];
    const onPayload = (p: unknown) => got.push(p);
    function Probe() {
      useViewEvent(key, onPayload);
      return null;
    }
    const root = createRoot(document.createElement('div'));
    await act(async () => root.render(<Probe />));
    return { got, root };
  }

  it('[MOG-08a] [ARC-5] a MOG view does not get the events sent to another MOG window', async () => {
    const OTHER = '00000000-0000-4000-8000-000000000002';
    const { got, root } = await probe(`mog-${ID}`, 'mog.view_state');
    await tauri.emitTo(`mog-${OTHER}`, 'mog:view_state', { mogId: OTHER });
    await tauri.emitTo('hub', 'mog:view_state', { mogId: OTHER });
    await tauri.emitTo(`mog-${ID}`, 'mog:view_state', { mogId: ID });
    expect(got).toEqual([{ mogId: ID }]);
    await act(async () => root.unmount());
  });

  it('[MOG-08a] [ARC-5] the hub gets the events sent to the hub only', async () => {
    const { got, root } = await probe('hub', 'mog.view_state');
    await tauri.emitTo(`mog-${ID}`, 'mog:view_state', { mogId: ID, n: 1 });
    await tauri.emitTo('hub', 'mog:view_state', { mogId: ID, n: 2 });
    expect(got).toEqual([{ mogId: ID, n: 2 }]);
    await act(async () => root.unmount());
  });
});

const fixture = (name: string): unknown =>
  JSON.parse(
    readFileSync(
      join(import.meta.dirname, '../../tests/fixtures/ipc-types', `${name}.json`),
      'utf8',
    ),
  );

describe('contract view payloads (#285, ADR 0040)', () => {
  beforeEach(() => tauri.reset());

  it('[ARC-5] the view shape checks accept the contract fixtures', () => {
    for (const name of ['MogViewState', 'MogViewState.minimal']) {
      expect(asMogViewState(fixture(name))).toEqual(fixture(name));
    }
    for (const name of ['CardUpdate', 'CardUpdate.minimal']) {
      expect(asCardUpdate(fixture(name))).toEqual(fixture(name));
    }
    for (const name of ['MicLevelFrame', 'MicLevelFrame.closed']) {
      expect(asMicLevelFrame(fixture(name))).toEqual(fixture(name));
    }
  });

  it('[ARC-5] [UX-06a] asMicLevelFrame drops a frame the meter cannot read', () => {
    for (const bad of [
      null,
      { dbfs: -1, t: 1 },
      { owner: 7, dbfs: -1, t: 1 },
      { owner: ID, dbfs: '-1', t: 1 },
      { owner: ID, dbfs: -1 },
    ]) {
      expect(asMicLevelFrame(bad)).toBeUndefined();
    }
  });

  it('[UX-06a] [UX-06b] [ARC-5] the MOG view gets mog.level frames sent to its own window only', async () => {
    tauri.label = `mog-${ID}`;
    const got: unknown[] = [];
    const off = viewMicLevels().subscribe((f) => got.push(f));
    await act(async () => undefined);
    expect(tauri.listenOptions.get('mog:level')).toEqual([{ target: `mog-${ID}` }]);
    const OTHER = '00000000-0000-4000-8000-000000000002';
    await tauri.emitTo(`mog-${OTHER}`, 'mog:level', { owner: OTHER, dbfs: -10, t: 1 });
    await tauri.emitTo(`mog-${ID}`, 'mog:level', { owner: ID, dbfs: -20, t: 2 });
    await tauri.emitTo(`mog-${ID}`, 'mog:level', { owner: ID, dbfs: 'loud', t: 3 });
    await tauri.emitTo(`mog-${ID}`, 'mog:level', { owner: null, dbfs: -120, t: 4 });
    expect(got).toEqual([
      { owner: ID, dbfs: -20, t: 2 },
      { owner: null, dbfs: -120, t: 4 },
    ]);
    off();
    await act(async () => undefined);
    expect(tauri.listeners('mog:level')).toBe(0);
  });

  it('[UX-06a] the MOG window entry feeds the level meter from mog.level', () => {
    const main = readFileSync(join(import.meta.dirname, 'mog', 'main.tsx'), 'utf8');
    expect(main).toMatch(/levels=\{viewMicLevels\(\)\}/);
  });
});
