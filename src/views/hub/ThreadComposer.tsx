/**
 * The hub thread reply composer (genie-followup, #145, #103, #584): a text input and Send at
 * the bottom of a thread's detail (`Conversations.tsx` renders it through the `composer` prop,
 * only for a thread that has a Genie `conversationId` to continue).
 *
 * - Send appends the question to this thread: `thread.send` carries the thread's
 *   `conversationId` and `profile`, so the brain asks Genie a follow-up on the same conversation
 *   and the same workspace (a conversation stays in its workspace). Enter sends; Shift+Enter
 *   makes a new line.
 * - While the request is in flight the field and Send are disabled, and a status line shows the
 *   same words the card shows for an Ask Genie request: "Asking Genie", then a fixed stage line,
 *   with a running "m:ss" timer once it has waited a few seconds. The terminal event clears the
 *   line; by then the finished exchange is in the thread's own list (`hub:conversations`).
 * - This view is thin (ARC-2): it holds no domain state beyond its own input and status, and it
 *   runs no store statement.
 * - Item 4 (voice into the thread): while mounted and the hub window has focus, it announces
 *   `thread.active` with this thread's `conversationId` and `profile`, so a ⌥G press is a
 *   follow-up in this thread instead of a new question on the home workspace. A blur, an
 *   unmount or a switch to another thread announces `thread.inactive` (or the new thread's
 *   `thread.active`) at once, so a ⌥G the moment the hub is not focused on it behaves as today.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useState, type FormEvent, type KeyboardEvent } from 'react';
import { ASKING_GENIE, timerText } from '../../shared/genie-status';
import {
  asThreadStatus,
  CONVERSATIONS_VIEW_ID,
  HUB_THREAD_STATUS_EVENT,
  type ThreadComposerThread,
} from '../../shared/hub-conversations';
import { emitView } from '../view-ipc';

/** Appendix-style hint under the field: the voice route into this same thread (item 4). */
const VOICE_HINT = 'Or hold Right Option and tap G';

function announceActive(thread: { conversationId: string; profile: string } | undefined): void {
  const payload = thread
    ? {
        action: 'thread.active' as const,
        conversationId: thread.conversationId,
        profile: thread.profile,
      }
    : { action: 'thread.inactive' as const };
  void emitView(CONVERSATIONS_VIEW_ID, 'click', payload).catch(() => undefined);
}

function sendQuestion(thread: ThreadComposerThread, question: string): Promise<unknown> {
  return emitView(CONVERSATIONS_VIEW_ID, 'click', {
    action: 'thread.send',
    conversationId: thread.conversationId,
    profile: thread.profile,
    question,
  });
}

export interface ThreadComposerProps {
  thread: ThreadComposerThread;
  /** The clock for the "m:ss" timer. Tests pass a simulated one. */
  now?: () => number;
}

export function ThreadComposer({ thread, now = Date.now }: ThreadComposerProps) {
  const [text, setText] = useState('');
  const [sentAt, setSentAt] = useState<number | undefined>(undefined);
  const [statusText, setStatusText] = useState<string | undefined>(undefined);
  const [, forceTick] = useState(0);
  const { conversationId, profile } = thread;

  // Item 4: the hub window is focused on this thread only while the OS window has focus. A blur
  // or an unmount clears it at once; a switch to another thread announces that one instead (this
  // effect depends only on the two fields the brain needs, so an unrelated re-render of the same
  // thread, for example its `exchangeId` moving on, never re-announces it).
  useEffect(() => {
    // `document.hasFocus()`, not a hardcoded `true`: the Conversations area can mount this
    // composer while the hub window itself has no OS focus (for example a reload while another
    // app is frontmost), and a ⌥G must not wrongly become a follow-up in that case.
    let focused = document.hasFocus();
    const announce = () => announceActive(focused ? { conversationId, profile } : undefined);
    const onFocus = () => {
      focused = true;
      announce();
    };
    const onBlur = () => {
      focused = false;
      announce();
    };
    announce();
    window.addEventListener('focus', onFocus);
    window.addEventListener('blur', onBlur);
    return () => {
      window.removeEventListener('focus', onFocus);
      window.removeEventListener('blur', onBlur);
      announceActive(undefined);
    };
  }, [conversationId, profile]);

  // The live status of a sent follow-up, filtered to this thread's conversation only: another
  // thread's (or another MOG's) request never shows here.
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    const target = getCurrentWindow().label;
    void listen<unknown>(
      HUB_THREAD_STATUS_EVENT,
      (e) => {
        const s = asThreadStatus(e.payload);
        if (!s || s.conversationId !== conversationId) return;
        if (s.phase === 'working') {
          setStatusText(s.text);
          return;
        }
        // The answer (or the failure) is about to show in the thread's own list; the status
        // line has nothing left to hold, and the field opens for the next question.
        setStatusText(undefined);
        setSentAt(undefined);
      },
      { target },
    ).then((u) => {
      if (gone) u();
      else off = u;
    });
    return () => {
      gone = true;
      off?.();
    };
  }, [conversationId]);

  // The "m:ss" timer, the same as the card's: it ticks once a second while a request is open.
  useEffect(() => {
    if (sentAt === undefined) return;
    const id = setInterval(() => forceTick((t) => t + 1), 1_000);
    return () => clearInterval(id);
  }, [sentAt]);

  const sending = sentAt !== undefined;

  const send = () => {
    const question = text.trim();
    if (question === '' || sending) return;
    setText('');
    setSentAt(now());
    setStatusText(ASKING_GENIE);
    void sendQuestion(thread, question).catch(() => {
      setSentAt(undefined);
      setStatusText(undefined);
    });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  };

  const timer = sentAt !== undefined ? timerText(now() - sentAt) : undefined;

  return (
    <div className="thread-composer">
      {statusText && (
        <p className="thread-composer-status" role="status">
          {statusText}
          {timer && <span className="thread-composer-timer"> {timer}</span>}
        </p>
      )}
      <form
        className="thread-composer-row"
        onSubmit={(e: FormEvent) => {
          e.preventDefault();
          send();
        }}
      >
        <textarea
          className="thread-composer-field"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder="Ask a follow-up…"
          aria-label="Ask a follow-up in this conversation"
          disabled={sending}
          rows={1}
        />
        <button
          type="submit"
          className="hub-primary thread-composer-send"
          disabled={sending || text.trim() === ''}
        >
          Send
        </button>
      </form>
      <p className="thread-composer-hint">{VOICE_HINT}</p>
    </div>
  );
}
