import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { COMMANDS, EVENTS, IPC_ERROR_CODES, type CommandName, type IpcError } from './ipc';

const schemaDoc = JSON.parse(
  readFileSync(resolve(import.meta.dirname, '../../src-tauri/schemas/ipc-commands.json'), 'utf8'),
) as {
  commands: { name: string; windows: string[]; phase: string }[];
  events: { name: string; specName: string; direction: string }[];
  errors: string[];
};

describe('IPC contract (TS side)', () => {
  it('[ARC-4] the TS command list equals the Rust table with the same windows', () => {
    const ts = Object.entries(COMMANDS).map(([name, c]) => ({
      name,
      windows: [...c.windows],
      phase: c.phase,
    }));
    expect(ts).toEqual(schemaDoc.commands);
  });

  it('[ARC-5] the TS event names equal the Rust event names', () => {
    const ts = Object.entries(EVENTS).map(([specName, e]) => ({
      name: e.name,
      specName,
      direction: e.direction,
    }));
    expect(ts).toEqual(schemaDoc.events);
  });

  it('[ARC-5] event names use only characters Tauri allows', () => {
    for (const e of Object.values(EVENTS)) expect(e.name).toMatch(/^[A-Za-z0-9\-/:_]+$/);
  });

  it('[ARC-4] MOG and card views may only call view_emit (the hub also has the brain-and-hub rows)', () => {
    const viewCommands = (Object.keys(COMMANDS) as CommandName[]).filter((n) =>
      COMMANDS[n].windows.some((w) => w !== 'brain' && w !== 'hub'),
    );
    expect(viewCommands).toEqual(['view_emit']);
  });
});

describe('capture lease (contract)', () => {
  it('[CAP-02a] capture_stop and capture_pause take mogId and leaseId', () => {
    const doc = schemaDoc as unknown as { arguments: Record<string, { required?: string[] }> };
    for (const cmd of ['capture_stop', 'capture_pause']) {
      expect(doc.arguments[cmd]?.required).toEqual(['mogId', 'leaseId']);
    }
  });
});

describe('content-free IPC errors (#333, ADR 0032)', () => {
  it('[SEC-03] the TS error codes equal the Rust ERROR_CODES', () => {
    expect([...IPC_ERROR_CODES]).toEqual(schemaDoc.errors);
  });

  it('[SEC-03] an IpcError carries only its code', () => {
    const cap: IpcError = { error: 'mog_cap_reached' };
    // @ts-expect-error a rejection names no window label
    const leaky: IpcError = { error: 'window_not_allowed', window: 'mog-a1' };
    expect(Object.keys(cap)).toEqual(['error']);
    expect(leaky.error).toBe('window_not_allowed');
  });

  it('[ARC-5] [UX-06a] mog.level is a brain-to-view event', () => {
    expect(EVENTS['mog.level']).toEqual({ name: 'mog:level', direction: 'brain_to_view' });
  });
});
