import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  COMMANDS,
  EVENTS,
  IPC_ERROR_CODES,
  type CaptureStopped,
  type CommandArgs,
  type CommandName,
  type CommandReturns,
  type IpcError,
} from './ipc';

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
  it('[CAP-02a] capture_stop, capture_pause and capture_resume take mogId and leaseId', () => {
    const doc = schemaDoc as unknown as { arguments: Record<string, { required?: string[] }> };
    for (const cmd of ['capture_stop', 'capture_pause', 'capture_resume']) {
      expect(doc.arguments[cmd]?.required).toEqual(['mogId', 'leaseId']);
    }
  });
});

describe('capture_resume (CAP-02a, #279)', () => {
  it('[CAP-02a] capture_resume is a brain-only row that returns the capture state', () => {
    expect(COMMANDS.capture_resume).toEqual({ windows: ['brain'], phase: 'MVP-D' });
    const names = Object.keys(COMMANDS);
    expect(names.indexOf('capture_resume')).toBe(names.indexOf('capture_pause') + 1);
    const args: CommandArgs['capture_resume'] = { mogId: 'a1', leaseId: '17' };
    const state: CommandReturns['capture_resume'] = {
      recording: true,
      paused: false,
      othersTrack: 'off',
    };
    expect([args.leaseId, state.paused]).toEqual(['17', false]);
  });
});

describe('content-free IPC errors (#333, ADR 0040)', () => {
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

  it('[ARC-5] [CAP-39c] [CAP-42] hotkey.status is a core-to-brain event on the brain Channel', () => {
    expect(EVENTS['hotkey.status']).toEqual({ name: 'hotkey:status', direction: 'core_to_brain' });
  });

  it('[CAP-36] [CAP-37] permission_request is a hub-only MVP-D command', () => {
    expect(COMMANDS.permission_request).toEqual({ windows: ['hub'], phase: 'MVP-D' });
    const doc = schemaDoc as unknown as {
      arguments: Record<string, { required?: string[]; additionalProperties?: boolean }>;
    };
    expect(doc.arguments['permission_request']?.required).toEqual(['permission']);
    expect(doc.arguments['permission_request']?.additionalProperties).toBe(false);
    const ask: CommandArgs['permission_request'] = { permission: 'input_monitoring' };
    // @ts-expect-error only the two permissions of CAP-36 and CAP-37
    const camera: CommandArgs['permission_request'] = { permission: 'camera' };
    const reply: CommandReturns['permission_request'] = { status: 'unknown' };
    expect([ask.permission, camera.permission, reply.status]).toHaveLength(3);
  });

  it('[ARC-5] [UX-06a] mog.level is a brain-to-view event', () => {
    expect(EVENTS['mog.level']).toEqual({ name: 'mog:level', direction: 'brain_to_view' });
  });

  it('[SEC-03] consent_storage_failed is a fixed IPC error code', () => {
    const failed: IpcError = { error: 'consent_storage_failed' };
    expect(IPC_ERROR_CODES).toContain(failed.error);
  });
});

describe('consent records (SEC-32a, #430, ADR 0060)', () => {
  const doc = schemaDoc as unknown as {
    arguments: Record<string, { required?: string[]; additionalProperties?: boolean }>;
  };

  it('[SEC-32a] [AT-24] consent_notice_record and consent_records_list are brain-only MVP-D rows', () => {
    expect(COMMANDS.consent_notice_record).toEqual({ windows: ['brain'], phase: 'MVP-D' });
    expect(COMMANDS.consent_records_list).toEqual({ windows: ['brain'], phase: 'MVP-D' });
  });

  it('[SEC-32a] [AT-24] the brain names only the notice version; Rust fills user, device and time', () => {
    expect(doc.arguments['consent_notice_record']?.required).toEqual(['noticeVersion']);
    expect(doc.arguments['consent_notice_record']?.additionalProperties).toBe(false);
    expect(doc.arguments['consent_records_list']?.additionalProperties).toBe(false);
    const args: CommandArgs['consent_notice_record'] = { noticeVersion: 3 };
    // @ts-expect-error the caller cannot name the user (SEC-32a)
    const forged: CommandArgs['consent_notice_record'] = { noticeVersion: 3, user: 'x' };
    const reply: CommandReturns['consent_notice_record'] = { recordId: 'r1' };
    expect([args.noticeVersion, forged.noticeVersion, reply.recordId]).toEqual([3, 3, 'r1']);
  });

  it('[SEC-32a] the records list reply is content-free', () => {
    const reply: CommandReturns['consent_records_list'] = {
      records: [
        { recordId: 'r1', kind: 'notice', noticeVersion: 3, sessionId: null, acceptedAt: 1 },
      ],
    };
    const leaky: CommandReturns['consent_records_list'] = {
      records: [
        {
          recordId: 'r1',
          kind: 'notice',
          noticeVersion: 3,
          sessionId: null,
          acceptedAt: 1,
          // @ts-expect-error no user crosses IPC
          user: 'x',
        },
      ],
    };
    expect([reply.records.length, leaky.records.length]).toEqual([1, 1]);
  });
});

describe('capture events (#425, ADR 0055)', () => {
  it('[CAP-02a] [CAP-04a] capture.stopped is a core-to-brain event with a content-free payload', () => {
    expect(EVENTS['capture.stopped']).toEqual({ name: 'capture:stopped', direction: 'core_to_brain' });
    const stop: CaptureStopped = { leaseId: '1', mogId: 'a1', mode: 'ptt', reason: 'hold_cap' };
    expect(Object.keys(stop).sort()).toEqual(['leaseId', 'mode', 'mogId', 'reason']);
  });
});
