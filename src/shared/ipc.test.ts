import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  COMMANDS,
  EVENTS,
  IPC_ERROR_CODES,
  type CaptureStopReason,
  type CaptureStopped,
  type CommandArgs,
  type CommandName,
  type CommandReturns,
  type IpcError,
  type ViewEmitKind,
  VIEW_EMIT_KINDS,
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

describe('#507 ready and #509 the hub-only notice (contract, ADR 0091)', () => {
  const doc = schemaDoc as unknown as {
    arguments: Record<
      string,
      { required?: string[]; $defs?: Record<string, { enum?: string[] }> } & Record<string, unknown>
    >;
  };

  it('[MOG-18a] [MOG-18b] ViewEmitKind has ready and equals the Rust enum', () => {
    const ready: ViewEmitKind = 'ready';
    expect(VIEW_EMIT_KINDS).toContain(ready);
    expect([...VIEW_EMIT_KINDS]).toEqual(doc.arguments.view_emit?.$defs?.ViewEmitKind?.enum);
    expect(COMMANDS.view_emit).toEqual({ windows: ['hub', 'mog', 'card'], phase: 'MVP-D' });
  });

  it('[SEC-02a] [MOG-05b] [MOG-06d] hub_notice_send is brain-only; view_notice_subscribe is hub-only', () => {
    expect(COMMANDS.hub_notice_send).toEqual({ windows: ['brain'], phase: 'MVP-D' });
    expect(COMMANDS.view_notice_subscribe).toEqual({ windows: ['hub'], phase: 'MVP-D' });
    expect(doc.arguments.hub_notice_send?.required).toEqual(['kind', 'mogId', 'text']);
    expect(doc.arguments.view_notice_subscribe?.required).toEqual(['onNotice']);
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

  it('[G-05b] [G-14] [W-14] [SEC-05a] every R8 mcp_call failure code is an IpcErrorCode (#479)', () => {
    const r8 = [
      'genie_one_off',
      'auth_expired',
      'rate_limited',
      'non_json_reply',
      'timeout',
      'offline',
      'host_refused',
      'redirect_refused',
      'reply_too_large',
      'bad_reply',
      'protocol_unsupported',
      'tool_not_allowed',
      'invalid_tool_args',
      'invalid_meta',
      'profile_invalid',
    ];
    for (const code of r8) expect(IPC_ERROR_CODES).toContain(code);
    const off: IpcError = { error: 'genie_one_off' };
    expect(Object.keys(off)).toEqual(['error']);
  });

  it('[G-07b] [ARC-F1] [G-16c] poll_inflight is an IpcErrorCode (#479)', () => {
    expect(IPC_ERROR_CODES).toContain('poll_inflight');
  });

  it('[SEC-03] consent_storage_failed is a fixed IPC error code', () => {
    const failed: IpcError = { error: 'consent_storage_failed' };
    expect(IPC_ERROR_CODES).toContain(failed.error);
  });
});

describe('app settings (CAP-22, #453)', () => {
  it('[CAP-22] [MOG-08a] app_setting_read and app_setting_write are brain-and-hub MVP-D rows', () => {
    expect(COMMANDS.app_setting_read).toEqual({ windows: ['brain', 'hub'], phase: 'MVP-D' });
    expect(COMMANDS.app_setting_write).toEqual({ windows: ['brain', 'hub'], phase: 'MVP-D' });
    const names = schemaDoc.commands.map((c) => c.name);
    expect(names).toContain('app_setting_read');
    expect(names).toContain('app_setting_write');
  });

  it('[CAP-22] app_setting_read takes a key; app_setting_write takes a key and a typed value', () => {
    const doc = schemaDoc as unknown as {
      arguments: Record<string, { required?: string[]; additionalProperties?: boolean }>;
    };
    expect(doc.arguments['app_setting_read']?.required).toEqual(['key']);
    expect(doc.arguments['app_setting_read']?.additionalProperties).toBe(false);
    const read: CommandArgs['app_setting_read'] = { key: 'app.capture.preferBuiltInMic' };
    const write: CommandArgs['app_setting_write'] = {
      key: 'app.capture.preferBuiltInMic',
      value: true,
    };
    // @ts-expect-error the value of app.capture.preferBuiltInMic is a boolean
    const bad: CommandArgs['app_setting_write'] = { key: 'app.capture.preferBuiltInMic', value: 1 };
    // @ts-expect-error only known keys
    const unknown: CommandArgs['app_setting_read'] = { key: 'app.capture.other' };
    const reply: CommandReturns['app_setting_write'] = write;
    expect([read.key, bad.key, unknown.key, reply.value]).toHaveLength(4);
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
    expect(EVENTS['capture.stopped']).toEqual({
      name: 'capture:stopped',
      direction: 'core_to_brain',
    });
    const stop: CaptureStopped = { leaseId: '1', mogId: 'a1', mode: 'ptt', reason: 'hold_cap' };
    expect(Object.keys(stop).sort()).toEqual(['leaseId', 'mode', 'mogId', 'reason']);
  });

  it('[CAP-02a] CaptureStopReason has the R2 stop codes', () => {
    const reasons: CaptureStopReason[] = [
      'manual',
      'hold_cap',
      'hands_free_cap',
      'preempted',
      'panic',
      'device_lost',
      'released',
      'discarded',
      'tap',
      'escape',
      'vad_end',
      'zero_buffers',
    ];
    expect(new Set(reasons).size).toBe(12);
  });
});
