/**
 * genie-followup (#145, #103, #584, ADR 0097): a follow-up question sent from the hub thread
 * composer (`ThreadComposer.tsx`) or from a ⌥G press while the hub is focused on a thread
 * (`thread-focus.ts`, item 4 of #145).
 *
 * It is the thread's own hand-off seam, parallel to `genie-direct.ts`'s ask-gate seam, but it
 * never touches a MOG card: the live status goes to the hub as a `ThreadStatus`
 * (`hub:thread.status`), and the terminal event is saved through the same local-history writer
 * (`onEnd`, `genie-history.ts`) that a direct ask uses, so the finished exchange shows up in the
 * thread's own list on the next `hub:conversations` refresh.
 *
 * - One send, straight to T8 `GenieBridge.submit`, with the thread's exact `conversationId` and
 *   `profile` (`binding.kind: 'genie_one_mcp'`): a conversation stays in its workspace. The
 *   composer never carries a `mogId` (only `conversationId`, `exchangeId` and `profile`), so
 *   `findMogId` resolves the MOG that owns this conversation from the stored exchanges — the
 *   same MOG the first question in the thread asked under.
 * - G-47, G-30a (same as `ask-gate.ts`'s `handOff`): the sanitizer runs first, then the 1,000
 *   character cap. A text with nothing left, or a thread `findMogId` cannot resolve (a stale or
 *   deleted thread), is never sent: the status line says only "Not sent.", with no opId.
 * - The status text is the same the card shows (`ASKING_GENIE`, then a fixed stage from
 *   `genieStage`): no raw Genie step ever reaches the hub.
 * - G-21b: one terminal event ends the request, whatever happens (a GL-5 failure, a broken
 *   stream, or a throw in `onEnd`); `send` never rejects.
 */
import type { AskRequest, GenieBridge, ProfileName } from '../shared/interfaces';
import { ASKING_GENIE, genieStage } from '../shared/genie-status';
import { NOT_SENT_LINE, capQuestion } from './ask-gate';
import { isTerminal, type TerminalAskEvent } from './components/ask-events';
import type { GenieEnd } from './genie-history';
import type { ThreadPhase, ThreadStatus } from '../shared/hub-conversations';
import { sanitizeQuestion } from './question-sanitizer';

/** One Send from the composer, or one ⌥G press while the hub is focused on a thread. */
export interface FollowupInput {
  conversationId: string;
  profile: ProfileName;
  question: string;
}

export interface GenieFollowupDeps {
  /** T8. Only `submit` is used; the bridge itself already knows how to continue a conversation
   * when `AskRequest.conversationId` is set (`genie-bridge.ts`). */
  genie: Pick<GenieBridge, 'submit'>;
  /**
   * The `mogId` that owns `conversationId` (`HubConversationsWiring.findMogId`): a conversation
   * stays in its workspace, and in the MOG that started it. `undefined` when no stored exchange
   * carries this id (a stale or deleted thread): nothing is sent.
   */
  findMogId(conversationId: string): Promise<string | undefined>;
  /** `hub:thread.status`, to the hub window only. Never throws. */
  sendStatus(status: ThreadStatus): void;
  /** #40: the same local-history writer a direct ask uses, so the finished exchange lands in
   * the thread's own list. Never throws (the callers of this module already catch). */
  onEnd?: ((x: GenieEnd) => void) | undefined;
  /** A new operation ID; only allocated once a question is actually going to be sent. */
  newOpId(): string;
}

export interface GenieFollowup {
  /** Never rejects: a refusal, a GL-5 failure or a client fault all end in one `ThreadStatus`. */
  send(input: FollowupInput): Promise<void>;
}

/** The mood phase of a terminal event (`genie-direct.ts`'s `phaseOf`, without the card). */
function phaseOf(e: TerminalAskEvent): ThreadPhase {
  if (e.t === 'cancel_requested') return 'cancelled';
  return e.t === 'answer' || e.t === 'needs_input' ? 'answered' : 'failed';
}

export function createGenieFollowup(deps: GenieFollowupDeps): GenieFollowup {
  const refuse = (conversationId: string): void => {
    deps.sendStatus({ conversationId, requestId: '', phase: 'failed', text: NOT_SENT_LINE });
  };

  return {
    async send(input) {
      const { conversationId, profile } = input;
      const clean = sanitizeQuestion(input.question);
      if (clean === '') {
        refuse(conversationId);
        return;
      }
      const { text: question } = capQuestion(clean);
      const mogId = await deps.findMogId(conversationId).catch(() => undefined);
      if (mogId === undefined) {
        refuse(conversationId);
        return;
      }
      const opId = deps.newOpId();
      const req: AskRequest = {
        opId,
        mogId,
        binding: { kind: 'genie_one_mcp', profile },
        question,
        conversationId,
        tags: { app: 'bottle', mog: mogId, op: 'genie' },
      };
      deps.sendStatus({ conversationId, requestId: opId, phase: 'working', text: ASKING_GENIE });
      let shownStage: string | undefined;
      let accepted: { conversationId: string; responseId: string } | undefined;
      let end: TerminalAskEvent | undefined;
      try {
        const { events } = deps.genie.submit(req);
        for await (const e of events) {
          if (e.t === 'accepted')
            accepted = { conversationId: e.conversationId, responseId: e.responseId };
          if (isTerminal(e)) {
            end = e;
            break;
          }
          if (e.t === 'progress') {
            const stage = genieStage(e.steps);
            if (stage !== undefined && stage !== shownStage) {
              shownStage = stage;
              deps.sendStatus({ conversationId, requestId: opId, phase: 'working', text: stage });
            }
          }
        }
      } catch {
        // T8 ends every request in one terminal event (G-21b); a throw here is a client fault.
      }
      end ??= { t: 'uncertain' };
      deps.sendStatus({ conversationId, requestId: opId, phase: phaseOf(end), text: '' });
      try {
        deps.onEnd?.({
          opId,
          mogId,
          profile,
          question,
          end,
          ...accepted,
        });
      } catch {
        // The status line already shows the outcome; a history fault changes nothing here.
      }
    },
  };
}
