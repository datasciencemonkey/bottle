/**
 * The local history of UC2 Ask Genie (#40, AT-09, MEM-25a, ADR 0061, ADR 0097).
 *
 * When a request of the direct Genie path ends (`genie-direct.ts`), this module saves one
 * `genie_exchange` row in the local R10 store through T10 `recordExchange`:
 * - The MOG row goes first (`mem.mog.upsert`): the exchange table links to it (ADR 0061). The
 *   upsert is idempotent, so it goes before each exchange. A Forget or a Wipe that removed the
 *   MOG row never makes a later exchange fail.
 * - One write per request (`opId`): a second end of the same request writes nothing. The check
 *   holds the newest `RECENT_OPS_MAX` ids only.
 * - The MOG name is never empty (`MOG_NAME_FALLBACK`): the table has a length CHECK.
 * - It saves the exact question that was sent, the answer, the key numbers that the card shows,
 *   the deep link, and the conversation and response ids of the `accepted` event.
 * - Status: an answer or a clarify is COMPLETED, then FAILED, UNCERTAIN, ABANDONED and
 *   CANCEL_REQUESTED (T10 maps them to the table status).
 * - ADR 0117: the card stays on the lamp, but the exchange is stored under the data MOG id of
 *   the profile that answered (`mogIds[0]` of its workspace row, `dataMogId`). With no such id
 *   (a profile with no config yet) it falls back to the lamp's id. History and memory stay in
 *   the folders of the old MOGs.
 * - The direct path targets the MOG's home profile, so its row is a home-workspace row. Slice 2a:
 *   an agent turn writes one row for each Genie call (`homeWorkspace` says which), and the rows
 *   of a turn share one `splitGroupId`.
 * - The direct path sends no context preamble, so the sent question is the user's question.
 * - A failed write never rejects and never touches the card. The log gets only the fixed line
 *   `event=memory.write ok=0` (SEC-16c): no question, answer or error text.
 */
import type {
  GenieExchange,
  MemoryService,
  MogId,
  OperationState,
  Profile,
  ProfileName,
} from '../shared/interfaces';
import type { TerminalAskEvent } from './components/ask-events';
import { ipcMogConfigClient } from './components/mog-manager';
import type { MogRow } from './components/memory-service';
import { ipcListProfiles } from './home-profile';
import { createBrainLog, type BrainLog } from './log';

/** The end of one Ask Genie request, as `genie-direct.ts` reports it. */
export interface GenieEnd {
  opId: string;
  mogId: MogId;
  profile: ProfileName;
  /** The exact question that was sent. */
  question: string;
  end: TerminalAskEvent;
  /** From the `accepted` event, when there was one. */
  conversationId?: string;
  responseId?: string;
  /**
   * Slice 2a: one agent turn makes one row for each Genie call, and the rows of a turn share this
   * UUID (`genie_exchange.split_group_id`, the `turnId`).
   */
  splitGroupId?: string;
  /** Slice 2a: false for a call to a workspace that is not home (the table keeps no question text). */
  homeWorkspace?: boolean;
}

/** The part of T10 that the history uses. */
export interface ExchangeMemory {
  registerMog(m: MogRow): Promise<void>;
  recordExchange(x: GenieExchange): Promise<void>;
}

export function isExchangeMemory(m: object): m is ExchangeMemory {
  const o = m as Partial<Record<keyof ExchangeMemory, unknown>>;
  return typeof o.registerMog === 'function' && typeof o.recordExchange === 'function';
}

export interface GenieHistoryDeps {
  memory: ExchangeMemory;
  /** The MOG's name (`config_read`). */
  mogName(mogId: MogId): Promise<string>;
  /** `mogIds[0]` of the profile's workspace row, or undefined (`workspaces.dataMogId`). */
  dataMogId?(profile: ProfileName): Promise<MogId | undefined>;
  /** The workspace host of a profile (`profiles_list`), or undefined if it is not listed. */
  hostOf(profile: ProfileName): Promise<string | undefined>;
  log: Pick<BrainLog, 'event'>;
}

/**
 * The MOG row name when `config_read` fails or gives no name. `mog.display_name` has
 * `CHECK (length BETWEEN 1 AND 80)`, so the row needs a name. A later upsert with the real name
 * replaces it.
 */
/** The backend of a MOG row that the direct Genie path makes (ADR 0097). */
export const MOG_BACKEND_KIND = 'genie_one_mcp';

export const MOG_NAME_FALLBACK = 'MOG';

/**
 * The most request ids that the once-per-request check holds. A request ends within the give-up
 * window, so only recent ids matter; the oldest goes first.
 */
export const RECENT_OPS_MAX = 256;

const STATUS: Record<TerminalAskEvent['t'], OperationState> = {
  answer: 'COMPLETED',
  needs_input: 'COMPLETED',
  failed: 'FAILED',
  uncertain: 'UNCERTAIN',
  abandoned: 'ABANDONED',
  cancel_requested: 'CANCEL_REQUESTED',
};

function exchangeOf(x: GenieEnd, host: string, mogId: MogId): GenieExchange {
  const e = x.end;
  return {
    requestId: x.opId,
    mogId,
    ref: {
      backend: 'genie_one_mcp',
      profile: x.profile,
      host,
      conversationId: x.conversationId ?? '',
      ...(x.responseId !== undefined ? { responseId: x.responseId } : {}),
    },
    questionUser: x.question,
    questionSent: x.question,
    status: STATUS[e.t],
    ...(e.t === 'answer'
      ? { answerMd: e.md }
      : e.t === 'needs_input'
        ? { answerMd: e.prompt }
        : {}),
    keyNumbers: e.t === 'answer' ? e.keyNumbers : [],
    deepLink: e.t === 'answer' ? e.deepLink : '',
    homeWorkspace: x.homeWorkspace ?? true,
    ...(x.splitGroupId !== undefined ? { splitGroupId: x.splitGroupId } : {}),
  };
}

/** The history writer. The returned function never rejects. */
export function createGenieHistory(deps: GenieHistoryDeps): (x: GenieEnd) => Promise<void> {
  const done = new Set<string>();
  return async (x) => {
    if (done.has(x.opId)) return;
    done.add(x.opId);
    // A Set iterates in insertion order, so the first id is the oldest.
    if (done.size > RECENT_OPS_MAX) done.delete(done.values().next().value as string);
    try {
      const dataMogId = (await deps.dataMogId?.(x.profile).catch(() => undefined)) ?? x.mogId;
      const [name, host] = await Promise.all([
        deps.mogName(dataMogId).catch(() => ''),
        deps.hostOf(x.profile).catch(() => undefined),
      ]);
      await deps.memory.registerMog({
        mogId: dataMogId,
        displayName: name.trim() === '' ? MOG_NAME_FALLBACK : name,
        backendKind: MOG_BACKEND_KIND,
      });
      await deps.memory.recordExchange(exchangeOf(x, host ?? '', dataMogId));
    } catch {
      deps.log.event('memory.write', { ok: 0 });
    }
  };
}

/**
 * The `startBrain` history over IPC, or undefined when T10 cannot record exchanges. `saved` runs
 * after each write attempt (#103: the hub Conversations list reads the store again).
 */
export function ipcGenieHistory(
  memory: MemoryService,
  saved?: () => void,
  dataMogId?: (profile: ProfileName) => Promise<MogId | undefined>,
): ((x: GenieEnd) => void) | undefined {
  if (!isExchangeMemory(memory)) return undefined;
  const configs = ipcMogConfigClient();
  const record = createGenieHistory({
    memory,
    mogName: async (mogId) => (await configs.read(mogId)).name,
    ...(dataMogId ? { dataMogId } : {}),
    hostOf: async (profile) =>
      (await ipcListProfiles()).find((p: Profile) => p.name === profile)?.host,
    log: createBrainLog(),
  });
  return (x) => void record(x).then(saved);
}
