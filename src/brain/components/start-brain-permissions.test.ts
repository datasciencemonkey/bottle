/** ADR 0101 (#552): startBrain reads the OS permissions at launch with no prompt. */
import { core } from '../test-core';
import { startBrain } from './index';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-core')).eventModule);

const flush = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};

describe('startBrain permission notice (ADR 0101)', () => {
  it('[CAP-36] [CAP-37] reads Input Monitoring and Accessibility at launch and asks for neither', async () => {
    core.invoke.mockImplementation(async (cmd: string) =>
      cmd === 'permission_status' ? { status: 'unknown' } : {},
    );
    startBrain();
    await flush();
    const calls = core.invoke.mock.calls.map(([cmd, a]) => [cmd, a]);
    expect(calls).toContainEqual([
      'permission_status',
      { args: { permission: 'input_monitoring' } },
    ]);
    expect(calls).toContainEqual(['permission_status', { args: { permission: 'accessibility' } }]);
    expect(calls.map(([cmd]) => cmd)).not.toContain('permission_request');
    expect(core.emitTo).toHaveBeenCalledWith('hub', 'hub:permissions', {
      needs: [
        { permission: 'input_monitoring', action: 'allow' },
        { permission: 'accessibility', action: 'allow' },
      ],
    });
  });
});
