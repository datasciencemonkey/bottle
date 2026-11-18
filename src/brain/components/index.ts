/**
 * The registry of the hidden TS brain components T1-T15 of 4.3 (ARC-2, MOG-08b).
 * `startBrain()` constructs each component once; all domain state lives in these objects.
 */
import { BrainBus, sendMicLevel, subscribeBrain } from '../bus';
import { wireForget } from '../forget-wiring';
import { wireHubMogs } from '../hub-mogs-wiring';
import {
  createCaptureLifecycle,
  ipcCaptureControl,
  wireCaptureLifecycle,
} from '../capture-lifecycle';
import { createSystemClock } from '../clock';
import { LevelRouter, subscribeLevels, wireLevelStream } from '../level-stream';
import { wireNotice } from '../notice-wiring';
import { createRequestOwner, sendOwnedCard } from '../request-owner';
import { configName, createAppTalkTarget, wireTalkTarget, type TalkTarget } from '../talk-target';
import { ipcUtteranceDeps } from '../utterance-route';
import { createUtteranceHandler } from '../utterance-wiring';
import { createAgentBridge } from './agent-bridge';
import { createCapturePolicy } from './capture-policy';
import { createCleanupProvider } from './cleanup-provider';
import { createConsentService } from './consent-service';
import { createConversationManager } from './conversation-manager';
import { createGenieBridge } from './genie-bridge';
import { ForgetFlow } from './forget-flow';
import { GenieOneDeleteUtility } from './genie-one-delete';
import { createMemoryService, LocalMemoryService } from './memory-service';
import { createMogManager } from './mog-manager';
import { createMoodMapper } from './mood-mapper';
import { createOperationTracker } from './operation-tracker';
import { createPageService } from './page-service';
import { createProfileService } from './profile-service';
import { createRecoveryManager } from './recovery-manager';
import { createRouterContextGate } from './router-context-gate';
import { createSttProvider } from './stt-provider';

export interface BrainComponentEntry {
  readonly id: `T${number}`;
  readonly name: string;
  readonly factory: () => unknown;
}

/** T1..T15 in 4.3 table order. */
export const BRAIN_COMPONENTS = [
  { id: 'T1', name: 'MogManager', factory: () => createMogManager() },
  { id: 'T2', name: 'OperationTracker', factory: () => createOperationTracker() },
  { id: 'T3', name: 'CapturePolicy', factory: () => createCapturePolicy() },
  { id: 'T4', name: 'SttProvider', factory: () => createSttProvider() },
  { id: 'T5', name: 'CleanupProvider', factory: () => createCleanupProvider() },
  { id: 'T6', name: 'RouterContextGate', factory: () => createRouterContextGate() },
  { id: 'T7', name: 'ProfileService', factory: () => createProfileService() },
  { id: 'T8', name: 'GenieBridge', factory: () => createGenieBridge() },
  { id: 'T9', name: 'ConversationManager', factory: () => createConversationManager() },
  { id: 'T10', name: 'MemoryService', factory: () => createMemoryService() },
  { id: 'T11', name: 'MoodMapper', factory: () => createMoodMapper() },
  { id: 'T12', name: 'ConsentService', factory: () => createConsentService() },
  { id: 'T13', name: 'RecoveryManager', factory: () => createRecoveryManager() },
  { id: 'T14', name: 'PageService', factory: () => createPageService() },
  { id: 'T15', name: 'AgentBridge', factory: () => createAgentBridge() },
] as const satisfies readonly BrainComponentEntry[];

type Entry = (typeof BRAIN_COMPONENTS)[number];

/** The constructed components, keyed by 4.3 ID. */
export type BrainComponents = { readonly [E in Entry as E['id']]: ReturnType<E['factory']> };

/** The started brain: the components, the Channel bus and the subscription result. */
export interface Brain {
  readonly components: BrainComponents;
  readonly bus: BrainBus;
  /** T1 talk-key target (#73, MB-06a, MB-06b, UX-28). */
  readonly talkTarget: TalkTarget;
  /** True once `brain_subscribe` succeeds. TODO(#8): retry and surface a refusal. */
  readonly ready: Promise<boolean>;
}

let brain: Brain | undefined;

/** Construct T1..T15 once and subscribe the brain Channel once. Later calls return the same brain. */
export function startBrain(): Brain {
  if (!brain) {
    const components = Object.fromEntries(
      BRAIN_COMPONENTS.map((c) => [c.id, c.factory()]),
    ) as BrainComponents;
    const bus = new BrainBus();
    // TODO(#8, #141): the other components register their handlers on `bus`.
    // MB-08a, MB-08b: the hub notice talks to T12 (#27).
    wireNotice(bus, components.T12);
    // #26: the hub MOG list and actions (MB-01).
    wireHubMogs(bus, components.T1);
    // #123 (MEM-32a, MEM-32c, MEM-32d): the hub Forget dialog runs Forget on T10 on a user click.
    // TODO(#163): pass `home` (the home host and workspace id from T7) for the memory review link.
    if (components.T10 instanceof LocalMemoryService) {
      wireForget(bus, {
        flow: new ForgetFlow({ memory: components.T10, genieOne: new GenieOneDeleteUtility() }),
      });
    }
    // MB-06a, MB-06b, UX-28: the talk-key target, its marker and its toast (#73). It is wired
    // before the capture lifecycle, so a `hotkey:ptt` down updates the target first.
    const talkTarget = createAppTalkTarget(configName);
    wireTalkTarget(bus, talkTarget);
    // MOG-13a (#73): the MOG that captures an utterance owns its request and its answer.
    const owner = createRequestOwner({
      isOpen: (id) => talkTarget.isOpen(id),
      newId: () => crypto.randomUUID(),
      sendCard: sendOwnedCard,
    });
    // #14: the PTT and hands-free lifecycle. Every start goes through the T12 consent gate.
    const lifecycle = createCaptureLifecycle({
      control: ipcCaptureControl({ consent: components.T12 }),
      // TODO(#46): T4 transcribes with local Parakeet; the stub rejects, so nothing is routed yet.
      transcribe: async (seg) =>
        (await components.T4.transcribe(seg, { app: 'bottle', mog: seg.mogId, op: 'asr' })).text,
      // #59 (ADR 0053): the MOG of the key-down owns the utterance (MOG-13a); a dictation ends
      // at `insert_text`; a preview or a chip goes to T3.
      onUtterance: createUtteranceHandler({
        owner,
        // TODO(#112, #163): the router input needs the MOG's home profile from T7 and the
        // profile list. Bottle never picks a profile, so until then no utterance is routed.
        routerInput: () => undefined,
        // TODO(#60): `app.dictation.insert` is on after the user grants Accessibility; until the
        // dictation card reads it, a dictation goes to Copy.
        insertionOn: () => false,
        route: ipcUtteranceDeps(),
        // TODO(#142): T3 runs the 1.5 s preview or shows the chip. Until then nothing is sent.
        onAsk: (_r, owned) => owner.release(owned.utteranceId),
      }),
    });
    wireCaptureLifecycle(bus, lifecycle, createSystemClock().now);
    // CAP-16a, CAP-16b (#23, #476): the mic level goes only to the target MOG, and stops on
    // stop. The host sends the levels on the `capture_subscribe` `onLevel` Channel.
    const levels = new LevelRouter(sendMicLevel, createSystemClock().now);
    wireLevelStream(bus, levels);
    void subscribeLevels(levels);
    brain = { components, bus, talkTarget, ready: subscribeBrain(bus) };
  }
  return brain;
}
