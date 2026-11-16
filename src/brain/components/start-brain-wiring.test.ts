/** startBrain wires the hub notice (#27) and the hub MOG list (#26) on the brain bus once. */
import { wireHubMogs } from '../hub-mogs-wiring';
import { wireNotice } from '../notice-wiring';
import { startBrain } from './index';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-core')).eventModule);
vi.mock('../hub-mogs-wiring', () => ({ wireHubMogs: vi.fn() }));
vi.mock('../notice-wiring', () => ({ wireNotice: vi.fn() }));

describe('startBrain wiring', () => {
  it('[MB-01] [MB-08a] wires the hub MOG list to T1 and the hub notice to T12, once', () => {
    const a = startBrain();
    startBrain();
    expect(vi.mocked(wireHubMogs).mock.calls).toEqual([[a.bus, a.components.T1]]);
    expect(vi.mocked(wireNotice).mock.calls).toEqual([[a.bus, a.components.T12]]);
  });
});
