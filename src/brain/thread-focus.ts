/**
 * thread-focus (#145, item 4): the hub's "focused on a thread" signal, and the view input of
 * the thread composer (`ThreadComposer.tsx`).
 *
 * - `thread.active` / `thread.inactive` (sent on mount, on a window focus or blur, on a thread
 *   switch, and on unmount) say whether the hub window is currently showing this thread with
 *   focus. A ⌥G press checks `active()` before it falls back to a new question on the home
 *   workspace (wired in `components/index.ts`): this module only holds the latest value, it
 *   never guesses one.
 * - `thread.send` is the composer's Send (or Enter): it goes straight to `GenieFollowup.send`,
 *   the same path a ⌥G follow-up uses.
 * - `thread.check` asks whether the thread's workspace is signed in (`hub:thread.access`).
 * - All of them travel under the Conversations view id (`CONVERSATIONS_VIEW_ID`), like
 *   `conversations.ready`, so no new `view_emit` kind is needed.
 */
import {
  asThreadCheckAction,
  asThreadFocusAction,
  asThreadSendAction,
  CONVERSATIONS_VIEW_ID,
  type ThreadAccess,
} from '../shared/hub-conversations';
import { onViewInput, type BrainBus } from './bus';
import type { GenieFollowup } from './genie-followup';

export interface ActiveThread {
  conversationId: string;
  profile: string;
}

export interface ThreadFocus {
  /** The thread the hub window is currently focused on, or undefined. */
  active(): ActiveThread | undefined;
  set(thread: ActiveThread | undefined): void;
}

/** One brain-wide focus: whichever hub window there is, only one can have OS focus at a time. */
export function createThreadFocus(): ThreadFocus {
  let current: ActiveThread | undefined;
  return {
    active: () => current,
    set(thread) {
      current = thread;
    },
  };
}

export interface ThreadFollowupWiring {
  dispose(): void;
}

/** Wire the composer's `thread.active` / `thread.inactive` / `thread.send`. */
export function wireThreadFollowup(
  bus: BrainBus,
  deps: {
    focus: ThreadFocus;
    followup: Pick<GenieFollowup, 'send' | 'access'>;
    /** `hub:thread.access`, to the hub window only. Never throws. */
    sendAccess?: ((access: ThreadAccess) => void) | undefined;
  },
): ThreadFollowupWiring {
  const off = onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.mogId !== CONVERSATIONS_VIEW_ID || input.kind !== 'click')
      return;
    const focusAction = asThreadFocusAction(input.payload);
    if (focusAction) {
      deps.focus.set(
        focusAction.action === 'thread.active'
          ? { conversationId: focusAction.conversationId, profile: focusAction.profile }
          : undefined,
      );
      return;
    }
    const check = asThreadCheckAction(input.payload);
    if (check) {
      void deps.followup
        .access(check.conversationId)
        .then((a) => deps.sendAccess?.(a))
        .catch(() => undefined);
      return;
    }
    const sendAction = asThreadSendAction(input.payload);
    if (sendAction) {
      void deps.followup.send({
        conversationId: sendAction.conversationId,
        profile: sendAction.profile,
        question: sendAction.question,
      });
    }
  });
  return { dispose: off };
}

/** The part of a routed utterance that decides whether ⌥G continues a thread. */
export interface RoutedAsk {
  route: string;
  question?: string;
  decision: { injection_suspected?: boolean | undefined };
}

/**
 * ⌥G while the hub window is focused on a thread (item 4): the question is a follow-up in that
 * thread, with no card and no home-profile guess. Only a clean `ask` route is taken; a dictation,
 * a chip fallback and an injection-suspected ask never are. `focus.active()` is only set while
 * the composer is mounted and the hub window has OS focus (the view clears it on blur, on a
 * view switch and on unmount), so a hidden thread is never the target. Returns true when it took
 * the ask (and `release` ran). It also asks the hub window for its OS focus (`hubFocused`).
 */
export async function routeAskToFocusedThread(
  r: RoutedAsk,
  deps: {
    focus: ThreadFocus;
    followup: Pick<GenieFollowup, 'send'>;
    release(): void;
    /** The hub window really has OS focus now (`isHubFocused`). A throw counts as false. */
    hubFocused(): Promise<boolean>;
  },
): Promise<boolean> {
  if (r.route !== 'ask' || r.question === undefined || r.decision.injection_suspected) return false;
  const thread = deps.focus.active();
  if (!thread) return false;
  // The view's `blur` can be missed (the hub hidden to the tray): ask the window itself, and
  // clear the stale focus so no later press is taken either.
  const focused = await deps.hubFocused().catch(() => false);
  if (!focused) {
    deps.focus.set(undefined);
    return false;
  }
  // The focus may have moved on while we asked.
  if (deps.focus.active() !== thread) return false;
  deps.release();
  void deps.followup.send({
    conversationId: thread.conversationId,
    profile: thread.profile,
    question: r.question,
  });
  return true;
}
