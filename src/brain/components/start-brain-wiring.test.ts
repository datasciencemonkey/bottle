/** startBrain wires the hub notice (#27), the hub MOG list (#26) and the level stream (#23) once. */
import { wireForget } from '../forget-wiring';
import { wireHubMogs } from '../hub-mogs-wiring';
import { LevelRouter, subscribeLevels, wireLevelStream } from '../level-stream';
import { wireNotice } from '../notice-wiring';
import { ForgetFlow } from './forget-flow';
import { startBrain } from './index';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-core')).eventModule);
vi.mock('../hub-mogs-wiring', () => ({ wireHubMogs: vi.fn() }));
vi.mock('../notice-wiring', () => ({ wireNotice: vi.fn() }));
vi.mock('../forget-wiring', () => ({ wireForget: vi.fn() }));
vi.mock('../level-stream', async (orig) => ({
  ...(await orig<typeof import('../level-stream')>()),
  wireLevelStream: vi.fn(() => () => undefined),
  subscribeLevels: vi.fn(async () => true),
}));

describe('startBrain wiring', () => {
  it('[MB-01] [MB-08a] [CAP-16a] [MEM-32a] wires the hub list, the notice and the level stream, once', () => {
    const a = startBrain();
    startBrain();
    expect(vi.mocked(wireHubMogs).mock.calls).toEqual([[a.bus, a.components.T1]]);
    expect(vi.mocked(wireNotice).mock.calls).toEqual([[a.bus, a.components.T12]]);
    const wired = vi.mocked(wireLevelStream).mock.calls;
    expect(wired).toHaveLength(1);
    expect(wired[0]?.[0]).toBe(a.bus);
    expect(wired[0]?.[1]).toBeInstanceOf(LevelRouter);
    expect(vi.mocked(subscribeLevels).mock.calls).toEqual([[wired[0]?.[1]]]);
    // #123 (MEM-32a, MEM-32d): the hub Forget dialog, once, with a ForgetFlow on T10.
    const forget = vi.mocked(wireForget).mock.calls;
    expect(forget).toHaveLength(1);
    expect(forget[0]?.[0]).toBe(a.bus);
    expect(forget[0]?.[1].flow).toBeInstanceOf(ForgetFlow);
  });
});
