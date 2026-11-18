/**
 * The card render of every terminal AskEvent (#33, ADR 0084) with the Appendix E templates:
 * Completed, Partial answer (G-23a), Failed (G-23b), Clarify (G-22), Gave up, Ask uncertain and
 * Cancelled. The line cuts, the bold key numbers and the footer line are #86.
 */
import type { AskEvent } from '../shared/interfaces';
import { BlurbStack } from './blurb-stack';
import { classifyTransportCode, toFailedEvent, userTextFor } from './components/genie-errors';
import { outcomeBlurb, outcomeBlurbInput } from './outcome-blurb';

const answer = (md: string, incomplete = false): Extract<AskEvent, { t: 'answer' }> => ({
  t: 'answer',
  md,
  incomplete,
  keyNumbers: [],
  deepLink: 'https://example.cloud.databricks.com/one/chat/threads/c1?o=1',
  queryItems: [],
});

describe('outcome blurbs (Appendix E)', () => {
  it('[G-23a] [AT-09] an incomplete answer shows "Partial answer" with [Ask again]', () => {
    const b = outcomeBlurb(answer('Revenue in EMEA was **$410,500**.', true));
    expect(b).toEqual({
      state: 'partial',
      lines: ['Partial answer: Revenue in EMEA was **$410,500**.', 'Genie did not finish all steps.'],
      controls: ['ask_again'],
      suggestions: [],
    });
  });

  it('[G-23b] [AT-09] incomplete without an answer shows a failure with its code and action', () => {
    const b = outcomeBlurb({
      t: 'failed',
      code: 'GENIE_INCOMPLETE',
      userText: userTextFor('GENIE_INCOMPLETE'),
      retry: 'user',
    });
    expect(b).toEqual({
      state: 'failed',
      lines: ['Genie did not finish all steps.', 'Code GENIE_INCOMPLETE'],
      controls: ['ask_again'],
      suggestions: [],
    });
    expect(b.lines.join(' ')).not.toMatch(/^Partial answer/);
  });

  it('[AT-09] a completed answer shows the answer and [Open in Databricks]', () => {
    const b = outcomeBlurb(answer('Revenue last quarter was **$1.2M**.'));
    expect(b.state).toBe('completed');
    expect(b.lines[0]).toBe('Revenue last quarter was **$1.2M**.');
    expect(b.controls).toEqual(['open_in_databricks']);
  });

  it('[G-22] [AT-09] a clarify shows "Genie asks:" with suggestion chips, [Answer by voice] and [Type]', () => {
    const b = outcomeBlurb({
      t: 'needs_input',
      kind: 'clarify',
      prompt: 'Do you mean gross revenue or net revenue?',
      options: ['Gross', 'Net'],
    });
    expect(b).toEqual({
      state: 'clarify',
      lines: ['Genie asks: Do you mean gross revenue or net revenue?'],
      controls: ['answer_by_voice', 'type'],
      suggestions: ['Answer: Gross', 'Answer: Net'],
    });
  });

  it('[AT-09] every failure code shows <user_text>, "Code <code>" and the action map', () => {
    expect(outcomeBlurb(toFailedEvent(classifyTransportCode('auth_expired')))).toMatchObject({
      lines: [userTextFor('AUTH_EXPIRED'), 'Code AUTH_EXPIRED'],
      controls: ['sign_in'],
    });
    expect(outcomeBlurb(toFailedEvent(classifyTransportCode('host_failed')))).toMatchObject({
      lines: ['Genie could not answer this question.', 'Code GENIE_FAILED'],
      controls: ['retry', 'open_in_databricks'],
    });
    // Policy denied: the Appendix E row "Policy denied (after retry)".
    expect(
      outcomeBlurb({ t: 'failed', code: 'POLICY_DENIED', userText: userTextFor('POLICY_DENIED'), retry: 'without_preamble' })
        .controls,
    ).toEqual(['resend_without_context']);
  });

  it('[ARC-F3] a final RATE_LIMITED failure offers [Retry] and no countdown', () => {
    const b = outcomeBlurb({
      t: 'failed',
      code: 'RATE_LIMITED',
      userText: 'Genie is busy. Try again in a few minutes.',
      retry: 'user',
    });
    expect(b.controls).toEqual(['retry']);
    expect(b.lines.join(' ')).not.toMatch(/retrying in/i);
  });

  it('[ARC-F1] a busy conversation offers [Wait] [New conversation], but a poll_inflight busy only [Wait]', () => {
    const busy = classifyTransportCode('poll_inflight');
    expect(outcomeBlurb(toFailedEvent(busy), { row: busy.row }).controls).toEqual(['wait']);
    expect(
      outcomeBlurb({ t: 'failed', code: 'CONVERSATION_BUSY', userText: userTextFor('CONVERSATION_BUSY'), retry: 'user' })
        .controls,
    ).toEqual(['wait', 'new_conversation']);
  });

  it('[AT-09] gave up, uncertain and cancelled use their Appendix E rows', () => {
    expect(outcomeBlurb({ t: 'abandoned' })).toMatchObject({
      state: 'gave_up',
      lines: ['No answer after 10 min. I will check again later.'],
      controls: ['check_now', 'open_in_databricks'],
    });
    expect(outcomeBlurb({ t: 'uncertain' })).toMatchObject({
      state: 'uncertain',
      lines: ['I do not know if Genie got your question.'],
      controls: ['check_again', 'ask_again'],
    });
    expect(outcomeBlurb({ t: 'cancel_requested' })).toMatchObject({
      state: 'cancelled',
      lines: ['Cancellation requested'],
      controls: [],
    });
  });

  it('[UX-16a] no outcome line holds a banned word', () => {
    const all: AskEvent[] = [
      answer('x'),
      answer('x', true),
      { t: 'needs_input', kind: 'clarify', prompt: 'Which?', options: [] },
      { t: 'abandoned' },
      { t: 'uncertain' },
      { t: 'cancel_requested' },
      ...(['offline', 'timeout', 'auth_expired', 'poll_inflight', 'rate_limited', 'bad_reply'] as const).map(
        (c) => toFailedEvent(classifyTransportCode(c)),
      ),
    ];
    for (const e of all) {
      const b = outcomeBlurb(e as Parameters<typeof outcomeBlurb>[0]);
      expect(b.lines.join(' ')).not.toMatch(/\b(?:MCP|backend|polling|poll|thread)\b/i);
    }
  });

  it('[AT-09] the blurb input is completed and goes on the card stack', () => {
    const stack = new BlurbStack();
    stack.add(outcomeBlurbInput('op-1', answer('Revenue in EMEA was $410,500.', true)), 0);
    expect(stack.view().blurbs).toEqual([
      {
        id: 'op-1',
        text: 'Partial answer: Revenue in EMEA was $410,500.\nGenie did not finish all steps.',
        pinned: false,
        completed: true,
      },
    ]);
  });
});
