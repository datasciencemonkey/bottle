/**
 * The registry of the hidden TS brain components T1-T15 of 4.3 (ARC-2, MOG-08b).
 * `startBrain()` constructs each component once; all domain state lives in these objects.
 */
import type { MogId } from '../../shared/interfaces';
import { call } from '../../shared/ipc';
import { createAskGate, wireAskGate } from '../ask-gate';
import {
  BrainBus,
  onViewInput,
  sendHubNotice,
  sendMicLevel,
  sendMogViewState,
  subscribeBrain,
} from '../bus';
import { createCaptureMood, wireCaptureMood } from '../capture-mood';
import { wireForget } from '../forget-wiring';
import { wireWipe } from '../wipe-wiring';
import { sendHubSnapshot, wireHubMogs } from '../hub-mogs-wiring';
import {
  createCaptureLifecycle,
  ipcCaptureControl,
  wireCaptureLifecycle,
} from '../capture-lifecycle';
import { cardStackDeps, createCardStack, timeoutScheduler, type CardStack } from '../card-stack';
import { createSystemClock } from '../clock';
import { createDictationSink } from '../dictation-copy';
import { createHomeProfiles, ipcHomeProfileDeps, ipcListProfiles } from '../home-profile';
import { LevelRouter, subscribeLevels, wireLevelStream } from '../level-stream';
import { createMoodPublisher, latencyTraceClock } from '../mood-publisher';
import { wireNotice } from '../notice-wiring';
import { withOfflineFallback } from '../offline-intent';
import { createProfilePick } from '../profile-pick';
import { createRequestOwner, sendOwnedCard } from '../request-owner';
import { wireTalkNoMog } from '../talk-no-mog';
import { configName, createAppTalkTarget, wireTalkTarget, type TalkTarget } from '../talk-target';
import { createLifecycleTranscriber } from '../stt-wiring';
import { ipcUtteranceDeps } from '../utterance-route';
import { createUtteranceHandler } from '../utterance-wiring';
import { createAgentBridge } from './agent-bridge';
import { createCapturePolicy } from './capture-policy';
import { createCleanupProvider } from './cleanup-provider';
import { createConsentService } from './consent-service';
import { createConversationManager } from './conversation-manager';
import { createGenieBridge } from './genie-bridge';
import { ForgetFlow, isForgetMemory } from './forget-flow';
import { GenieOneDeleteUtility } from './genie-one-delete';
import { createMemoryService } from './memory-service';
import { createMogManager } from './mog-manager';
import { createMoodMapper } from './mood-mapper';
import { createOperationTracker } from './operation-tracker';
import { createPageService } from './page-service';
import { createProfileService } from './profile-service';
import { readAdminPolicy, retentionSettings } from './retention-settings';
import { createRecoveryManager } from './recovery-manager';
import { createRouterContextGate } from './router-context-gate';
import { createSttProvider } from './stt-provider';
import { WipeFlow } from './wipe-flow';

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
    if (isForgetMemory(components.T10)) {
      wireForget(bus, {
        flow: new ForgetFlow({ memory: components.T10, genieOne: new GenieOneDeleteUtility() }),
      });
    }
    // #124 (MEM-33a, SEC-15c, U9): Settings -> Wipe. The hub dialog runs the R14 Wipe only on the
    // user's confirm click. TODO(#525): the default core sends `mem_wipe` once it is in the
    // contract; until then a Wipe fails safe and the dialog says that nothing changed.
    wireWipe(bus, {
      flow: new WipeFlow({ genieOne: new GenieOneDeleteUtility() }),
      settings: async () => retentionSettings(await readAdminPolicy()),
    });
    // MB-06a, MB-06b, UX-28: the talk-key target, its marker and its toast (#73). It is wired
    // before the capture lifecycle, so a `hotkey:ptt` down updates the target first.
    const talkTarget = createAppTalkTarget(configName);
    wireTalkTarget(bus, talkTarget);
    // #562 (MB-06a): a press with no open MOG starts the last-used MOG, or the hub says
    // "Create a MOG to start".
    wireTalkNoMog(bus, {
      manager: components.T1,
      lastUsed: () => talkTarget.lastUsed(),
      openWindow: (mogId) => call('window_mog_open', { mogId }),
      notify: (notice) => sendHubSnapshot(components.T1, notice),
    });
    // MOG-13a (#73): the MOG that captures an utterance owns its request and its answer.
    const owner = createRequestOwner({
      isOpen: (id) => talkTarget.isOpen(id),
      newId: () => crypto.randomUUID(),
      sendCard: sendOwnedCard,
    });
    // #498: the card stack of each MOG, made on first use. TODO(#374): drop a stack when its MOG
    // closes.
    const cardClock = createSystemClock();
    const stacks = new Map<MogId, CardStack>();
    const stackFor = (mogId: MogId): CardStack => {
      let s = stacks.get(mogId);
      if (!s)
        stacks.set(
          mogId,
          (s = createCardStack(mogId, {
            ...cardStackDeps(mogId, cardClock),
            // #142: a click on a live Ask Genie chip (UX-20c). #163: a click on a pick card chip.
            onChip: (a) => {
              askGate.chip(mogId, a);
              void pick.chip(mogId, a);
            },
          })),
        );
      return s;
    };
    // #142 (ADR 0086), B1: T3 runs the 1.5 s preview or shows the Ask Genie chip. Nothing goes
    // to the main agent, and so to Genie, without the preview or a chip click.
    const askGate = createAskGate({
      clock: cardClock,
      schedule: timeoutScheduler(cardClock),
      cardFor: stackFor,
      showCard: (mogId) => call('window_card_show', { mogId }),
      owner,
      isOpen: (id) => talkTarget.isOpen(id),
      // #163: the target is the MOG's home profile (no hop yet, TODO(#164)). With none,
      // nothing is sent: the pick card asks the user to pick one.
      target: (mogId) => homes.home(mogId),
      // The hand-off seam: T15 `agent_send`. The core approves this one question (#141).
      send: (t) => components.T15.send(t),
    });
    // TODO(#374): the card-hide signal calls `askGate.cardClosed`; none reaches the brain yet.
    wireAskGate(bus, askGate, stackFor);
    // #551 (UX-01a, UX-05b, UX-07): the MOG mood follows `capture:state`, and a refused start
    // shows its reason on the MOG (MB-08a). It is wired before the capture lifecycle, so the
    // `listening` state goes out in the same task as the `capture:state` item.
    const moodClock = createSystemClock();
    const captureMood = createCaptureMood({
      publisher: createMoodPublisher({
        clock: moodClock,
        send: (s) => sendMogViewState(s.mogId, s).catch(() => undefined),
        scheduler: { at: timeoutScheduler(moodClock) },
        traceNow: latencyTraceClock(import.meta.env.VITE_BOTTLE_LATENCY_TRACE),
      }),
      clock: moodClock,
      scheduler: { at: timeoutScheduler(moodClock) },
    });
    wireCaptureMood(bus, captureMood);
    // #498 (ADR 0091): the Copy card of a dictation. The card is shown first, then the text goes
    // on its stack. If the card cannot show, the text goes to the hub only. No main agent and no
    // Genie take part (RTR-01c).
    const dictation = createDictationSink({
      stackFor,
      showCard: (mogId) => call('window_card_show', { mogId }),
      sendHubNotice,
    });
    // #507: a view sends `ready` after it listens. The replay is keyed on `from`, which the host
    // sets, and it sends state only: it takes no user action.
    onViewInput(bus, (input) => {
      if (input.kind !== 'ready') return;
      if (input.from === 'hub') void dictation.hubReady();
      else if (input.from === `card-${input.mogId}`) dictation.cardReady(input.mogId);
    });
    // #163 (RTR-15c, W-07d): the home profile of each MOG, and the "Pick a profile to ask
    // Genie" card for a question with no home profile. The user picks; Bottle never picks.
    const homes = createHomeProfiles(ipcHomeProfileDeps());
    const insertionOn = () => true;
    const route = withOfflineFallback(ipcUtteranceDeps());
    const pick = createProfilePick({
      stackFor,
      showCard: (mogId) => call('window_card_show', { mogId }),
      listProfiles: ipcListProfiles,
      saveHome: (mogId, profile) => components.T1.edit(mogId, { home: profile }),
      insertionOn,
      insertText: route.insertText,
    });
    // #14: the PTT and hands-free lifecycle. Every start goes through the T12 consent gate.
    const lifecycle = createCaptureLifecycle({
      control: ipcCaptureControl({ consent: components.T12 }),
      // MB-08a: a refused start is never silent. The consent gate also shows the hub notice.
      onRefused: (e) => captureMood.refused(e.mogId, e.error),
      // #52 (ADR 0088): T4 runs the 8.3 chain over `stt_transcribe` (host route TODO(#514)).
      // A failure shows its fixed text on the owner's card, and no text is routed.
      transcribe: createLifecycleTranscriber(components.T4, sendOwnedCard),
      // #59 (ADR 0053): the MOG of the key-down owns the utterance (MOG-13a); a dictation ends
      // at `insert_text`; a preview or a chip goes to T3.
      onUtterance: createUtteranceHandler({
        owner,
        // #163: every utterance is routed. With no home profile, or when the router model call
        // fails, the RTR-15c offline heuristic decides.
        routerInput: (mogId) => homes.routerInput(mogId),
        // MOG-06a, MOG-06d (#551): insertion is on. `insert_text` checks Accessibility before it
        // touches the clipboard and answers `no_permission`; the route turns that, or a throw,
        // into `insert_failed` with the text, so the dictation goes to Copy and is not lost.
        // TODO(#60): the Appendix F `app.dictation.insert` setting; TODO(#552) is parked.
        insertionOn,
        route,
        // #142: T3 runs the 1.5 s preview or shows the chip, and it releases the owner.
        // #163: with no home profile an ask shows the pick card instead; it never sends (B1).
        onAsk: (r, owned) => {
          if (homes.home(owned.mogId) !== undefined) return askGate.onAsk(r, owned);
          void pick.showNoProfile(r, owned);
          owner.release(owned.utteranceId);
        },
        // #498, MOG-05b, MOG-06d: a Copy or a failed insertion reaches the Copy card.
        onDone: (r, owned) => void dictation.onDone(r, owned).catch(() => undefined),
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
