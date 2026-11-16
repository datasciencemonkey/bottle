import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { COMMANDS, EVENTS, type CommandName } from './ipc';

const schemaDoc = JSON.parse(
  readFileSync(resolve(import.meta.dirname, '../../src-tauri/schemas/ipc-commands.json'), 'utf8'),
) as {
  commands: { name: string; windows: string[]; phase: string }[];
  events: { name: string; specName: string; direction: string }[];
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
