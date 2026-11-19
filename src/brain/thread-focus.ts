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
 * - All three travel under the Conversations view id (`CONVERSATIONS_VIEW_ID`), like
 *   `conversations.ready`, so no new `view_emit` kind is needed.
 */
import {
  asThreadFocusAction,
  asThreadSendAction,
  CONVERSATIONS_VIEW_ID,
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
  deps: { focus: ThreadFocus; followup: Pick<GenieFollowup, 'send'> },
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
