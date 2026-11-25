/** The owner's "attach Genie's answer to the clipboard" request. Fake values. */
import type { AgentTurnView } from '../../shared/ipc';
import {
  createAnswerClipboard,
  type AnswerClipboardDeps,
  type CopyOutcome,
} from './answer-clipboard';

const MOG = 'a1';
const DIRECT_HOST = 'sales.cloud.databricks.com';

function turn(o: Partial<AgentTurnView> = {}): AgentTurnView {
  return {
    turnId: 't1',
    question: 'q',
    state: 'done',
    used: 1,
    budget: 10,
    agentWorkspace: { name: 'Sales', colorVariant: 'coral' },
    calls: [],
    parts: [
      {
        callId: 'c1',
        workspace: { name: 'Sales', colorVariant: 'coral' },
        sections: [{ id: 'answer', md: 'The answer.' }],
      },
    ],
    ...o,
  };
}

function fakeDeps(o: Partial<AnswerClipboardDeps> = {}) {
  const turnNotes: { turnId: string; outcome: CopyOutcome | undefined }[] = [];
  const directNotes: { mogId: string; opId: string; outcome: CopyOutcome | undefined }[] = [];
  const writes: string[] = [];
  const htmls: (string | undefined)[] = [];
  const faults: number[] = [];
  const deps: AnswerClipboardDeps = {
    enabled: () => true,
    write: async (text, html) => {
      writes.push(text);
      htmls.push(html);
      return 'written';
    },
    whenClear: (fn) => fn(),
    noteTurn: (turnId, outcome) => turnNotes.push({ turnId, outcome }),
    noteDirect: (mogId, opId, outcome) => directNotes.push({ mogId, opId, outcome }),
    onFault: () => faults.push(1),
    ...o,
  };
  return { deps, turnNotes, directNotes, writes, htmls, faults };
}

describe('createAnswerClipboard: onTurnFinished (the orchestrator path, H1)', () => {
  it('[owner request] copies the finished answer and reports "copied"', async () => {
    const { deps, turnNotes, writes } = fakeDeps();
    const clipboard = createAnswerClipboard(deps);
    clipboard.onTurnFinished(MOG, turn());
    await Promise.resolve();
    await Promise.resolve();
    expect(writes).toEqual(['The answer.']);
    expect(turnNotes).toEqual([{ turnId: 't1', outcome: 'copied' }]);
  });

  it('[M-A] clears its own note a short time after setting it, so it never resurfaces later', async () => {
    vi.useFakeTimers();
    const { deps, turnNotes } = fakeDeps();
    createAnswerClipboard(deps).onTurnFinished(MOG, turn());
    await vi.advanceTimersByTimeAsync(0);
    expect(turnNotes).toEqual([{ turnId: 't1', outcome: 'copied' }]);
    await vi.advanceTimersByTimeAsync(1_999);
    expect(turnNotes).toEqual([{ turnId: 't1', outcome: 'copied' }]);
    await vi.advanceTimersByTimeAsync(1);
    expect(turnNotes).toEqual([
      { turnId: 't1', outcome: 'copied' },
      { turnId: 't1', outcome: undefined },
    ]);
    vi.useRealTimers();
  });

  it('reports "failed" when the write answers too_large or unsupported', async () => {
    for (const result of ['too_large', 'unsupported'] as const) {
      const { deps, turnNotes } = fakeDeps({ write: async () => result });
      createAnswerClipboard(deps).onTurnFinished(MOG, turn());
      for (let i = 0; i < 4; i++) await Promise.resolve();
      expect(turnNotes).toEqual([{ turnId: 't1', outcome: 'failed' }]);
    }
  });

  it('falls back to plain text when the HTML copy is too large', async () => {
    const calls: Array<string | undefined> = [];
    const { deps, turnNotes } = fakeDeps({
      write: async (_text: string, html?: string) => {
        calls.push(html === undefined ? 'text' : 'html');
        return html === undefined ? 'written' : 'too_large';
      },
    });
    createAnswerClipboard(deps).onTurnFinished(MOG, turn());
    for (let i = 0; i < 4; i++) await Promise.resolve();
    expect(turnNotes[0]).toEqual({ turnId: 't1', outcome: 'copied' });
    expect(calls.at(-1)).toBe('text');
  });

  it('reports "failed" and calls onFault when the write throws, never with the answer text', async () => {
    const { deps, turnNotes, faults } = fakeDeps({
      write: async () => {
        throw new Error('fake: the answer, leaked');
      },
    });
    createAnswerClipboard(deps).onTurnFinished(MOG, turn());
    await Promise.resolve();
    await Promise.resolve();
    expect(faults).toEqual([1]);
    expect(turnNotes).toEqual([{ turnId: 't1', outcome: 'failed' }]);
  });

  it('never copies a withheld (quarantined) lone answer', async () => {
    const { deps, writes } = fakeDeps();
    createAnswerClipboard(deps).onTurnFinished(
      MOG,
      turn({
        parts: [
          {
            callId: 'c1',
            workspace: { name: 'Sales', colorVariant: 'coral' },
            sections: [
              {
                id: 'answer',
                md: 'This answer was held back. It looked like it held instructions.',
              },
            ],
          },
        ],
      }),
    );
    await Promise.resolve();
    expect(writes).toEqual([]);
  });

  it('copies exactly once per turn, even if called again with the same state', async () => {
    const { deps, writes } = fakeDeps();
    const clipboard = createAnswerClipboard(deps);
    const t = turn();
    clipboard.onTurnFinished(MOG, t);
    clipboard.onTurnFinished(MOG, t);
    clipboard.onTurnFinished(MOG, t);
    await Promise.resolve();
    await Promise.resolve();
    expect(writes).toEqual(['The answer.']);
  });

  it('the toggle off means no copy, but still marks the turn handled (no later retry)', async () => {
    const { deps, writes } = fakeDeps({ enabled: () => false });
    const clipboard = createAnswerClipboard(deps);
    clipboard.onTurnFinished(MOG, turn());
    clipboard.onTurnFinished(MOG, turn());
    await Promise.resolve();
    expect(writes).toEqual([]);
  });

  it('waits for the dictation-insertion gate before writing (the ordering race)', async () => {
    let held: (() => void) | undefined;
    const { deps, writes } = fakeDeps({
      whenClear: (fn) => {
        held = fn;
      },
    });
    createAnswerClipboard(deps).onTurnFinished(MOG, turn());
    expect(writes).toEqual([]);
    held?.();
    await Promise.resolve();
    expect(writes).toEqual(['The answer.']);
  });

  it('copies a combined summary with its own [n] markers stripped', async () => {
    const { deps, writes } = fakeDeps();
    createAnswerClipboard(deps).onTurnFinished(
      MOG,
      turn({
        parts: [
          {
            callId: 'c1',
            workspace: { name: 'Sales', colorVariant: 'coral' },
            sections: [{ id: 'answer', md: 'x' }],
          },
          {
            callId: 'c2',
            workspace: { name: 'Finance', colorVariant: 'rose' },
            sections: [{ id: 'answer', md: 'y' }],
          },
        ],
        summary: {
          md: 'Revenue grew [1].',
          kind: 'combined',
          workspaces: [],
          citations: [{ n: 1, callId: 'c1', workspace: { name: 'Sales', colorVariant: 'coral' } }],
        },
      }),
    );
    await Promise.resolve();
    expect(writes).toEqual(['Revenue grew.']);
  });
});

describe('createAnswerClipboard: onDirectAnswered (the direct Genie path, H2)', () => {
  it('copies the verbatim direct answer and reports "copied"', async () => {
    const { deps, directNotes, writes } = fakeDeps();
    createAnswerClipboard(deps).onDirectAnswered(MOG, 'op-1', 'Revenue was $5M.');
    await Promise.resolve();
    await Promise.resolve();
    expect(writes).toEqual(['Revenue was $5M.']);
    expect(directNotes).toEqual([{ mogId: MOG, opId: 'op-1', outcome: 'copied' }]);
  });

  it('never copies an empty answer', async () => {
    const { deps, writes } = fakeDeps();
    createAnswerClipboard(deps).onDirectAnswered(MOG, 'op-1', '   ');
    await Promise.resolve();
    expect(writes).toEqual([]);
  });

  it('copies exactly once per opId', async () => {
    const { deps, writes } = fakeDeps();
    const clipboard = createAnswerClipboard(deps);
    clipboard.onDirectAnswered(MOG, 'op-1', 'Revenue was $5M.');
    clipboard.onDirectAnswered(MOG, 'op-1', 'Revenue was $5M.');
    await Promise.resolve();
    await Promise.resolve();
    expect(writes).toEqual(['Revenue was $5M.']);
  });

  it('a turn key and a direct key with the same literal id never collide', async () => {
    const { deps, writes } = fakeDeps();
    const clipboard = createAnswerClipboard(deps);
    clipboard.onTurnFinished(MOG, turn({ turnId: 'same-id' }));
    clipboard.onDirectAnswered(MOG, 'same-id', 'A different direct answer.');
    await Promise.resolve();
    await Promise.resolve();
    expect(writes).toEqual(['The answer.', 'A different direct answer.']);
  });

  it('[ADR 0124 amendment] trusts the given host for the HTML form', async () => {
    const { deps, writes, htmls } = fakeDeps();
    createAnswerClipboard(deps).onDirectAnswered(
      MOG,
      'op-1',
      `Revenue was **$5M**. See [the report](https://${DIRECT_HOST}/x) for more.`,
      DIRECT_HOST,
    );
    await Promise.resolve();
    expect(writes).toEqual([
      'Revenue was $5M. See the report (https://sales.cloud.databricks.com/x) for more.',
    ]);
    expect(htmls[0]).toContain('<strong>$5M</strong>');
    expect(htmls[0]).toContain(`<a href="https://${DIRECT_HOST}/x">the report</a>`);
  });

  it('[ADR 0124 amendment] with no host, an https link is never trusted in the HTML form', async () => {
    const { deps, htmls } = fakeDeps();
    createAnswerClipboard(deps).onDirectAnswered(
      MOG,
      'op-1',
      `See [the report](https://${DIRECT_HOST}/x).`,
    );
    await Promise.resolve();
    expect(htmls[0]).not.toContain('<a ');
  });
});

describe('createAnswerClipboard: the ADR 0124 amendment (html plus the "Copy Markdown" button)', () => {
  it('onTurnFinished writes a plain-text form (markdown syntax removed) and an HTML form', async () => {
    const { deps, writes, htmls } = fakeDeps();
    createAnswerClipboard(deps).onTurnFinished(
      MOG,
      turn({
        parts: [
          {
            callId: 'c1',
            workspace: { name: 'Sales', colorVariant: 'coral' },
            sections: [
              { id: 'answer', md: 'Revenue grew **a lot**.' },
              { id: 'footer', workspaceHost: DIRECT_HOST },
            ],
          },
        ],
      }),
    );
    await Promise.resolve();
    expect(writes).toEqual(['Revenue grew a lot.']);
    expect(htmls[0]).toContain('<strong>a lot</strong>');
  });

  it('copyMarkdownNow writes the raw Markdown text only, with no html', async () => {
    const { deps, writes, htmls, turnNotes } = fakeDeps();
    createAnswerClipboard(deps).copyMarkdownNow(
      't1',
      turn({
        parts: [
          {
            callId: 'c1',
            workspace: { name: 'Sales', colorVariant: 'coral' },
            sections: [{ id: 'answer', md: 'Revenue grew **a lot**.' }],
          },
        ],
      }),
    );
    for (let i = 0; i < 4; i++) await Promise.resolve();
    expect(writes).toEqual(['Revenue grew **a lot**.']);
    expect(htmls).toEqual([undefined]);
    expect(turnNotes).toEqual([{ turnId: 't1', outcome: 'copied' }]);
  });

  it('copyMarkdownNow works even when the auto-copy toggle is off', async () => {
    const { deps, writes } = fakeDeps({ enabled: () => false });
    createAnswerClipboard(deps).copyMarkdownNow('t1', turn());
    await Promise.resolve();
    expect(writes).toEqual(['The answer.']);
  });

  it('copyMarkdownNow is a no-op for a withheld (quarantined) answer', async () => {
    const { deps, writes } = fakeDeps();
    createAnswerClipboard(deps).copyMarkdownNow(
      't1',
      turn({
        parts: [
          {
            callId: 'c1',
            workspace: { name: 'Sales', colorVariant: 'coral' },
            sections: [
              {
                id: 'answer',
                md: 'This answer was held back. It looked like it held instructions.',
              },
            ],
          },
        ],
      }),
    );
    await Promise.resolve();
    expect(writes).toEqual([]);
  });

  it('copyMarkdownNow can run again for the same turn (no once-per-turn guard)', async () => {
    const { deps, writes } = fakeDeps();
    const clipboard = createAnswerClipboard(deps);
    clipboard.copyMarkdownNow('t1', turn());
    clipboard.copyMarkdownNow('t1', turn());
    await Promise.resolve();
    expect(writes).toEqual(['The answer.', 'The answer.']);
  });
});

describe('createAnswerClipboard: copyNow and copyDirectNow (the owner\'s "Copy" button request)', () => {
  it('copyNow writes both text and html, reported through noteTurn', async () => {
    const { deps, writes, htmls, turnNotes } = fakeDeps();
    createAnswerClipboard(deps).copyNow(
      't1',
      turn({
        parts: [
          {
            callId: 'c1',
            workspace: { name: 'Sales', colorVariant: 'coral' },
            sections: [{ id: 'answer', md: 'Revenue grew **a lot**.' }],
          },
        ],
      }),
    );
    for (let i = 0; i < 4; i++) await Promise.resolve();
    expect(writes).toEqual(['Revenue grew a lot.']);
    expect(htmls[0]).toContain('<strong>a lot</strong>');
    expect(turnNotes).toEqual([{ turnId: 't1', outcome: 'copied' }]);
  });

  it('copyNow works even when the auto-copy toggle is off', async () => {
    const { deps, writes, htmls } = fakeDeps({ enabled: () => false });
    createAnswerClipboard(deps).copyNow('t1', turn());
    await Promise.resolve();
    expect(writes).toEqual(['The answer.']);
    expect(htmls[0]).toBeDefined();
  });

  it('copyNow can run again for the same turn (no once-per-turn guard)', async () => {
    const { deps, writes } = fakeDeps();
    const clipboard = createAnswerClipboard(deps);
    clipboard.copyNow('t1', turn());
    clipboard.copyNow('t1', turn());
    await Promise.resolve();
    expect(writes).toEqual(['The answer.', 'The answer.']);
  });

  it('copyNow is a no-op for a withheld (quarantined) answer', async () => {
    const { deps, writes } = fakeDeps();
    createAnswerClipboard(deps).copyNow(
      't1',
      turn({
        parts: [
          {
            callId: 'c1',
            workspace: { name: 'Sales', colorVariant: 'coral' },
            sections: [
              {
                id: 'answer',
                md: 'This answer was held back. It looked like it held instructions.',
              },
            ],
          },
        ],
      }),
    );
    await Promise.resolve();
    expect(writes).toEqual([]);
  });

  it('copyNow waits for the dictation-insertion gate before writing', async () => {
    let held: (() => void) | undefined;
    const { deps, writes } = fakeDeps({
      whenClear: (fn) => {
        held = fn;
      },
    });
    createAnswerClipboard(deps).copyNow('t1', turn());
    expect(writes).toEqual([]);
    held?.();
    await Promise.resolve();
    expect(writes).toEqual(['The answer.']);
  });

  it('copyDirectNow writes both text and html, reported through noteDirect', async () => {
    const { deps, writes, htmls, directNotes } = fakeDeps();
    createAnswerClipboard(deps).copyDirectNow(
      MOG,
      'op-1',
      `Revenue was **$5M**. See [the report](https://${DIRECT_HOST}/x) for more.`,
      DIRECT_HOST,
    );
    for (let i = 0; i < 4; i++) await Promise.resolve();
    expect(writes).toEqual([
      'Revenue was $5M. See the report (https://sales.cloud.databricks.com/x) for more.',
    ]);
    expect(htmls[0]).toContain('<strong>$5M</strong>');
    expect(htmls[0]).toContain(`<a href="https://${DIRECT_HOST}/x">the report</a>`);
    expect(directNotes).toEqual([{ mogId: MOG, opId: 'op-1', outcome: 'copied' }]);
  });

  it('copyDirectNow works even when the auto-copy toggle is off', async () => {
    const { deps, writes } = fakeDeps({ enabled: () => false });
    createAnswerClipboard(deps).copyDirectNow(MOG, 'op-1', 'Revenue was $5M.');
    await Promise.resolve();
    expect(writes).toEqual(['Revenue was $5M.']);
  });

  it('copyDirectNow can run again for the same opId (no once guard)', async () => {
    const { deps, writes } = fakeDeps();
    const clipboard = createAnswerClipboard(deps);
    clipboard.copyDirectNow(MOG, 'op-1', 'Revenue was $5M.');
    clipboard.copyDirectNow(MOG, 'op-1', 'Revenue was $5M.');
    await Promise.resolve();
    expect(writes).toEqual(['Revenue was $5M.', 'Revenue was $5M.']);
  });

  it('copyDirectNow never copies an empty answer', async () => {
    const { deps, writes } = fakeDeps();
    createAnswerClipboard(deps).copyDirectNow(MOG, 'op-1', '   ');
    await Promise.resolve();
    expect(writes).toEqual([]);
  });

  it('copyDirectNow with no host never trusts an https link in the HTML form', async () => {
    const { deps, htmls } = fakeDeps();
    createAnswerClipboard(deps).copyDirectNow(
      MOG,
      'op-1',
      `See [the report](https://${DIRECT_HOST}/x).`,
    );
    await Promise.resolve();
    expect(htmls[0]).not.toContain('<a ');
  });
});
