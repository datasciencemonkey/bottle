/** The owner's "attach Genie's answer to the clipboard" request. Fake values. */
import type { AgentTurnView } from '../../shared/ipc';
import {
  createAnswerClipboard,
  type AnswerClipboardDeps,
  type CopyOutcome,
} from './answer-clipboard';

const MOG = 'a1';

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
  const faults: number[] = [];
  const deps: AnswerClipboardDeps = {
    enabled: () => true,
    write: async (text) => {
      writes.push(text);
      return 'written';
    },
    whenClear: (fn) => fn(),
    noteTurn: (turnId, outcome) => turnNotes.push({ turnId, outcome }),
    noteDirect: (mogId, opId, outcome) => directNotes.push({ mogId, opId, outcome }),
    onFault: () => faults.push(1),
    ...o,
  };
  return { deps, turnNotes, directNotes, writes, faults };
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
      await Promise.resolve();
      await Promise.resolve();
      expect(turnNotes).toEqual([{ turnId: 't1', outcome: 'failed' }]);
    }
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
});
