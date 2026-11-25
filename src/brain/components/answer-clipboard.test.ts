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
  const notes: { mogId: string; turnId: string; outcome: CopyOutcome }[] = [];
  const writes: string[] = [];
  const faults: number[] = [];
  const deps: AnswerClipboardDeps = {
    enabled: () => true,
    write: async (text) => {
      writes.push(text);
      return 'written';
    },
    whenClear: (fn) => fn(),
    note: (mogId, t, outcome) => notes.push({ mogId, turnId: t.turnId, outcome }),
    onFault: () => faults.push(1),
    ...o,
  };
  return { deps, notes, writes, faults };
}

describe('createAnswerClipboard', () => {
  it('[owner request] copies the finished answer and reports "copied"', async () => {
    const { deps, notes, writes } = fakeDeps();
    const clipboard = createAnswerClipboard(deps);
    clipboard.onTurn(MOG, turn());
    await Promise.resolve();
    await Promise.resolve();
    expect(writes).toEqual(['The answer.']);
    expect(notes).toEqual([{ mogId: MOG, turnId: 't1', outcome: 'copied' }]);
  });

  it('reports "failed" when the write answers too_large or unsupported', async () => {
    for (const result of ['too_large', 'unsupported'] as const) {
      const { deps, notes } = fakeDeps({ write: async () => result });
      createAnswerClipboard(deps).onTurn(MOG, turn());
      await Promise.resolve();
      await Promise.resolve();
      expect(notes).toEqual([{ mogId: MOG, turnId: 't1', outcome: 'failed' }]);
    }
  });

  it('reports "failed" and calls onFault when the write throws, never with the answer text', async () => {
    const { deps, notes, faults } = fakeDeps({
      write: async () => {
        throw new Error('fake: the answer, leaked');
      },
    });
    createAnswerClipboard(deps).onTurn(MOG, turn());
    await Promise.resolve();
    await Promise.resolve();
    expect(faults).toEqual([1]);
    expect(notes).toEqual([{ mogId: MOG, turnId: 't1', outcome: 'failed' }]);
  });

  it('never copies a turn that is not done (running, stopped, failed, needs_you)', async () => {
    for (const state of [
      'planning',
      'running',
      'slow',
      'stopped',
      'failed',
      'needs_you',
    ] as const) {
      const { deps, writes } = fakeDeps();
      createAnswerClipboard(deps).onTurn(MOG, turn({ state }));
      await Promise.resolve();
      expect(writes).toEqual([]);
    }
  });

  it('never copies a withheld (quarantined) lone answer', async () => {
    const { deps, writes } = fakeDeps();
    createAnswerClipboard(deps).onTurn(
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

  it('copies exactly once per finished turn, even if showTurn fires again with the same state', async () => {
    const { deps, writes } = fakeDeps();
    const clipboard = createAnswerClipboard(deps);
    const t = turn();
    clipboard.onTurn(MOG, t);
    clipboard.onTurn(MOG, t);
    clipboard.onTurn(MOG, t);
    await Promise.resolve();
    await Promise.resolve();
    expect(writes).toEqual(['The answer.']);
  });

  it('the toggle off means no copy, but still marks the turn handled (no later retry)', async () => {
    const { deps, writes } = fakeDeps({ enabled: () => false });
    const clipboard = createAnswerClipboard(deps);
    clipboard.onTurn(MOG, turn());
    clipboard.onTurn(MOG, turn());
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
    createAnswerClipboard(deps).onTurn(MOG, turn());
    expect(writes).toEqual([]);
    held?.();
    await Promise.resolve();
    expect(writes).toEqual(['The answer.']);
  });

  it('copies a combined summary with its [n] markers stripped', async () => {
    const { deps, writes } = fakeDeps();
    createAnswerClipboard(deps).onTurn(
      MOG,
      turn({
        summary: {
          md: 'Revenue grew [1].',
          kind: 'combined',
          workspaces: [{ name: 'Sales', colorVariant: 'coral' }],
        },
      }),
    );
    await Promise.resolve();
    expect(writes).toEqual(['Revenue grew.']);
  });
});
