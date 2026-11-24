/** ADR 0122: the brain maps `[n]` markers of the summary to citations. Fake values. */
import type { AgentPartView } from '../shared/ipc';
import { citationsOf } from './agent-citations';

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
const src = [{ callId: 'c-1' }, { callId: 'c-2' }];
const both = parts(part('c-1', sales), part('c-2', fin));

describe('citationsOf', () => {
  it('maps a valid marker to the call, the chip and the checked link', () => {
    expect(citationsOf('Sales is up [1].', src, both)).toEqual([
      {
        n: 1,
        callId: 'c-1',
        workspace: sales,
        deepLink: 'https://sales.cloud.databricks.com/one/chat/threads/c-1',
      },
    ]);
  });

  it('keeps one citation for a repeated marker', () => {
    expect(citationsOf('A [1] and again [1].', src, both).map((c) => c.n)).toEqual([1]);
  });

  it('reads adjacent markers [1][2] as two citations, in order of n', () => {
    expect(citationsOf('Both [2][1].', src, both).map((c) => c.n)).toEqual([1, 2]);
  });

  it('ignores out of range markers, [x] and [10] with two sources', () => {
    expect(citationsOf('See [x], [10], [0] and [3].', src, both)).toEqual([]);
  });

  it('ignores a marker whose part was withheld, and a source with no part', () => {
    const p = parts(part('c-1', sales, { notShownToAgent: true }));
    expect(citationsOf('A [1] B [2].', src, p)).toEqual([]);
  });

  it('does not take a link label [1](url) for a marker', () => {
    expect(citationsOf('[1](https://x.example) text', src, both)).toEqual([]);
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
    expect(citationsOf('A [1].', src, parts(bad))[0]).not.toHaveProperty('deepLink');
  });
});
