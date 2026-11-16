import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import type {
  CaptureMode,
  CaptureState,
  Intent,
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

const typed: Record<string, unknown> = {
  MogId: MOG satisfies MogId,
  ProfileName: 'DEFAULT' satisfies ProfileName,
  Mood: [
    'idle',
    'listening',
    'processing',
    'answering',
    'success',
    'error',
    'sleeping',
  ] satisfies Mood[],
  Track: ['you', 'others'] satisfies Track[],
  CaptureMode: ['ptt', 'hands_free', 'session'] satisfies CaptureMode[],
  Intent: [
    'question',
    'memory_question',
    'context',
    'command',
    'dictation',
    'noise',
  ] satisfies Intent[],
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
    recording: true,
    paused: false,
    sessionId: 'sess-01',
    startedAt: 1763251200000,
    directedTo: OTHER_MOG,
    othersTrack: 'ok',
  } satisfies CaptureState,
  'CaptureState.minimal': {
    recording: false,
    paused: false,
    othersTrack: 'off',
  } satisfies CaptureState,
  SegmentRef: {
    segmentId: 'seg-0001',
    mogId: MOG,
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
] = [true, true, true, true, true, true];

describe('IPC type fixtures', () => {
  it('every fixture file has a typed twin, and every twin has a file', () => {
    expect(files.map((f) => f.replace(/\.json$/, '')).sort()).toEqual(Object.keys(typed).sort());
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
