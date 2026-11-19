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
  viewLabel,
} from '../bus';
import { createCaptureMood, wireCaptureMood } from '../capture-mood';
import { wireForget } from '../forget-wiring';
import { wireWipe } from '../wipe-wiring';
import { wireCliInstall, wireCliSettings } from '../cli-wiring';
import { cliPlatform } from '../../shared/cli-view';
import { wireConnectWorkspace } from '../connect-wiring';
import { sendHubSnapshot, wireHubMogs } from '../hub-mogs-wiring';
import {
  sendHubConversations,
  sendHubConversationsOpen,
  wireHubConversations,
  type HubConversationsWiring,
} from '../hub-conversations-wiring';
import {
  createCaptureLifecycle,
  ipcCaptureControl,
  wireCaptureLifecycle,
} from '../capture-lifecycle';
import { cardStackDeps, createCardStack, timeoutScheduler, type CardStack } from '../card-stack';
import { createSystemClock } from '../clock';
import { createDictationSink } from '../dictation-copy';
import { genieBlurbId } from '../../shared/genie-status';
import { createGenieDirectSend } from '../genie-direct';
import { wireGenieCard } from '../genie-card-wiring';
import { showHub } from '../hub-window';
import { asPttEvent } from '../ptt';
import { emitTo } from '@tauri-apps/api/event';
import { EVENTS } from '../../shared/ipc';
import { ipcGenieHistory } from '../genie-history';
import { createHomeProfiles, ipcHomeProfileDeps, ipcListProfiles } from '../home-profile';
import { LevelRouter, subscribeLevels, wireLevelStream } from '../level-stream';
import { createMoodPublisher, latencyTraceClock } from '../mood-publisher';
import { wireNotice } from '../notice-wiring';
import { wirePermissions } from '../permission-wiring';
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
import { IpcLocalMemoryStore } from './memory-store';
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
  /** #103: the hub Conversations list; `openConversations` opens it on one exchange. */
  readonly conversations: HubConversationsWiring;
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
    // #103 (#40): the hub Conversations list. The brain reads the saved exchanges of each MOG
    // (`mem.exchange.recent`) and sends them to the hub; the hub view runs no store statement.
    const conversations = wireHubConversations(bus, {
      store: new IpcLocalMemoryStore(),
      mogs: async () =>
        (await components.T1.list()).mogs.map((m) => ({ mogId: m.mogId, name: m.name })),
      send: sendHubConversations,
      sendOpen: sendHubConversationsOpen,
    });
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
    // #16 (W-02a, W-02c, W-02d): the hub Settings show the CLI path and version from T7.
    wireCliSettings(bus, { cliInfo: () => components.T7.cliInfo() });
    // #161 (W-17b, W-17c, U16): the first-run CLI install step. Only the hub's Install click
    // calls `cli_install {consent:true}`.
    wireCliInstall(bus, {
      cliInfo: () => components.T7.cliInfo(),
      install: () => call('cli_install', { consent: true }),
      platform: cliPlatform(globalThis.navigator?.userAgent ?? ''),
    });
    // #162 (W-07a..W-07i, W-15): "Connect a workspace" in the hub signs in through T7.
    wireConnectWorkspace(bus, {
      login: (host, name, opts) => components.T7.login(host, name, opts),
      cancel: (profile) => call('profile_login_cancel', { profile }),
      profiles: () => call('profiles_list', {}),
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
    /** #573: the newest Genie request of each MOG, for a double-click on the MOG. */
    const lastGenieOp = new Map<MogId, string>();
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
    // #142 (ADR 0086, ADR 0098), B1: T3 sends an ⌥G ask at once or shows the Ask Genie chip.
    // Nothing goes to Genie without ⌥G or a chip press.
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
      // #573 (ADR 0097): the hand-off seam sends the question straight to Genie One MCP through
      // T8 `submit` (`mcp_call` `genie_ask`, then the PollActor). No main agent, no retry.
      send: createGenieDirectSend({
        genie: components.T8,
        cardFor: stackFor,
        newOpId: () => crypto.randomUUID(),
        // #40 (AT-09): each request that ends saves one local exchange row through T10.
        onEnd: ipcGenieHistory(components.T10, () => void conversations.refresh()),
        // #573: the MOG shows the work (processing), then the answer (answering, success) or
        // the failure (error); the card shows next to the MOG when the outcome is on it.
        onPhase: (mogId, phase, opId) => {
          lastGenieOp.set(mogId, opId);
          captureMood.ask(mogId, phase);
        },
        showCard: (mogId) => call('window_card_show', { mogId }),
      }),
      // #557: an ask or a press with no home profile opens the picker; the user picks, then the
      // text goes.
      pickProfile: (mogId, then) => void pick.choose(mogId, then),
    });
    // ADR 0101 (#552, CAP-36, CAP-37): a missing Input Monitoring or Accessibility shows in the
    // hub and in the pop-up of the first MOG, with one control. Only its click asks the OS.
    wirePermissions(bus, {
      status: async (permission) => (await call('permission_status', { permission })).status,
      request: async (permission) => (await call('permission_request', { permission })).status,
      stackFor,
      showCard: (mogId) => call('window_card_show', { mogId }),
      hideCard: (mogId) => call('window_card_hide', { mogId }),
    });
    wireAskGate(bus, stackFor, {
      // #559, UX-19b: a cancel hides the card (`window_card_hide`, R11).
      hideCard: (mogId) => void call('window_card_hide', { mogId }).catch(() => undefined),
    });
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
    // #573: the ⌥G key-up shows the working mood at once, before the text is ready.
    bus.on(EVENTS['hotkey.ptt'].name, (p) => {
      const e = asPttEvent(p, 0);
      if (e?.phase === 'up' && e.ask === true) captureMood.ask(e.mogId, 'pending');
    });
    // #573: the transient Genie card (its timer holds while the pointer is on the MOG) and the
    // double-click on a MOG or its card, which opens hub Conversations.
    wireGenieCard(bus, {
      hideCard: (mogId) => void call('window_card_hide', { mogId }).catch(() => undefined),
      clearAnswer: (mogId) => stackFor(mogId).setExpanded(undefined),
      // The faded answer leaves the card for good: a card window that loads again (the R11
      // reaper) gets an empty Genie blurb, which shows nothing.
      forget: (mogId, requestId) => {
        const stack = stackFor(mogId);
        const id = genieBlurbId(requestId);
        stack.clearControls(`retry:${id}`);
        stack.add({ id, text: '', completed: true });
      },
      sendToCard: (mogId, event, payload) => {
        try {
          void emitTo(viewLabel('card', mogId), event, payload).catch(() => undefined);
        } catch {
          // Not a valid window label: nothing to hold or fade.
        }
      },
      lastOpId: (mogId) => lastGenieOp.get(mogId),
      // TODO(#103): the hub Conversations wiring (`hub-conversations-wiring.ts`) replaces the
      // emit below with `openConversations(requestId)`, which also sends it again when the hub
      // says it is ready.
      openConversations: (requestId) => {
        void showHub();
        void emitTo('hub', 'hub:conversations.open', { requestId }).catch(() => undefined);
      },
    });
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
    // #163 (RTR-15c, W-07d): the home profile of each MOG. #557: a press on Ask Genie with no
    // home profile opens the profile picker. The user picks; Bottle never picks.
    const homes = createHomeProfiles(ipcHomeProfileDeps());
    const insertionOn = () => true;
    const route = withOfflineFallback(ipcUtteranceDeps());
    const pick = createProfilePick({
      stackFor,
      showCard: (mogId) => call('window_card_show', { mogId }),
      listProfiles: ipcListProfiles,
      saveHome: (mogId, profile) => components.T1.edit(mogId, { home: profile }),
    });
    // #14: the PTT and hands-free lifecycle. Every start goes through the T12 consent gate.
    const lifecycle = createCaptureLifecycle({
      control: ipcCaptureControl({ consent: components.T12 }),
      // MB-08a: a refused start is never silent. The consent gate also shows the hub notice.
      onRefused: (e) => captureMood.refused(e.mogId, e.error),
      // #52 (ADR 0088): T4 runs the 8.3 chain over `stt_transcribe` (host route TODO(#514)).
      // A failure shows its fixed text on the owner's card, and no text is routed.
      transcribe: createLifecycleTranscriber(components.T4, sendOwnedCard),
      // #59 (ADR 0053, ADR 0098): the MOG of the key-down owns the utterance (MOG-13a); a
      // dictation ends at `insert_text`; an ⌥G ask or a chip goes to T3.
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
        // #142: T3 sends an ⌥G ask or shows the chip, and it releases the owner.
        // ADR 0098: with no ⌥G the utterance is dictated and gets the fallback chip.
        onAsk: (r, owned) => askGate.onAsk(r, owned),
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
    brain = { components, bus, talkTarget, conversations, ready: subscribeBrain(bus) };
  }
  return brain;
}
