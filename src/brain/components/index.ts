/**
 * The registry of the hidden TS brain components T1-T15 of 4.3 (ARC-2, MOG-08b).
 * `startBrain()` constructs each component once; all domain state lives in these objects.
 */
import type { AgentTurn, CaptureState, MogId, SegmentRef } from '../../shared/interfaces';
import { createBrainLog } from '../log';
import { createArrivalGate, noPasteOnEmpty } from '../arrival-gate';
import { call } from '../../shared/ipc';
import { capQuestion, createAskGate, wireAskGate } from '../ask-gate';
import { sanitizeQuestion } from '../question-sanitizer';
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
import { clearingDictationWords } from '../wipe-dictation-words';
import { wireWipe } from '../wipe-wiring';
import { wireCliInstall, wireCliSettings } from '../cli-wiring';
import { cliPlatform } from '../../shared/cli-view';
import { wireConnectWorkspace } from '../connect-wiring';
import { recordingProfile } from '../recording-profile';
import {
  createWorkspaces,
  ipcWorkspacesDeps,
  lampOnlyManager,
  wireWorkspaces,
} from '../workspaces';
import { createHeldNote, ipcRecordingApi, wireRecording } from '../recording-wiring';
import { wireHubMogs } from '../hub-mogs-wiring';
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
import {
  HUB_THREAD_ACCESS_EVENT,
  HUB_THREAD_STATUS_EVENT,
  type ThreadAccess,
  type ThreadStatus,
} from '../../shared/hub-conversations';
import { createGenieDirectSend } from '../genie-direct';
import { createAgentCard, wireAgentChips } from '../agent-card';
import {
  asTurnEvent,
  createAgentClient,
  ipcAgentApi,
  uuidOfTurn,
  type AgentClient,
} from '../agent-run';
import { wireOrchestrator, ipcOrchestratorDeps } from '../orchestrator';
import { createAskRouter, createFollowUpWindow, threadReplyMog } from '../agent-route';
import {
  clearingAgentPanel,
  sendHubAgent,
  wireHubAgent,
  type HubAgentWiring,
} from '../hub-agent-wiring';
import { MOG_PALETTE } from '../../shared/mog-palette';
import { asLinkOpenClick, bareHost } from '../../shared/link-trust';
import { createGenieFollowup } from '../genie-followup';
import { wireGenieCard } from '../genie-card-wiring';
import { wireHubMemory } from '../hub-memory-wiring';
import { createRememberFlow, sendMemorySaved } from '../remember-flow';
import { wireLinkOpen } from '../link-open-wiring';
import { createAppMogContextMenu } from '../mog-context-menu';
import { createThreadFocus, routeAskToFocusedThread, wireThreadFollowup } from '../thread-focus';
import { hubExists, isHubFocused, showHub } from '../hub-window';
import { asPttEvent } from '../ptt';
import { createRecordToggle } from '../record-toggle';
import { emitTo } from '@tauri-apps/api/event';
import { EVENTS } from '../../shared/ipc';
import { ipcGenieHistory, MOG_BACKEND_KIND } from '../genie-history';
import { createHomeProfiles, ipcHomeProfileDeps, ipcListProfiles } from '../home-profile';
import { LevelRouter, subscribeLevels, wireLevelStream } from '../level-stream';
import { wireMogLook } from '../mog-look-relay';
import { createMoodPublisher, latencyTraceClock } from '../mood-publisher';
import { sendHubNotice as sendLegalNotice, wireNotice } from '../notice-wiring';
import { storeGate } from '../store-gate';
import { isTauriMac, wireStoreGate } from '../store-wiring';
import { STORE_EVENT } from '../../shared/store-view';
import { wirePermissions } from '../permission-wiring';
import { withDictationCleanup } from '../dictation-cleanup';
import { offlineDecision, withOfflineFallback } from '../offline-intent';
import { createProfilePick } from '../profile-pick';
import { createRequestOwner, sendOwnedCard } from '../request-owner';
import { wireTalkNoMog } from '../talk-no-mog';
import { createAppTalkTarget, wireTalkTarget, type TalkTarget } from '../talk-target';
import { createDictationCorrector } from '../../shared/dictation-vocab';
import { createLifecycleTranscriber } from '../stt-wiring';
import { ipcUtteranceDeps } from '../utterance-route';
import { createUtteranceHandler } from '../utterance-wiring';
import { createAgentBridge } from './agent-bridge';
import { createCapturePolicy } from './capture-policy';
import { createConsentService } from './consent-service';
import { createConversationManager } from './conversation-manager';
import { createGenieBridge } from './genie-bridge';
import { ForgetFlow, isForgetMemory } from './forget-flow';
import { GenieOneDeleteUtility } from './genie-one-delete';
import { createMemoryService } from './memory-service';
import { LocalMemoryService } from './memory-service';
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

/** T1..T15 in 4.3 table order. T5 (CleanupProvider) is removed: `withDictationCleanup` runs. */
export const BRAIN_COMPONENTS = [
  { id: 'T1', name: 'MogManager', factory: () => createMogManager() },
  { id: 'T2', name: 'OperationTracker', factory: () => createOperationTracker() },
  { id: 'T3', name: 'CapturePolicy', factory: () => createCapturePolicy() },
  { id: 'T4', name: 'SttProvider', factory: () => createSttProvider() },
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
    // ADR 0105 (MEM-08b): the local store opens once, on first use. On macOS the hub first says
    // why macOS asks for the password. A refusal turns history off until Try again.
    wireStoreGate(bus, storeGate, {
      explain: isTauriMac(),
      onRetry: () =>
        void components.T12.noticeState()
          .then(sendLegalNotice)
          .catch(() => undefined),
    });
    // MB-08a, MB-08b: the hub notice talks to T12 (#27).
    wireNotice(bus, components.T12);
    // #26: the hub MOG list and actions (MB-01). #ui-4 (review #603, finding 3): a successful
    // edit or colour pick re-reads that MOG's identity and pushes it to its open MOG and card
    // (`talkTarget.reidentify`, below), so neither keeps a stale name or colour until restart.
    wireHubMogs(
      bus,
      components.T1,
      undefined,
      (mogId) => talkTarget.reidentify(mogId),
      () => workspaces.lampMogId(),
    );
    // ADR 0117: the one MOG with workspaces. The brain keeps the last `WorkspaceList`, sends the
    // hub Settings → Workspace list and runs the hub workspace actions. A change of the home
    // workspace, or of its name or colour, pushes the new identity to the lamp.
    const workspaces = createWorkspaces(
      ipcWorkspacesDeps(() => {
        void workspaces.lampMogId().then((id) => id && talkTarget.reidentify(id));
        // The orchestrator runs on the home workspace: a new home can change its status.
        void orchestrator.refresh();
      }),
    );
    wireWorkspaces(bus, workspaces);
    // #103 (#40): the hub Conversations list. The brain reads the saved exchanges of each MOG
    // (`mem.exchange.recent`) and sends them to the hub; the hub view runs no store statement.
    const agentRef: {
      client?: AgentClient;
      /** A follow-up in an agent thread (the hub composer, or ⌥G on the thread). */
      send?: (
        threadId: string,
        mogId: string | undefined,
        question: string,
        ticket?: string,
      ) => void;
    } = {};
    const focusRef: { current?: ReturnType<typeof createThreadFocus> } = {};
    const conversations = wireHubConversations(bus, {
      store: new IpcLocalMemoryStore(),
      // ADR 0121: the agent threads. The chips come from the workspaces list; the turns in memory
      // win over the stored turn with the same UUID; a Delete makes the client forget the turns.
      chipOf: (profile) => {
        const w = workspaces.catalog().find((c) => c.profile === profile);
        return {
          name: w?.name ?? profile,
          colorVariant: w?.colorVariant ?? MOG_PALETTE[0]?.id ?? '',
        };
      },
      memoryTurns: () => agentRef.client?.turns() ?? [],
      forgetThread: (threadId) => agentRef.client?.forgetThread(threadId),
      // A deleted Genie thread is no follow-up target: ⌥G then asks as usual.
      forgetConversation: (id) => {
        const a = focusRef.current?.active();
        if (a !== undefined && 'conversationId' in a && a.conversationId === id)
          focusRef.current?.set(undefined);
      },
      mogs: async () =>
        (await components.T1.list()).mogs.map((m) => ({
          mogId: m.mogId,
          name: m.name,
          colorVariant: m.colorVariant,
        })),
      send: sendHubConversations,
      sendOpen: sendHubConversationsOpen,
      hubPresent: () => hubExists(),
    });
    // The hub asks for its list with `conversations.ready`, and that request is lost while this
    // brain has no Channel yet (the core refuses `view_emit`) or while the store is locked. So
    // the brain also sends the list by itself when the store opens (ADR 0105), and once it
    // listens (below).
    storeGate.onOpen(() => void conversations.push());
    // Local memory slice 1 (ADR 0112, ADR 0113): the hub Memory folders, the card "remember" and
    // the voice "remember". The host mints the tickets; the brain only passes them on.
    const memorySvc = components.T10 instanceof LocalMemoryService ? components.T10 : undefined;
    const hubMemory = memorySvc
      ? wireHubMemory(bus, {
          memory: memorySvc,
          mogs: async () =>
            (await components.T1.list()).mogs.map((m) => ({
              mogId: m.mogId,
              name: m.name,
              colorVariant: m.colorVariant,
              backendKind: MOG_BACKEND_KIND,
            })),
          hubPresent: () => hubExists(),
        })
      : undefined;
    if (hubMemory) storeGate.onOpen(() => void hubMemory.push());
    const rememberFlow = memorySvc
      ? createRememberFlow({
          memory: memorySvc,
          store: memorySvc.store,
          send: sendMemorySaved,
          showCard: (mogId) => call('window_card_show', { mogId }),
          ensureMog: async (mogId) => {
            const m = (await components.T1.list()).mogs.find((x) => x.mogId === mogId);
            if (m)
              await memorySvc.ensureMog({ mogId, name: m.name, backendKind: MOG_BACKEND_KIND });
          },
          // A save or an Undo changes the store: the hub list is stale.
          onChanged: () => void hubMemory?.push(),
        })
      : undefined;
    // Without a memory service the card still hears back: nothing is saved.
    const memoryUnavailable = (mogId: MogId) =>
      void sendMemorySaved(mogId, { mogId, outcome: 'failed', undoMs: 0 }).catch(() => undefined);
    // #40 (AT-09): one local-history writer, shared by a direct ask and a thread follow-up, so
    // each saved exchange refreshes the hub Conversations list the same way.
    // ADR 0117: the card stays on the lamp; the row goes to the data MOG of the answering profile.
    const recordGenieExchange = ipcGenieHistory(
      components.T10,
      () => void conversations.refresh(),
      (profile) => workspaces.dataMogId(profile),
    );
    // genie-followup (#145, #103): a follow-up from the thread composer or from a ⌥G press while
    // the hub is focused on a thread (item 4, wired below with `insertionOn`/`onAsk`). It never
    // touches a MOG card: the status goes to the hub, and the MOG and the profile that own the
    // thread come from the stored exchanges (ARC-2), because the composer itself never carries a
    // `mogId`, and its own copy of the profile is for display only.
    const threadFocus = createThreadFocus();
    focusRef.current = threadFocus;
    const followup = createGenieFollowup({
      genie: components.T8,
      findThread: (conversationId) => conversations.findThread(conversationId),
      sendStatus: (status: ThreadStatus) => {
        void emitTo('hub', HUB_THREAD_STATUS_EVENT, status).catch(() => undefined);
      },
      profileStatus: (profile) => call('profile_status', { profile }),
      onEnd: recordGenieExchange,
      newOpId: () => crypto.randomUUID(),
    });
    wireThreadFollowup(bus, {
      focus: threadFocus,
      followup,
      sendAccess: (access: ThreadAccess) => {
        void emitTo('hub', HUB_THREAD_ACCESS_EVENT, access).catch(() => undefined);
      },
      // ADR 0121: the composer of an agent thread. The host minted the Ask ticket for the click.
      agentSend: (a, ticket) => agentRef.send?.(a.threadId, a.mogId, a.question, ticket),
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
    // Slice 2a: every Wipe scope ends the agent session, so the panel content goes too.
    const agentClear: { fn?: () => void; fence?: () => () => void } = {};
    wireWipe(bus, {
      flow: clearingAgentPanel(
        clearingDictationWords(new WipeFlow({ genieOne: new GenieOneDeleteUtility() })),
        () => agentClear.fn?.(),
        () => agentClear.fence?.() ?? (() => undefined),
        () => void createBrainLog().event('agent.turn', { ok: 0 }),
      ),
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
      // ADR 0117: the first Connect with no home makes that profile home. A later one stays off.
      onConnected: (profile) => workspaces.connected(profile),
    });
    // MB-06a, MB-06b, UX-28: the talk-key target, its marker and its toast (#73). It is wired
    // before the capture lifecycle, so a `hotkey:ptt` down updates the target first.
    // ADR 0117: the target is always the lamp, which shows the home workspace's name and colour.
    const talkTarget = createAppTalkTarget((mogId) => workspaces.homeIdentity(mogId));
    wireTalkTarget(bus, talkTarget);
    // #562 (MB-06a): a press with no open MOG starts the last-used MOG, or the hub says
    // "Create a MOG to start".
    wireTalkNoMog(bus, {
      // ADR 0117: the press starts the lamp only.
      manager: lampOnlyManager(components.T1, () => workspaces.lampMogId()),
      lastUsed: () => talkTarget.lastUsed(),
      openWindow: (mogId) => call('window_mog_open', { mogId }),
      showHub: () => showHub(),
      // ADR 0117: the hub lists workspaces, not MOGs. A notice goes with the workspace list; the
      // push with no notice first makes each notice new to the hub, so it shows each time.
      notify: async (notice) => {
        await workspaces.refresh();
        if (notice) await workspaces.refresh({ notice });
      },
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
    // UX slice 1: no Ask Genie arrival opens the card while the talk key is held (or hands-free
    // listens); it applies 1.2 s after the paste of the hold.
    const arrivalLog = createBrainLog();
    const arrivalGate = createArrivalGate({
      clock: cardClock,
      schedule: timeoutScheduler(cardClock),
      // A held arrival that throws: a fixed code, no content (SEC-16c).
      onFault: () => void arrivalLog.event('card.render', { ok: 0 }),
    });
    bus.on(EVENTS['hotkey.ptt'].name, (p) => {
      const e = asPttEvent(p, 0);
      if (e) arrivalGate.onPtt(e);
    });
    bus.on(EVENTS['capture.state'].name, (p) => {
      if (typeof p === 'object' && p !== null) arrivalGate.onCaptureState(p as CaptureState);
    });
    bus.on(EVENTS['capture.segment'].name, (p) => {
      const seg = p as Partial<SegmentRef> | null;
      if (seg?.final === true && typeof seg.segmentId === 'string')
        arrivalGate.segmentFinal({ segmentId: seg.segmentId, leaseId: seg.leaseId });
    });
    // Esc in a card stops hands-free and drops its audio: no paste follows.
    onViewInput(bus, (input) => {
      if (input.kind === 'cancel' && input.from === `card-${input.mogId}`)
        arrivalGate.handsFreeDiscarded();
    });
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
    // Slice 2a: the direct home path of ADR 0097 (#573). The router below sends a question here
    // unless the orchestrator model takes it (ADR 0122, R-1 to R-15).
    // #573 (ADR 0097): the hand-off seam sends the question straight to Genie One MCP through
    // T8 `submit` (`mcp_call` `genie_ask`, then the PollActor). No main agent, no retry.
    const directSend = createGenieDirectSend({
      genie: components.T8,
      cardFor: stackFor,
      newOpId: () => crypto.randomUUID(),
      // #40 (AT-09): each request that ends saves one local exchange row through T10.
      onEnd: recordGenieExchange,
      // #573: the MOG shows the work (processing), then the answer (answering, success) or
      // the failure (error); the card shows next to the MOG when the outcome is on it.
      onPhase: (mogId, phase, opId) => {
        lastGenieOp.set(mogId, opId);
        captureMood.ask(mogId, phase);
      },
      showCard: (mogId) => call('window_card_show', { mogId }),
      arrivals: arrivalGate,
      // The card's "Change workspace…": the same path as the right-click "Change home
      // workspace…": show the hub on that MOG's Edit form.
      changeWorkspace: () => {
        void showHub();
        void workspaces.showSettings();
      },
      hostOf: async (profile) => (await ipcListProfiles()).find((p) => p.name === profile)?.host,
    });
    // Slice 2a: the one live orchestrator session. The client turns `agent:turn` and `agent:session`
    // events into the card of the turn and the history rows; the follow-up window is R-13.
    const followUp = createFollowUpWindow(cardClock);
    const agentLog = createBrainLog();
    const agentCard = createAgentCard({
      stackFor,
      showCard: (mogId) => call('window_card_show', { mogId }),
      arrivals: arrivalGate,
      onCardOpen: (open) => followUp.card(open),
      onFault: () => void agentLog.event('card.render', { ok: 0 }),
    });
    const routeNames = () => {
      const cat = workspaces.catalog();
      const home = cat.find((c) => c.isHome);
      // ADR 0122: the orchestrator runs on the home workspace, always.
      const name = home?.name ?? 'your home workspace';
      return { agent: name, home: name };
    };
    /** A one-line pop-up (queue full, "Not sent."): it takes the card and waits for the hold gate. */
    const showNote = (mogId: MogId, text: string) => {
      // With an agent card open the note is one line inside it; the live and waiting turns stay.
      if (agentCard.note(mogId, text)) return;
      arrivalGate.whenClear(() => {
        stackFor(mogId).add({ id: `agent-note:${crypto.randomUUID()}`, text, completed: true });
        void call('window_card_show', { mogId }).catch(() => undefined);
      });
    };
    const agentClient = createAgentClient({
      api: ipcAgentApi(),
      chipOf: (profile) => {
        const w = workspaces.catalog().find((c) => c.profile === profile);
        return {
          name: w?.name ?? profile,
          colorVariant: w?.colorVariant ?? MOG_PALETTE[0]?.id ?? '',
        };
      },
      names: routeNames,
      homeProfile: () => workspaces.home(),
      hostOf: async (profile) => (await ipcListProfiles()).find((p) => p.name === profile)?.host,
      newId: () => crypto.randomUUID(),
      wallNow: () => Date.now(),
      ...(recordGenieExchange ? { recordCall: recordGenieExchange } : {}),
      // The direct answer takes the card: the agent card must not cover it.
      // A fallback takes the card for its own turn only; a refused submit has no turn: the
      // direct ask takes the card from every agent turn of this MOG.
      direct: (turn, notice, turnId, isStale) => {
        agentCard.takeover(turn.mogId, turnId);
        // The fallback row joins its turn (ADR 0121): it is the home part of the turn.
        return directSend(turn, {
          notice,
          ...(turnId !== undefined ? { splitGroupId: uuidOfTurn(turnId) } : {}),
          ...(turnId !== undefined && isStale ? { isStale } : {}),
        });
      },
      showTurn: (mogId, view) => agentCard.show(mogId, view),
      dropTurn: (mogId, turnId) => agentCard.drop(mogId, turnId),
      note: showNote,
      phase: (mogId, phase) => captureMood.ask(mogId, phase),
      followUp,
      onChange: () => {
        void hubRef.current?.push();
        // A Delete that waited for a turn to settle goes now.
        void conversations.retryDeferred();
      },
      log: agentLog,
    });
    agentRef.client = agentClient;
    agentRef.send = (threadId, mogId, raw, ticket) => {
      void (async () => {
        // The hub names the MOG (the host bound the ticket to it); ⌥G takes the MOG of the thread.
        // Either way it must be a defined MOG.
        const known = conversations.threadMog(threadId) ?? agentClient.threadMog(threadId);
        // The hub's MOG must own the thread: a mismatch writes a turn under one MOG with the
        // root of another.
        const owner = threadReplyMog(mogId, known);
        if (owner === 'refused')
          return mogId !== undefined ? showNote(mogId, 'Not sent.') : undefined;
        const target = owner.mogId;
        const clean = sanitizeQuestion(raw);
        if (target === undefined) return;
        if (!(await components.T1.list()).mogs.some((m) => m.mogId === target)) return;
        if (clean === '') return showNote(target, 'Not sent.');
        const home = await workspaces.home();
        if (home === undefined) return showNote(target, 'Not sent.');
        const question = capQuestion(clean).text;
        const turn: AgentTurn = {
          mogId: target,
          question,
          decision: offlineDecision(question),
          recall: [],
          target: home,
        };
        // A reply in the thread of the open session is a follow-up; else it starts a session.
        const mode = agentClient.inSession(threadId) ? 'follow_up' : 'ask';
        await agentClient.submit(turn, mode, ticket, threadId);
      })().catch(() => (mogId !== undefined ? showNote(mogId, 'Not sent.') : undefined));
    };
    const hubRef: { current?: HubAgentWiring } = {};
    agentClear.fn = () => {
      agentClient.clear();
      void hubRef.current?.push();
    };
    agentClear.fence = () => agentClient.fence();
    bus.on(EVENTS['agent.turn'].name, (p) => {
      agentClient.onTurnEvent(p);
      // M3: a turn that ends failed, signed out or offline may mean the model is not ready.
      const e = asTurnEvent(p);
      if (e?.kind === 'ended') orchestrator.turnEnded(e.outcome);
    });
    bus.on(EVENTS['agent.session'].name, (p) => agentClient.onSessionEvent(p));
    const askRouter = createAskRouter({
      context: () => ({ orchestrator: orchestrator.status() }),
      names: routeNames,
      followUp,
      agent: agentClient,
      direct: directSend,
      closeAgentCard: (mogId) => agentCard.takeover(mogId),
      refreshOrchestrator: () => void orchestrator.refresh(),
    });
    // ADR 0122: the orchestrator model and the dictation switch in Settings → Workspace, and the
    // status that the router reads.
    const orchestrator = wireOrchestrator(
      bus,
      ipcOrchestratorDeps({
        settings: () => workspaces.last()?.agent,
        homeName: () => workspaces.catalog().find((c) => c.isHome)?.name ?? null,
        write: (args) => workspaces.writeSettings(args),
      }),
    );
    // The agent turns in memory (live, waiting, earlier) go to hub Conversations (ADR 0121).
    hubRef.current = wireHubAgent(bus, {
      client: agentClient,
      send: sendHubAgent,
      openSettings: () => {
        void showHub();
        void workspaces.showSettings();
      },
    });
    // The agent chips of the card: Stop, Open in Conversations, Open Settings.
    wireAgentChips(bus, {
      stop: (turnId) => void agentClient.stop(turnId),
      // ADR 0121: the thread of the turn opens in hub Conversations, also before it is stored.
      openAgent: (turnId) => {
        void showHub();
        const threadId = agentClient.threadOf(turnId);
        if (threadId !== undefined) void conversations.openThread(threadId);
      },
      openSettings: () => {
        void showHub();
        void workspaces.showSettings();
      },
      cardDone: (mogId) => agentCard.close(mogId),
    });
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
      // The router sends the question to the orchestrator model or to the direct home path
      // (ADR 0097).
      send: (turn) => askRouter.send(turn),
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
    // ADR 0103, ADR 0117, recording-profile.ts: the one workspace of a call recording is the
    // home workspace, shared by every cloud-bound recording command and the record key (⌥R).
    // Bottle never picks one: with no home it is `no_profile`.
    const recordingHomeProfile = () => recordingProfile({ home: () => workspaces.home() });
    // ADR 0103: call recording. The hub sends clicks; the brain calls the `recording_*`
    // commands, relays `recording.state` and `recording.job` to the hub, and shows REC and the
    // answer-ready moment on the MOG through the one mood publisher. The core checks the knob
    // again before any capture. TODO(#95): the short OS notice when the notes are ready, once
    // the notification host has an IPC command.
    const recordingApi = ipcRecordingApi(bus, recordingHomeProfile);
    // ADR 0118: the first hold in a recording shows "Your call can still hear you." once.
    const heldNote = createHeldNote({ api: recordingApi, mood: captureMood });
    wireRecording(bus, {
      api: recordingApi,
      platform:
        cliPlatform(globalThis.navigator?.userAgent ?? '') === 'windows' ? 'windows' : 'mac',
      mood: {
        callRecording: (on) => {
          const id = talkTarget.lastUsed();
          captureMood.callRecording(on, id === undefined ? [] : [id]);
        },
        ready: () => captureMood.recordingReady(),
      },
    });
    // #573: the ⌥G key-up shows the working mood at once, before the text is ready.
    bus.on(EVENTS['hotkey.ptt'].name, (p) => {
      const e = asPttEvent(p, 0);
      if (e?.phase === 'up' && e.ask === true) captureMood.ask(e.mogId, 'pending');
    });
    // #523: a link click in the card or in hub Conversations. The brain finds the workspace from
    // its own data (the MOG's home profile, or the exchange the hub lists), checks the URL again
    // and asks Rust to open it (Rust checks it a third time).
    wireLinkOpen(bus, {
      resolve: async (input, requestId) => {
        // Slice 2a (L3): a link in an agent answer opens under the workspace whose host the URL
        // names (not under the workspace that answered). It must be the host of a workspace that
        // the user switched on; `trustedUrl` and Rust check it again. The click names a
        // host; it counts only if it is the host of a workspace that the user switched on (the
        // trust rule then checks the URL against that host again).
        const ofAgentAnswer = async (): Promise<{ profile: string; host: string } | undefined> => {
          const click = asLinkOpenClick(input.payload);
          if (!click) return undefined;
          let host = '';
          try {
            host = new URL(click.url).host.toLowerCase();
          } catch {
            return undefined;
          }
          const on = new Set(workspaces.catalog().map((c) => c.profile));
          const p = (await ipcListProfiles()).find(
            (x) => on.has(x.name) && bareHost(x.host) === host,
          );
          return p ? { profile: p.name, host: p.host } : undefined;
        };
        if (input.from === 'hub') {
          const ws = requestId ? conversations.workspaceOf(requestId) : undefined;
          if (ws && ws.host !== '') return { profile: ws.profile, host: ws.host };
          return requestId ? undefined : ofAgentAnswer();
        }
        if (agentCard.shown(input.mogId)) {
          const w = await ofAgentAnswer();
          if (w) return w;
        }
        const profile = homes.home(input.mogId);
        if (!profile) return undefined;
        const host = (await ipcListProfiles()).find((p) => p.name === profile)?.host;
        return host ? { profile, host } : undefined;
      },
      open: (args) => call('link_open', args),
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
      isWorking: (mogId, requestId) => directSend.isWorking(mogId, requestId),
      closePending: (mogId, requestId) => directSend.close(mogId, requestId),
      lastOpId: (mogId) => lastGenieOp.get(mogId),
      // #103: show the hub, then open Conversations on this exchange. The wiring holds the open
      // until the hub acks it or asks for the list, so a hub that was not listening still gets it.
      openConversations: (requestId) => {
        void showHub();
        void conversations.openConversations(requestId);
      },
      // #ui-4: the MOG right-click menu's Close runs the same stop as the hub's Stop button
      // (the window closes and the mic lease releases; the MOG stays defined). "Change home
      // workspace…" shows the hub and opens that MOG's own Edit form, where the home field is
      // (review #603, MEDIUM finding: it used to show only the hub, with no next step for the
      // user to take).
      remember: (mogId, requestId, ticket) =>
        rememberFlow
          ? void rememberFlow.card(mogId, requestId, ticket).catch(() => undefined)
          : memoryUnavailable(mogId),
      undoRemember: (mogId, itemId) =>
        void rememberFlow?.undo(mogId, itemId).catch(() => undefined),
      closeMog: (mogId) => void components.T1.stop(mogId).catch(() => undefined),
      changeHome: () => {
        void showHub();
        void workspaces.showSettings();
      },
    });
    // #ui-4 (review #603, findings 1 and 4): the MOG right-click menu's native popup. A MOG
    // view may only listen and call `view_emit` (ARC-4); the brain holds `core:menu:*` and
    // builds the menu on the MOG's own window, then replays Close/Open in Bottle/Change home
    // workspace… through the bus exactly like a real click (`wireGenieCard` above and
    // `wireTalkTarget` act on it unchanged).
    createAppMogContextMenu(bus);
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
    const homes = createHomeProfiles(ipcHomeProfileDeps(() => workspaces.home()));
    const insertionOn = () => true;
    // ADR 0122: the pre-pass, the 1500 ms deadline and the grounding check wrap the router step.
    const route = withDictationCleanup(withOfflineFallback(ipcUtteranceDeps()), {
      cleanDictation: () => orchestrator.cleanDictation(),
    });
    const pick = createProfilePick({
      stackFor,
      showCard: (mogId) => call('window_card_show', { mogId }),
      listProfiles: ipcListProfiles,
      // ADR 0117: the pick makes that workspace the home. The router input is read again, so
      // the ask gate finds the new target when the press goes on.
      saveHome: async (mogId, profile) => {
        await workspaces.pickHome(profile);
        await homes.routerInput(mogId);
      },
    });
    // #14: the PTT and hands-free lifecycle. Every start goes through the T12 consent gate.
    const lifecycle = createCaptureLifecycle({
      control: ipcCaptureControl({ consent: components.T12 }),
      // MB-08a: a refused start is never silent. The consent gate also shows the hub notice.
      onRefused: (e) => {
        captureMood.refused(e.mogId, e.error);
        // UX slice 1: a refused start has no paste, so held arrivals need not wait for one.
        arrivalGate.refused();
      },
      // ADR 0118: a hold in a call recording shows "Call only" and, the first time, the note.
      onHold: ({ mogId, on }) => {
        if (mogId === '') return;
        captureMood.hold(mogId, on);
        if (on) heldNote.held(mogId);
      },
      // ADR 0103: ⌥R in the hold starts or stops the call recording, after the PTT discard. A
      // refusal shows on the MOG of the press (none with no open MOG).
      onRecordToggle: ({ mogId }) =>
        void createRecordToggle({
          call,
          // recording-profile.ts: the same home profile the hub Record control uses, so ⌥R and
          // the hub always reach the same workspace. No open MOG: no profile. With no home,
          // `recordingHomeProfile` throws `{ error: 'no_profile' }`, which
          // `createRecordToggle`'s catch forwards to `onRefused`.
          home: mogId === '' ? async () => undefined : recordingHomeProfile,
          onRefused: (error) => {
            if (mogId !== '') captureMood.refused(mogId, error);
          },
        })(),
      // #52 (ADR 0088): T4 runs the 8.3 chain over `stt_transcribe` (host route TODO(#514)).
      // A failure shows its fixed text on the owner's card, and no text is routed.
      transcribe: noPasteOnEmpty(
        createLifecycleTranscriber(components.T4, sendOwnedCard, createDictationCorrector()),
        (seg) => arrivalGate.pasteResolved(false, seg),
      ),
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
        // genie-followup (#145, item 4): while the hub window is focused on a thread, a ⌥G
        // press is a follow-up in it instead, with no card and no home-profile guess. A chip
        // (no ⌥G) and an injection-suspected ask are never affected: only a clean ⌥G is.
        onAsk: (r, owned) => {
          void routeAskToFocusedThread(r, {
            focus: threadFocus,
            followup,
            agentSend: (threadId, question) => agentRef.send?.(threadId, undefined, question),
            release: () => owner.release(owned.utteranceId),
            hubFocused: () => isHubFocused(),
          })
            .catch(() => false)
            .then((took) => {
              if (!took) askGate.onAsk(r, owned);
            });
        },
        // ADR 0113: Rust parsed "remember this"; the brain saves it and pastes nothing.
        onRemember: (r, owned) =>
          rememberFlow
            ? void rememberFlow.voice(owned.mogId, r.text, r.ticket).catch(() => undefined)
            : memoryUnavailable(owned.mogId),
        // #498, MOG-05b, MOG-06d: a Copy or a failed insertion reaches the Copy card.
        onDone: (r, owned) => void dictation.onDone(r, owned).catch(() => undefined),
        // UX slice 1: the paste ended (or none came): the held arrivals follow 1.2 s later.
        onPasteResolved: (pasted, seg) => arrivalGate.pasteResolved(pasted, seg),
      }),
    });
    wireCaptureLifecycle(bus, lifecycle, createSystemClock().now);
    // CAP-16a, CAP-16b (#23, #476): the mic level goes only to the target MOG, and stops on
    // stop. The host sends the levels on the `capture_subscribe` `onLevel` Channel.
    const levels = new LevelRouter(sendMicLevel, createSystemClock().now);
    wireLevelStream(bus, levels);
    // lamp-alive: the core's look vector goes to the one MOG view it is for.
    wireMogLook(bus);
    void subscribeLevels(levels);
    const subscribed = subscribeBrain(bus);
    // The brain listens now: send the hub what it could not ask for while the brain was not up.
    // The store notice goes first, because on macOS the store waits for the hub to show its reason.
    void subscribed.then((ok) => {
      if (!ok) return;
      void emitTo('hub', STORE_EVENT, storeGate.view()).catch(() => undefined);
      void conversations.push();
      void workspaces.refresh().then(() => orchestrator.refresh());
    });
    brain = { components, bus, talkTarget, conversations, ready: subscribed };
  }
  return brain;
}
