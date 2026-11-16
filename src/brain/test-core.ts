/**
 * Test double for `@tauri-apps/api/core` and `@tauri-apps/api/event` in brain tests.
 * Brain tests mock the Tauri modules with `vi.mock` and read these handles.
 */
export const core = {
  invoke: vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => ({})),
  channels: [] as FakeChannel[],
  emitTo: vi.fn(async (_target: string, _event: string, _payload?: unknown) => undefined),
  emit: vi.fn(async () => undefined),
  reset(): void {
    this.invoke.mockReset();
    this.invoke.mockImplementation(async () => ({}));
    this.emitTo.mockClear();
    this.emit.mockClear();
    this.channels.length = 0;
  },
};

export class FakeChannel {
  onmessage: (m: unknown) => void = () => undefined;
  constructor() {
    core.channels.push(this);
  }
}

export const coreModule = {
  invoke: (cmd: string, args?: unknown) => core.invoke(cmd, args),
  Channel: FakeChannel,
};

export const eventModule = {
  emitTo: (target: string, event: string, payload?: unknown) => core.emitTo(target, event, payload),
  emit: () => core.emit(),
  listen: async () => () => undefined,
};
