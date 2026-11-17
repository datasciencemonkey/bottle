import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import type {
  CaptureMode,
  CaptureState,
  Intent,
  MicHealth,
  MogId,
  Mood,
  Profile,
  ProfileInfo,
  ProfileName,
  RouterDecision,
  SegmentRef,
  SessionPolicy,
  Tags,
  TokenStatus,
  Track,
  Transcript,
} from '../src/shared/interfaces';
import type {
  AppSetting,
  AppSettingKey,
  CardUpdate,
  ConsentRecordKind,
  ConsentRecordView,
  MicLevelFrame,
  MogViewState,
  TalkKeyReason,
  TalkKeyState,
  TalkKeyStatus,
} from '../src/shared/ipc';

// Each JSON fixture in tests/fixtures/ipc-types has a typed twin here. `satisfies` makes
// `pnpm typecheck` fail if a fixture does not fit its TS type; the test below checks that the
// JSON file equals its twin. The Rust test (crates/mog-core/tests/ipc_types.rs) round-trips
// the same files through the serde mirror in `mog_core::types`.
const MOG: MogId = '6f1c2a9e-4b3d-4e8a-9c7f-2d5e8a1b3c4d';
const OTHER_MOG: MogId = '0a9b8c7d-6e5f-4a3b-8c2d-1e0f9a8b7c6d';
const PROFILE_FULL = {
  name: 'DEFAULT',
  host: 'https://example.cloud.databricks.com',
  authType: 'databricks-cli',
  supported: true,
  workspaceId: '1234567890123456',
} satisfies Profile;

// Every value of each string union, one fixture per union (the Rust enum name). The `Equal`
// checks make `pnpm typecheck` fail if a value is missing or extra, so a Rust enum that drops
// or renames a value fails the round trip.
type Equal<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false;
const MOODS = [
  'idle',
  'listening',
  'processing',
  'answering',
  'success',
  'error',
  'sleeping',
] as const satisfies Mood[];
const TRACKS = ['you', 'others'] as const satisfies Track[];
const CAPTURE_MODES = ['ptt', 'hands_free', 'session'] as const satisfies CaptureMode[];
const INTENTS = [
  'question',
  'memory_question',
  'context',
  'command',
  'dictation',
  'noise',
] as const satisfies Intent[];
const TAG_APPS = ['bottle'] as const satisfies Tags['app'][];
const TAG_OPS = [
  'asr',
  'cleanup',
  'router',
  'writer',
  'judge',
  'embed',
  'genie',
  'merge',
  'agent',
  'page',
] as const satisfies Tags['op'][];
const OTHERS_TRACKS = [
  'off',
  'ok',
  'zero_buffers',
] as const satisfies CaptureState['othersTrack'][];
const PROVIDERS = [
  'local_parakeet',
  'dbx_gemini',
  'dbx_whisper',
] as const satisfies Transcript['provider'][];
const TOKEN_STATES = [
  'valid',
  'expiring',
  'login_required',
  'unknown',
] as const satisfies TokenStatus['state'][];
const ROUTER_COMMANDS = [
  'none',
  'switch_workspace_once',
  'switch_workspace_session',
  'reset_workspace',
  'new_topic',
  'cancel',
  'stop_recording',
  'pause',
  'resume',
  'forget_session',
  'other',
] as const satisfies RouterDecision['command'][];
// The `hotkey.status` payload (#297, #294, ADR 0039).
const TALK_KEY_STATES = ['active', 'paused', 'unavailable'] as const satisfies TalkKeyState[];
const TALK_KEY_REASONS = [
  'secure_input',
  'elevated_foreground',
  'fallback_chord_unavailable',
  'hook_failed',
] as const satisfies TalkKeyReason[];
const CONSENT_RECORD_KINDS = [
  'notice',
  'only_me',
  'others_agreed',
] as const satisfies ConsentRecordKind[];
// The mic fault in `capture.state` and the app setting (#453, CAP-17b, CAP-22).
const MIC_HEALTHS = [
  'ok',
  'zero_buffers',
  'permission_denied',
  'no_device',
] as const satisfies MicHealth[];
const APP_SETTING_KEYS = ['app.capture.preferBuiltInMic'] as const satisfies AppSettingKey[];
const everyUnionValueListed: [
  Equal<(typeof MOODS)[number], Mood>,
  Equal<(typeof TRACKS)[number], Track>,
  Equal<(typeof CAPTURE_MODES)[number], CaptureMode>,
  Equal<(typeof INTENTS)[number], Intent>,
  Equal<(typeof TAG_APPS)[number], Tags['app']>,
  Equal<(typeof TAG_OPS)[number], Tags['op']>,
  Equal<(typeof OTHERS_TRACKS)[number], CaptureState['othersTrack']>,
  Equal<(typeof PROVIDERS)[number], Transcript['provider']>,
  Equal<(typeof TOKEN_STATES)[number], TokenStatus['state']>,
  Equal<(typeof ROUTER_COMMANDS)[number], RouterDecision['command']>,
  Equal<(typeof TALK_KEY_STATES)[number], TalkKeyState>,
  Equal<(typeof TALK_KEY_REASONS)[number], TalkKeyReason>,
  Equal<(typeof MIC_HEALTHS)[number], MicHealth>,
  Equal<(typeof MIC_HEALTHS)[number], NonNullable<CaptureState['mic']>>,
  Equal<(typeof APP_SETTING_KEYS)[number], AppSettingKey>,
  Equal<TalkKeyStatus['talkKey'], TalkKeyState>,
  Equal<TalkKeyStatus['reason'], TalkKeyReason | null>,
  Equal<(typeof CONSENT_RECORD_KINDS)[number], ConsentRecordKind>,
  Equal<ConsentRecordView['sessionId'], string | null>,
] = [
  true,
  true,
  true,
  true,
  true,
  true,
  true,
  true,
  true,
  true,
  true,
  true,
  true,
  true,
  true,
  true,
  true,
  true,
  true,
];

// Brain-to-view payloads (#285, ADR 0040).
const VIEW_STATE = {
  mogId: MOG,
  mood: 'listening',
  overlays: {
    attention: false,
    recording: true,
    offline: false,
    hop: 'staging',
    localOnly: false,
  },
  badges: [
    {
      kind: 'identity',
      text: 'DEFAULT · Genie',
      label: 'DEFAULT · Genie, workspace example.cloud.databricks.com',
      detail: 'example.cloud.databricks.com',
      hoverOnly: true,
    },
    {
      kind: 'stt',
      text: 'STT: local',
      label: 'STT: local, speech stays on this device',
      hoverOnly: true,
      action: 'open_data_disclosures',
    },
  ],
  homeDomain: {
    scope: 'home',
    paste: 'The domain list cannot be read.',
    error: 'Paste a domain ID.',
  },
  trace: {
    seq: 7,
    eventAt: 1763251200000.25,
    sentAt: 1763251200001.5,
    keyDownAt: 1763251199999.75,
  },
} satisfies MogViewState;
const CARD_UPDATE = {
  mogId: MOG,
  blurbs: [
    { id: 'b1', text: 'Revenue last quarter was 4.2 M.', pinned: true, completed: true },
    {
      id: 'b2',
      text: 'Two answers',
      parts: [
        { id: 'b2-1', text: 'Prod revenue rose.' },
        { id: 'b2-2', text: 'Staging churn fell.' },
      ],
    },
  ],
  chips: [{ id: 'c1', label: 'Show the SQL' }],
  answerMd: '**4.2 M** in EMEA.',
  historyCount: 3,
  sttBadge: 'STT: local',
} satisfies CardUpdate;

const typed: Record<string, unknown> = {
  MogViewState: VIEW_STATE,
  'MogViewState.minimal': {
    mogId: OTHER_MOG,
    mood: 'idle',
    overlays: { attention: false, recording: false, offline: false, localOnly: true },
  } satisfies MogViewState,
  CardUpdate: CARD_UPDATE,
  'CardUpdate.sections': {
    mogId: '6f1c2a9e-4b3d-4e8a-9c7f-2d5e8a1b3c4d',
    blurbs: [],
    chips: [{ id: 'c1', label: 'Show the SQL' }],
    overflowChips: [{ id: 'c5', label: 'Open in Genie' }],
    sections: [
      { id: 'answer', md: '**4.2 M**' },
      { id: 'keyNumbers', items: ['4.2 M'] },
      { id: 'sql', sql: 'SELECT 1', collapsed: true },
      {
        id: 'tablePreview',
        columns: ['region', 'revenue'],
        rows: [['EMEA', '4.2 M']],
        moreRows: 3,
        moreColumns: 1,
      },
      { id: 'sentToGenie', text: 'Revenue by region' },
      { id: 'resend' },
      { id: 'copy', formats: ['markdown', 'csv'] },
      {
        id: 'footer',
        sttBadge: 'STT: local',
        deepLink: 'https://example.cloud.databricks.com/genie',
      },
      { id: 'dataFlow', text: 'Sent to Genie in workspace A.' },
    ],
    pageDomain: {
      scope: 'draft',
      reviewId: 'r1',
      current: { id: 'd1', name: 'Sales' },
      options: [
        { id: 'd1', name: 'Sales' },
        { id: 'd2', name: 'Ops' },
      ],
    },
    pagesChecklist: {
      text: 'Pages are not ready in this workspace.',
      items: ['Turn on UC Pages.'],
    },
  } satisfies CardUpdate,
  'CardUpdate.minimal': { mogId: OTHER_MOG, blurbs: [], chips: [] } satisfies CardUpdate,
  MicLevelFrame: { owner: MOG, dbfs: -42.5, t: 1250.5 } satisfies MicLevelFrame,
  'MicLevelFrame.closed': { owner: null, dbfs: -120.5, t: 1300.25 } satisfies MicLevelFrame,
  TalkKeyStatus: { talkKey: 'paused', reason: 'secure_input' } satisfies TalkKeyStatus,
  'TalkKeyStatus.active': { talkKey: 'active', reason: null } satisfies TalkKeyStatus,
  TalkKeyState: TALK_KEY_STATES,
  TalkKeyReason: TALK_KEY_REASONS,
  // The `consent_records_list` reply (#430, ADR 0060).
  ConsentRecordView: {
    recordId: 'c0c0c0c0-0000-4000-8000-000000000002',
    kind: 'others_agreed',
    noticeVersion: 3,
    sessionId: '5e550000-0000-4000-8000-000000000001',
    acceptedAt: 1736942400000,
  } satisfies ConsentRecordView,
  'ConsentRecordView.notice': {
    recordId: 'c0c0c0c0-0000-4000-8000-000000000001',
    kind: 'notice',
    noticeVersion: 3,
    sessionId: null,
    acceptedAt: 1736942400000,
  } satisfies ConsentRecordView,
  ConsentRecordKind: CONSENT_RECORD_KINDS,
  MicHealth: MIC_HEALTHS,
  AppSettingKey: APP_SETTING_KEYS,
  AppSetting: { key: 'app.capture.preferBuiltInMic', value: true } satisfies AppSetting,
  MogId: MOG satisfies MogId,
  ProfileName: 'DEFAULT' satisfies ProfileName,
  Mood: MOODS,
  Track: TRACKS,
  CaptureMode: CAPTURE_MODES,
  Intent: INTENTS,
  TagApp: TAG_APPS,
  TagOp: TAG_OPS,
  OthersTrack: OTHERS_TRACKS,
  TranscriptProvider: PROVIDERS,
  TokenState: TOKEN_STATES,
  RouterCommand: ROUTER_COMMANDS,
  Tags: { app: 'bottle', mog: MOG, op: 'router' } satisfies Tags,
  SessionPolicy: {
    checkInAtMin: 55,
    softCapMin: 60,
    extendMin: 30,
    silenceNudgeMin: 5,
    silenceStopMin: 15,
    hardCapMin: 240,
    lockStopMin: 10,
    holdMaxSec: 120,
    handsFreeMaxMin: 5,
  } satisfies SessionPolicy,
  CaptureState: {
    owner: MOG,
    mode: 'session',
    leaseId: '17',
    recording: true,
    paused: false,
    sessionId: 'sess-01',
    startedAt: 1763251200000,
    directedTo: OTHER_MOG,
    othersTrack: 'ok',
    mic: 'ok',
  } satisfies CaptureState,
  'CaptureState.micFault': {
    owner: MOG,
    mode: 'ptt',
    recording: false,
    paused: false,
    othersTrack: 'off',
    mic: 'zero_buffers',
  } satisfies CaptureState,
  'CaptureState.minimal': {
    recording: false,
    paused: false,
    othersTrack: 'off',
  } satisfies CaptureState,
  SegmentRef: {
    segmentId: 'seg-0001',
    mogId: MOG,
    leaseId: '17',
    sessionId: 'sess-01',
    track: 'others',
    tStartMs: 12000,
    tEndMs: 15500,
    final: true,
    directedTo: OTHER_MOG,
  } satisfies SegmentRef,
  'SegmentRef.minimal': {
    segmentId: 'seg-0002',
    mogId: MOG,
    leaseId: '18',
    track: 'you',
    tStartMs: 0,
    tEndMs: 900,
    final: false,
  } satisfies SegmentRef,
  Transcript: {
    segmentId: 'seg-0001',
    text: 'What was revenue last quarter?',
    raw: 'uh what was revenue last quarter',
    lang: 'en',
    provider: 'local_parakeet',
    model: 'parakeet-tdt-0.6b-v3',
    runtime: 'onnx',
    latencyMs: 420,
  } satisfies Transcript,
  'Transcript.minimal': {
    segmentId: 'seg-0003',
    text: 'Hello.',
    raw: 'hello',
    provider: 'dbx_whisper',
    model: 'whisper-large-v3',
    runtime: 'serving',
    latencyMs: 1300,
  } satisfies Transcript,
  Profile: PROFILE_FULL,
  'Profile.minimal': {
    name: 'legacy-pat',
    host: 'https://example.azuredatabricks.net',
    authType: 'pat',
    supported: false,
  } satisfies Profile,
  'ProfileInfo.list': [
    PROFILE_FULL,
    {
      name: 'other',
      host: 'https://other.gcp.databricks.com',
      authType: 'databricks-cli',
      supported: true,
    },
  ] satisfies ProfileInfo[],
  TokenStatus: {
    profile: 'DEFAULT',
    state: 'expiring',
    expiresAt: '2026-11-16T04:00:00Z',
  } satisfies TokenStatus,
  'TokenStatus.minimal': { profile: 'other', state: 'login_required' } satisfies TokenStatus,
  RouterDecision: {
    intent: 'question',
    cleaned_text: 'What was revenue last quarter in EMEA?',
    genie_question: 'What was revenue last quarter in EMEA?',
    target_profile_hint: '',
    confidence: 0.92,
    follow_up: false,
    topic_label: 'revenue',
    part_questions: [],
    part_profile_hints: [],
    command: 'none',
    command_arg: '',
    context_note: '',
    injection_suspected: false,
    directed_mog_id: '',
  } satisfies RouterDecision,
  'RouterDecision.command': {
    intent: 'command',
    cleaned_text: 'Switch to the staging workspace for this session.',
    genie_question: '',
    target_profile_hint: 'staging',
    confidence: 0.81,
    follow_up: false,
    topic_label: 'workspace',
    part_questions: [],
    part_profile_hints: [],
    command: 'switch_workspace_session',
    command_arg: 'staging',
    context_note: '',
    injection_suspected: false,
    directed_mog_id: OTHER_MOG,
  } satisfies RouterDecision,
  'RouterDecision.split': {
    intent: 'question',
    cleaned_text: 'Compare revenue in prod and churn in staging.',
    genie_question: 'Compare revenue in prod and churn in staging.',
    target_profile_hint: '',
    confidence: 0.74,
    follow_up: true,
    topic_label: 'revenue vs churn',
    part_questions: ['What was revenue last quarter?', 'What was churn last quarter?'],
    part_profile_hints: ['prod', 'staging'],
    command: 'none',
    command_arg: '',
    context_note: 'the user means fiscal quarters',
    injection_suspected: false,
    directed_mog_id: '',
  } satisfies RouterDecision,
};

const dir = join(import.meta.dirname, 'fixtures', 'ipc-types');
const files = readdirSync(dir).filter((f) => f.endsWith('.json'));

// Type-level checks: no IPC-facing type has a field that could carry a credential.
type Keys<T> = T extends readonly (infer E)[] ? keyof E : keyof T;
type Credential = 'token' | 'accessToken' | 'access_token' | 'authorization' | 'password';
type NoCredential<T> = Extract<Keys<T>, Credential> extends never ? true : false;
const noCredentialField: [
  NoCredential<TokenStatus>,
  NoCredential<Profile>,
  NoCredential<CaptureState>,
  NoCredential<SegmentRef>,
  NoCredential<Transcript>,
  NoCredential<RouterDecision>,
  NoCredential<MogViewState>,
  NoCredential<CardUpdate>,
  NoCredential<MicLevelFrame>,
  NoCredential<TalkKeyStatus>,
  NoCredential<ConsentRecordView>,
  NoCredential<AppSetting>,
] = [true, true, true, true, true, true, true, true, true, true, true, true];

describe('IPC type fixtures', () => {
  it('every fixture file has a typed twin, and every twin has a file', () => {
    expect(files.map((f) => f.replace(/\.json$/, '')).sort()).toEqual(Object.keys(typed).sort());
  });

  it('each union fixture lists every value of its TS union', () => {
    expect(everyUnionValueListed.every(Boolean)).toBe(true);
  });

  it.each(files)('%s equals its typed twin', (file) => {
    const json: unknown = JSON.parse(readFileSync(join(dir, file), 'utf8'));
    expect(json).toEqual(typed[file.replace(/\.json$/, '')]);
  });

  it('[ARC-6] TokenStatus, Profile and the capture types hold no token field', () => {
    expect(noCredentialField.every(Boolean)).toBe(true);
    for (const file of files) {
      const text = readFileSync(join(dir, file), 'utf8');
      expect(text).not.toMatch(/"(token|accessToken|access_token|authorization|password)"/i);
    }
  });
});

describe('mic fault and app setting (#453)', () => {
  it('[CAP-17b] a CaptureState carries the mic fault code for the fix-it card', () => {
    const fault = typed['CaptureState.micFault'] as CaptureState;
    expect(fault.mic).toBe('zero_buffers');
    // Older payloads have no mic field and still type-check.
    const idle = typed['CaptureState.minimal'] as CaptureState;
    expect(idle.mic).toBeUndefined();
  });

  it('[CAP-17b] [CAP-01b] the mic fault is a fixed code with no device identity', () => {
    // @ts-expect-error a device name is not a MicHealth
    const named: MicHealth = 'AirPods Pro';
    // @ts-expect-error the fault is a code, not an object with a device
    const withDevice: CaptureState['mic'] = { code: 'zero_buffers', device: 'AirPods Pro' };
    expect([named, withDevice]).toHaveLength(2);
    for (const code of MIC_HEALTHS) expect(code).toMatch(/^[a-z_]+$/);
  });

  it('[CAP-22] the app setting app.capture.preferBuiltInMic has a boolean value', () => {
    const s = typed['AppSetting'] as AppSetting;
    expect(s).toEqual({ key: 'app.capture.preferBuiltInMic', value: true });
    // @ts-expect-error the value is a boolean
    const text: AppSetting = { key: 'app.capture.preferBuiltInMic', value: 'yes' };
    expect(text.key).toBe('app.capture.preferBuiltInMic');
  });
});

describe('capture lease (CAP-02a)', () => {
  it('[CAP-02a] [AT-17] a live CaptureState carries its lease ID', () => {
    const live = typed['CaptureState'] as CaptureState;
    expect(live.recording).toBe(true);
    expect(live.leaseId).toBe('17');
    const idle = typed['CaptureState.minimal'] as CaptureState;
    expect(idle.leaseId).toBeUndefined();
  });

  it('[CAP-02a] [AT-17] every segment names the lease it came from', () => {
    // Type level: a SegmentRef without leaseId does not compile.
    // @ts-expect-error leaseId is required
    const noLease: SegmentRef = {
      segmentId: 's',
      mogId: MOG,
      track: 'you',
      tStartMs: 0,
      tEndMs: 1,
      final: true,
    };
    expect(noLease.segmentId).toBe('s');
    for (const key of ['SegmentRef', 'SegmentRef.minimal']) {
      expect((typed[key] as SegmentRef).leaseId).toMatch(/^[A-Za-z0-9-]{1,64}$/);
    }
  });
});

describe('talk-key status (CAP-39c, CAP-42)', () => {
  it('[CAP-39c] [CAP-42] reason is required and nullable, and the payload names no window or key', () => {
    // @ts-expect-error reason is required (null when the talk key works)
    const noReason: TalkKeyStatus = { talkKey: 'active' };
    expect(noReason.talkKey).toBe('active');
    // @ts-expect-error the payload carries no window title
    const leaky: TalkKeyStatus = { talkKey: 'paused', reason: 'elevated_foreground', title: 'x' };
    expect(leaky.reason).toBe('elevated_foreground');
    expect(Object.keys(typed['TalkKeyStatus.active'] as TalkKeyStatus).sort()).toEqual([
      'reason',
      'talkKey',
    ]);
  });
});
