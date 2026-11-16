// @vitest-environment jsdom
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { tauri } from './test-tauri';
import { emitView, mogIdFromLabel, ownMogId, useViewEvent } from './view-ipc';

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
});
