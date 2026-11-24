/** ADR 0122: the brain maps `[n]` markers of the summary to citations. Fake values. */
import type { AgentPartView } from '../shared/ipc';
import { citationsByWorkspace } from './agent-citations';

const sales = { name: 'sales', colorVariant: 'sky' };
const fin = { name: 'finance', colorVariant: 'rose' };
const part = (callId: string, workspace: typeof sales, o: Partial<AgentPartView> = {}) =>
  ({
    callId,
    workspace,
    sections: [
      { id: 'answer', md: 'x' },
      {
        id: 'footer',
        deepLink: `https://sales.cloud.databricks.com/one/chat/threads/${callId}`,
        workspaceHost: 'sales.cloud.databricks.com',
      },
    ],
    ...o,
  }) as AgentPartView;
const parts = (...ps: AgentPartView[]) => new Map(ps.map((p) => [p.callId, p]));
const src = [
  { callId: 'c-1', workspaceId: 'sales' },
  { callId: 'c-2', workspaceId: 'finance' },
];
const both = parts(part('c-1', sales), part('c-2', fin));
const cites = (md: string, s = src, p = both) => citationsByWorkspace(md, s, p);

describe('citationsByWorkspace', () => {
  it('maps a valid marker to the call, the chip and the checked link', () => {
    expect(cites('Sales is up [1].').citations).toEqual([
      {
        n: 1,
        callId: 'c-1',
        workspace: sales,
        deepLink: 'https://sales.cloud.databricks.com/one/chat/threads/c-1',
        workspaceHost: 'sales.cloud.databricks.com',
      },
    ]);
  });

  it('keeps one citation for a repeated marker', () => {
    expect(cites('A [1] and again [1].').citations.map((c) => c.n)).toEqual([1]);
  });

  it('numbers the workspaces in the order the text first cites them', () => {
    const got = cites('Both [2][1].');
    expect(got.md).toBe('Both [1][2].');
    expect(got.citations.map((c) => [c.n, c.workspace.name])).toEqual([
      [1, 'finance'],
      [2, 'sales'],
    ]);
    expect(got.workspaceIds).toEqual(['finance', 'sales']);
  });

  it('gives two calls to the same workspace one number, reaching the last call (owner bug)', () => {
    const s = [
      { callId: 'c-1', workspaceId: 'logfood' },
      { callId: 'c-2', workspaceId: 'logfood' },
    ];
    const lf = { name: 'logfood', colorVariant: 'sky' };
    const got = cites('Top is X [1] [2].', s, parts(part('c-1', lf), part('c-2', lf)));
    expect(got.md).toBe('Top is X [1] [1].');
    expect(got.citations).toHaveLength(1);
    expect(got.citations[0]).toMatchObject({ n: 1, callId: 'c-2', workspace: lf });
    expect(got.citations[0]?.deepLink).toContain('/c-2');
    expect(got.workspaceIds).toEqual(['logfood']);
  });

  it('drops out of range markers from the text, and leaves [x] alone', () => {
    const got = cites('See [x] and [3]. Then [10] [0] more.');
    expect(got.citations).toEqual([]);
    expect(got.md).toBe('See [x] and. Then more.');
  });

  it('drops a marker whose part was withheld, and a source with no part', () => {
    const got = cites('A [1] B [2].', src, parts(part('c-1', sales, { notShownToAgent: true })));
    expect(got.citations).toEqual([]);
    expect(got.md).toBe('A B.');
  });

  it('does not take a link label [1](url) for a marker', () => {
    const got = cites('[1](https://x.example) text');
    expect(got.citations).toEqual([]);
    expect(got.md).toBe('[1](https://x.example) text');
  });

  it('drops a deep link that fails the trust rule', () => {
    const bad = part('c-1', sales);
    bad.sections = [
      {
        id: 'footer',
        deepLink: 'https://evil.example/x',
        workspaceHost: 'sales.cloud.databricks.com',
      },
    ];
    expect(cites('A [1].', src, parts(bad)).citations[0]).not.toHaveProperty('deepLink');
  });
});
