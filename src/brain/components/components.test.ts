import { readdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import type {
  AskRequest,
  PageDraft,
  RouterDecision,
  SegmentRef,
  Tags,
} from '../../shared/interfaces';
import { BrainActor } from '../actor';
import { core } from '../test-core';
import { NotImplementedError } from '../errors';
import { BRAIN_COMPONENTS, startBrain } from './index';
import { createAgentBridge } from './agent-bridge';
import { createCapturePolicy } from './capture-policy';
import { createCleanupProvider } from './cleanup-provider';
import { createConsentService } from './consent-service';
import { createConversationManager } from './conversation-manager';
import { createGenieBridge } from './genie-bridge';
import { createMemoryService } from './memory-service';
import { createMogManager } from './mog-manager';
import { createOperationTracker } from './operation-tracker';
import { createPageService } from './page-service';
import { createProfileService } from './profile-service';
import { createRecoveryManager } from './recovery-manager';
import { createRouterContextGate } from './router-context-gate';
import type { RouterInput } from './router-step';
import { createSttProvider } from './stt-provider';
import { createRouterModelDouble } from '../../test-support/model-invoke';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-core')).eventModule);

const dir = import.meta.dirname;
const MOG = '00000000-0000-4000-8000-000000000001';
const tags: Tags = { app: 'bottle', mog: MOG, op: 'genie' };
const binding = { kind: 'genie_one_mcp', profile: 'p' } as const;
const ask: AskRequest = { opId: 'op-1', mogId: MOG, binding, question: 'q', tags };
const ref = { backend: 'genie_one_mcp', profile: 'p', host: 'h', conversationId: 'c' } as const;
const seg: SegmentRef = {
  segmentId: 's',
  mogId: MOG,
  leaseId: '1',
  track: 'you',
  tStartMs: 0,
  tEndMs: 1,
  final: true,
};
const page: PageDraft = {
  domainId: 'd',
  displayName: 'n',
  body: 'b',
  synonyms: [],
  description: '',
  requestId: 'r',
  source: 'you',
};
const decision = {} as RouterDecision;

/** Calls `fn` and checks that it throws (or rejects with) a NotImplementedError for `issue`. */
async function expectStub(fn: () => unknown, issue: number): Promise<void> {
  let err: unknown;
  try {
    await fn();
  } catch (e) {
    err = e;
  }
  expect(err).toBeInstanceOf(NotImplementedError);
  expect((err as NotImplementedError).issue).toBe(issue);
}

describe('brain component registry (4.3)', () => {
  it('[MOG-08b] lists T1..T15 in table order with the 4.3 names', () => {
    expect(BRAIN_COMPONENTS.map((c) => [c.id, c.name])).toEqual([
      ['T1', 'MogManager'],
      ['T2', 'OperationTracker'],
      ['T3', 'CapturePolicy'],
      ['T4', 'SttProvider'],
      ['T5', 'CleanupProvider'],
      ['T6', 'RouterContextGate'],
      ['T7', 'ProfileService'],
      ['T8', 'GenieBridge'],
      ['T9', 'ConversationManager'],
      ['T10', 'MemoryService'],
      ['T11', 'MoodMapper'],
      ['T12', 'ConsentService'],
      ['T13', 'RecoveryManager'],
      ['T14', 'PageService'],
      ['T15', 'AgentBridge'],
    ]);
  });

  it('[MOG-08b] [ARC-2] startBrain constructs every component and the Channel once', async () => {
    const a = startBrain();
    const b = startBrain();
    expect(b).toBe(a);
    expect(Object.keys(a.components)).toEqual(BRAIN_COMPONENTS.map((c) => c.id));
    for (const c of BRAIN_COMPONENTS) expect(a.components[c.id]).toBeDefined();
    await expect(a.ready).resolves.toBe(true);
    expect(core.invoke.mock.calls.filter(([c]) => c === 'brain_subscribe')).toHaveLength(1);
    expect(core.channels).toHaveLength(1);
  });

  it('[MOG-08b] [ARC-2] the brain entry point starts the components', () => {
    const main = readFileSync(resolve(dir, '../main.ts'), 'utf8');
    expect(main).toMatch(/import \{ startBrain \} from '\.\/components'/);
    expect(main).toMatch(/startBrain\(\);/);
  });

  it('[MOG-08b] every component module lives in the brain bundle with its 4.3 row', () => {
    const files = readdirSync(dir).filter((f) => f.endsWith('.ts') && !f.endsWith('.test.ts'));
    const rows = readFileSync(resolve(dir, '../../../research/final-requirements.md'), 'utf8');
    for (const id of BRAIN_COMPONENTS.map((c) => c.id)) {
      const row = rows.split('\n').find((l) => l.startsWith(`| ${id} |`));
      expect(row, id).toBeDefined();
      const responsibilities = (row ?? '').split('|')[3]?.trim() ?? '<missing>';
      const owner = files.filter(
        (f) =>
          readFileSync(resolve(dir, f), 'utf8').includes(`${id} `) &&
          readFileSync(resolve(dir, f), 'utf8')
            .replace(/\s*\n\s*\*\s*/g, ' ')
            .includes(responsibilities),
      );
      expect(owner, `${id} doc comment copies its 4.3 row`).toHaveLength(1);
    }
  });

  it('[ARC-2] every stub names its feature issue in a TODO', () => {
    for (const f of readdirSync(dir).filter((x) => x.endsWith('.ts') && !x.endsWith('.test.ts'))) {
      const src = readFileSync(resolve(dir, f), 'utf8');
      for (const m of src.matchAll(/new NotImplementedError\([^)]*?(\d+)\)/g)) {
        expect(src, `${f} TODO(#${m[1]})`).toContain(`TODO(#${m[1]})`);
      }
    }
  });
});

describe('brain component stubs', () => {
  it('[T1] MogManager stubs list, start and stop (#26)', async () => {
    const t = createMogManager();
    await expectStub(() => t.list(), 26);
    await expectStub(() => t.start(MOG), 26);
    await expectStub(() => t.stop(MOG), 26);
  });

  it('[T2] OperationTracker stubs state, transition and mayAutoResend (#39)', async () => {
    const t = createOperationTracker();
    await expectStub(() => t.state('op'), 39);
    await expectStub(() => t.transition('op', 'SENDING'), 39);
    await expectStub(() => t.mayAutoResend('op'), 39);
  });

  it('[T3] CapturePolicy stubs decide (#142) and directedTo (#119)', async () => {
    const t = createCapturePolicy();
    await expectStub(() => t.decide({ mode: 'ptt', intent: 'question', confidence: 0.9 }), 142);
    await expectStub(() => t.directedTo(seg), 119);
  });

  it('[T4] SttProvider stubs transcribe and activeProvider (#52)', async () => {
    const t = createSttProvider();
    await expectStub(() => t.transcribe(seg, { ...tags, op: 'asr' }), 52);
    await expectStub(() => t.activeProvider(), 52);
  });

  it('[T5] CleanupProvider stubs clean (#55)', async () => {
    await expectStub(() => createCleanupProvider().clean('raw', { ...tags, op: 'cleanup' }), 55);
  });

  it('[T6] RouterContextGate routes (#57) and stubs approve and verify (#152)', async () => {
    const double = createRouterModelDouble([{ decision: {} }]);
    const t = createRouterContextGate({ invoke: double.invoke });
    const input: RouterInput = {
      profile: 'p',
      speakerTrack: 'you',
      captureMode: 'ptt',
      pendingClarification: '',
      recentTopics: [],
      activeMogs: [],
      profiles: [],
      currentTarget: 'p',
    };
    await expect(t.route('text', MOG, input)).resolves.toMatchObject({ intent: 'question' });
    expect(double.calls).toHaveLength(1);
    await expectStub(() => t.approve(ask, 1), 152);
    await expectStub(() => t.verify('a', ask, 1), 152);
  });

  it('[T7] ProfileService implements the 4.7 interface as stubs', async () => {
    const t = createProfileService();
    await expectStub(() => t.list(), 17);
    await expectStub(() => t.status('p'), 17);
    await expectStub(() => t.login('h', 'p'), 162);
    await expectStub(() => t.cliInfo(), 16);
    await expectStub(() => t.home(MOG), 163);
    await expectStub(() => t.resolveTarget([], MOG), 164);
  });

  it('[T8] GenieBridge implements the 4.7 interface as stubs', async () => {
    const t = createGenieBridge();
    expect(t.backendFor(binding).kind).toBe('genie_one_mcp'); // G-02, #28
    await expectStub(() => t.submit(ask), 33);
    await expectStub(() => t.operations(MOG), 39);
    await expectStub(() => t.mapError({ httpStatus: 500 }), 32);
    await expectStub(() => t.workspaceLine('p'), 40);
    await expectStub(() => t.pinnedProfile('c'), 145);
  });

  it('[T9] ConversationManager runs polls of one conversation one at a time on the brain actor', async () => {
    const actor = new BrainActor();
    const t = createConversationManager(actor);
    const log: string[] = [];
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const first = t.enqueuePoll('c1', async () => {
      log.push('a:start');
      await gate;
      log.push('a:end');
    });
    const second = t.enqueuePoll('c1', async () => {
      log.push('b');
    });
    await Promise.resolve();
    expect(actor.pending('poll:c1')).toBe(2);
    release();
    await Promise.all([first, second]);
    expect(log).toEqual(['a:start', 'a:end', 'b']);
  });

  it('[T9] ConversationManager stubs resetTopic (#145) and replayOutbox (#41)', async () => {
    const t = createConversationManager();
    await expectStub(() => t.resetTopic(MOG), 145);
    await expectStub(() => t.replayOutbox(), 41);
  });

  it('[T10] MemoryService implements the 4.7 interface; mode is local (MEM-41)', async () => {
    const t = createMemoryService();
    const q = { mogId: MOG, text: 't', intent: 'question', target: { workspaceId: 'w' } } as const;
    await expectStub(() => t.gate(q), 200);
    await expectStub(() => t.assembleContext(MOG, { workspaceId: 'w' }, 'q'), 92);
    await expectStub(() => t.write([]), 114);
    await expectStub(
      () =>
        t.recordExchange({
          requestId: 'r',
          mogId: MOG,
          ref,
          questionUser: 'q',
          questionSent: 'q',
          status: 'COMPLETED',
          keyNumbers: [],
          deepLink: '',
          homeWorkspace: true,
        }),
      38,
    );
    await expectStub(() => t.forget({ mogId: MOG }), 123);
    expect(t.mode()).toBe('local');
    expect(t.syncStatus()).toEqual({ pending: 0 });
  });

  it('[T11] the registry factory returns the MoodMapper function', () => {
    const t11 = BRAIN_COMPONENTS[10].factory();
    expect(typeof t11).toBe('function');
  });

  it('[T12] ConsentService stubs attest (#64) and records (#431)', async () => {
    const t = createConsentService();
    await expectStub(() => t.attest(MOG, 'only_me'), 64);
    await expectStub(() => t.records(), 431);
  });

  it('[T13] RecoveryManager stubs reconcile, isStale (#39) and offerJournalRecovery (#125)', async () => {
    const t = createRecoveryManager();
    await expectStub(() => t.reconcile(), 39);
    await expectStub(() => t.isStale({ opId: 'op', epoch: 1 }), 39);
    await expectStub(() => t.offerJournalRecovery(), 125);
  });

  it('[T14] PageService implements the 4.7 interface as stubs', async () => {
    const t = createPageService();
    await expectStub(() => t.propose(page), 204);
    await expectStub(() => t.createDraft('r'), 203);
  });

  it('[T15] AgentBridge implements the 4.7 interface as stubs', async () => {
    const t = createAgentBridge();
    await expectStub(() => t.start(MOG, 'claude'), 112);
    await expectStub(
      () => t.send({ mogId: MOG, question: 'q', decision, recall: [], target: 'p' }),
      112,
    );
    await expectStub(() => t.onTool({ callId: 'c', tool: 'set_mood', args: {} }), 141);
  });

  it('[ARC-2] NotImplementedError names the component, method and issue', () => {
    const e = new NotImplementedError('MogManager', 'start', 26);
    expect(e).toBeInstanceOf(Error);
    expect(e.name).toBe('NotImplementedError');
    expect(e.message).toBe('MogManager.start is not implemented yet (TODO #26)');
    expect([e.component, e.method, e.issue]).toEqual(['MogManager', 'start', 26]);
  });
});
