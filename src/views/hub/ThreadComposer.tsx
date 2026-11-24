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
 * - An error stays inline under the thread as one fixed line (the brain's genie-errors text, never
 *   upstream text) with Try again, which sends the same question again.
 * - The brain says whether the thread's workspace is signed in (`thread.check`,
 *   `hub:thread.access`). When it is not, the field and Send are disabled and the line says why.
 * - Item 4 (voice into the thread): while mounted and the hub window has focus, it announces
 *   `thread.active` with this thread's `conversationId` and `profile`, so a ⌥G press is a
 *   follow-up in this thread instead of a new question on the home workspace. A blur, an
 *   unmount or a switch to another thread announces `thread.inactive` (or the new thread's
 *   `thread.active`) at once, so a ⌥G the moment the hub is not focused on it behaves as today.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from 'react';
import { ASKING_GENIE, timerText } from '../../shared/genie-status';
import { HUB_SCOPE, type HubAgentView } from '../../shared/hub-agent';
import {
  asThreadAccess,
  asThreadStatus,
  CONVERSATIONS_VIEW_ID,
  HUB_THREAD_ACCESS_EVENT,
  HUB_THREAD_STATUS_EVENT,
  type ThreadComposerThread,
} from '../../shared/hub-conversations';
import { Button } from '../ui/button';
import { Textarea } from '../ui/textarea';
import { emitView } from '../view-ipc';
import { notReadyText, QUEUE_LIMIT } from './AgentThread';
import { useHubOrchestrator } from './use-hub-orchestrator';

/**
 * A half-typed question for each thread, kept in memory only (never saved): the composer
 * unmounts when the hub shows another area, and the draft must be there when it comes back.
 */
const drafts = new Map<string, string>();

/** The field grows to about six lines (20 px each, plus its padding), then scrolls inside. */
const MAX_FIELD_PX = 136;

/** The inline line when the view itself cannot reach the brain (a fixed Bottle text). */
const SEND_FAILED = 'Bottle could not send this question.';
/** The most characters of one agent question (`asAgentThreadSendAction`). */
const QUESTION_MAX = 1000;
const QUESTION_TOO_LONG = 'A question can have up to 1000 characters.';

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

function checkAccess(conversationId: string): void {
  void emitView(CONVERSATIONS_VIEW_ID, 'click', { action: 'thread.check', conversationId }).catch(
    () => undefined,
  );
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
  const [text, setText] = useState(() => drafts.get(thread.conversationId) ?? '');
  const fieldRef = useRef<HTMLTextAreaElement>(null);
  const [sentAt, setSentAt] = useState<number | undefined>(undefined);
  const [statusText, setStatusText] = useState<string | undefined>(undefined);
  const [error, setError] = useState<{ text: string; question: string | undefined } | undefined>(
    undefined,
  );
  const [signIn, setSignIn] = useState<string | undefined>(undefined);
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
          // Also a follow-up this view did not send (⌥G): the same busy state and timer.
          setError(undefined);
          setStatusText(s.text);
          setSentAt((t) => t ?? now());
          return;
        }
        // The answer is about to show in the thread's own list; the status line has nothing
        // left to hold, and the field opens for the next question. A failure stays inline.
        setStatusText(undefined);
        setSentAt(undefined);
        if (s.phase === 'failed') {
          setError({ text: s.text, question: s.question });
          checkAccess(conversationId);
        }
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

  // Whether the thread's workspace is signed in: asked on mount and whenever the hub window gets
  // focus (the user may have signed in meanwhile). The brain reads the profile from the store.
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    const target = getCurrentWindow().label;
    void listen<unknown>(
      HUB_THREAD_ACCESS_EVENT,
      (e) => {
        const a = asThreadAccess(e.payload);
        if (!a || a.conversationId !== conversationId) return;
        setSignIn(a.state === 'sign_in' ? a.text : undefined);
        // A request of this thread is already running (the view was away, or it was ⌥G).
        if (a.busy) setSentAt((t) => t ?? now());
      },
      { target },
    ).then((u) => {
      if (gone) u();
      else off = u;
    });
    const recheck = () => checkAccess(conversationId);
    window.addEventListener('focus', recheck);
    recheck();
    return () => {
      gone = true;
      off?.();
      window.removeEventListener('focus', recheck);
    };
  }, [conversationId]);

  // The "m:ss" timer, the same as the card's: it ticks once a second while a request is open.
  useEffect(() => {
    if (sentAt === undefined) return;
    const id = setInterval(() => forceTick((t) => t + 1), 1_000);
    return () => clearInterval(id);
  }, [sentAt]);

  const sending = sentAt !== undefined;

  const blocked = signIn !== undefined;

  const sendText = (question: string) => {
    setError(undefined);
    setSentAt(now());
    setStatusText(ASKING_GENIE);
    void sendQuestion(thread, question).catch(() => {
      setSentAt(undefined);
      setStatusText(undefined);
      setError({ text: SEND_FAILED, question });
    });
  };

  const send = () => {
    const question = text.trim();
    if (question === '' || sending || blocked) return;
    setText('');
    drafts.delete(conversationId);
    sendText(question);
  };

  const retry = () => {
    if (!error?.question || sending || blocked) return;
    sendText(error.question);
  };

  // Focus: the field stays focusable while a request runs (read-only, not disabled), and takes
  // focus back when the answer or the error arrives, unless the user moved it to another control.
  const wasSending = useRef(false);
  useEffect(() => {
    if (wasSending.current && !sending) {
      const active = document.activeElement;
      if (active === null || active === document.body || active === fieldRef.current)
        fieldRef.current?.focus();
    }
    wasSending.current = sending;
  }, [sending]);

  // Grow with the text, up to about six lines, then scroll inside.
  useEffect(() => {
    const el = fieldRef.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, MAX_FIELD_PX)}px`;
  }, [text]);

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // An Enter that confirms an IME candidate must not send a half-written question.
    if (e.nativeEvent.isComposing || e.keyCode === 229) return;
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  };

  const timer = sentAt !== undefined ? timerText(now() - sentAt) : undefined;

  return (
    <div className="thread-composer max-w-[68ch]">
      {statusText && (
        <p
          className="thread-composer-status mb-2 text-sm tabular-nums text-muted-foreground"
          role="status"
        >
          {statusText}
          {timer && <span className="thread-composer-timer"> {timer}</span>}
        </p>
      )}
      {signIn !== undefined && (
        <p className="thread-composer-signin mb-2 text-sm text-muted-foreground" role="status">
          {signIn}
        </p>
      )}
      {error && !statusText && (
        <div
          className="thread-composer-error mb-2 flex items-center gap-3 text-sm text-destructive"
          role="alert"
        >
          <span>{error.text}</span>
          {error.question && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              data-thread-retry
              disabled={blocked}
              onClick={retry}
            >
              Try again
            </Button>
          )}
        </div>
      )}
      <form
        className="thread-composer-row flex items-end gap-2"
        onSubmit={(e: FormEvent) => {
          e.preventDefault();
          send();
        }}
      >
        <Textarea
          className="thread-composer-field flex-1 overflow-y-auto"
          value={text}
          ref={fieldRef}
          onChange={(e) => {
            setText(e.target.value);
            drafts.set(conversationId, e.target.value);
          }}
          onKeyDown={onKeyDown}
          placeholder={blocked ? 'Sign in to ask a follow-up' : 'Ask a follow-up'}
          aria-label="Ask a follow-up in this conversation"
          readOnly={sending}
          disabled={blocked}
          aria-busy={sending}
          rows={1}
        />
        <Button type="submit" disabled={sending || blocked || text.trim() === ''}>
          Send
        </Button>
      </form>
      <p className="thread-composer-hint mt-2 text-xs text-muted-foreground">{VOICE_HINT}</p>
    </div>
  );
}

/** The inline line when the queue is full. */
export const queueFullText = (n: number) => `${n} questions are waiting. Send this when one lands.`;

function announceAgent(threadId: string | undefined): void {
  const payload = threadId
    ? { action: 'thread.active_agent' as const, threadId }
    : { action: 'thread.inactive' as const };
  void emitView(CONVERSATIONS_VIEW_ID, 'click', payload).catch(() => undefined);
}

export interface AgentComposerProps {
  threadId: string;
  /** The MOG the follow-up goes to. Undefined until the Hub knows it: Send stays off. */
  mogId: string | undefined;
  agent: HubAgentView | undefined;
}

/**
 * The composer of an agent thread: "Ask a follow-up". Send sends `agent.thread_send` for this
 * thread (the reply of a live turn or a queued turn is the brain's call). While the hub has focus
 * on it, a ⌥G is a follow-up in this thread (`thread.active_agent`). With the orchestrator model not ready
 * it shows why, with Open Settings. With a full queue it keeps the text and turns Send off.
 */
export function AgentComposer({ threadId, mogId, agent }: AgentComposerProps) {
  const key = `agent:${threadId}`;
  const [text, setText] = useState(() => drafts.get(key) ?? '');
  const [failed, setFailed] = useState(false);
  const fieldRef = useRef<HTMLTextAreaElement>(null);
  const orchestrator = useHubOrchestrator();
  const waiting = agent?.waiting.length ?? 0;
  const full = waiting >= QUEUE_LIMIT;

  useEffect(() => {
    let focused = document.hasFocus();
    const announce = () => announceAgent(focused ? threadId : undefined);
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
      announceAgent(undefined);
    };
  }, [threadId]);

  useEffect(() => {
    const el = fieldRef.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, MAX_FIELD_PX)}px`;
  }, [text]);

  if (agent !== undefined && orchestrator?.state !== undefined && orchestrator.state !== 'ready') {
    return (
      <div className="thread-composer max-w-[68ch]" data-agent-composer="not-ready">
        <p className="thread-composer-signin mb-2 text-sm text-muted-foreground" role="status">
          {notReadyText(orchestrator.state, orchestrator.homeName)}
        </p>
        <Button
          type="button"
          variant="outline"
          size="sm"
          data-agent-open-settings
          onClick={() =>
            void emitView(HUB_SCOPE, 'click', { action: 'agent.open_settings' }).catch(
              () => undefined,
            )
          }
        >
          Open Settings
        </Button>
      </div>
    );
  }

  const blocked = full || mogId === undefined;
  const send = () => {
    const question = text.trim();
    if (question === '' || blocked || mogId === undefined) return;
    setFailed(false);
    setText('');
    drafts.delete(key);
    void emitView(CONVERSATIONS_VIEW_ID, 'click', {
      action: 'agent.thread_send',
      threadId,
      mogId,
      question,
    }).catch(() => {
      // The question comes back into the field, so nothing typed is lost.
      setText(question);
      drafts.set(key, question);
      setFailed(true);
    });
  };

  return (
    <div className="thread-composer max-w-[68ch]" data-agent-composer="ready">
      {full && (
        <p
          className="thread-composer-status mb-2 text-sm text-muted-foreground"
          role="status"
          data-agent-queue-full
        >
          {queueFullText(waiting)}
        </p>
      )}
      {failed && (
        <p className="thread-composer-error mb-2 text-sm text-destructive" role="alert">
          {SEND_FAILED}
        </p>
      )}
      {text.length >= QUESTION_MAX && (
        <p
          className="thread-composer-status mb-2 text-sm text-muted-foreground"
          role="status"
          data-agent-too-long
        >
          {QUESTION_TOO_LONG}
        </p>
      )}
      <form
        className="thread-composer-row flex items-end gap-2"
        onSubmit={(e: FormEvent) => {
          e.preventDefault();
          send();
        }}
      >
        <Textarea
          className="thread-composer-field flex-1 overflow-y-auto"
          value={text}
          ref={fieldRef}
          // The brain refuses a question over 1000 characters: the field stops at the limit.
          maxLength={QUESTION_MAX}
          onChange={(e) => {
            setText(e.target.value);
            drafts.set(key, e.target.value);
          }}
          onKeyDown={(e: KeyboardEvent<HTMLTextAreaElement>) => {
            if (e.nativeEvent.isComposing || e.keyCode === 229) return;
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault();
              send();
            }
          }}
          placeholder="Ask a follow-up"
          aria-label="Ask a follow-up in this conversation"
          rows={1}
        />
        <Button type="submit" disabled={blocked || text.trim() === ''}>
          Send
        </Button>
      </form>
      <p className="thread-composer-hint mt-2 text-xs text-muted-foreground">{VOICE_HINT}</p>
    </div>
  );
}
