/** The owner rule for the answer of an agent turn, and its Genie One links. Fake values. */
import {
  foldedUnderLone,
  genieOneLinks,
  isLoneAnswer,
  loneAnswerIndex,
  openGenieOneText,
  type AnswerPart,
} from './agent-answer-rule';

const p = (workspace: string, o: Partial<AnswerPart> = {}): AnswerPart => ({
  answered: true,
  withheld: false,
  fromEarlier: false,
  workspace,
  ...o,
});

describe('loneAnswerIndex', () => {
  it('[owner rule] exactly one answered call is the answer', () => {
    expect(loneAnswerIndex([p('sales')])).toBe(0);
    expect(loneAnswerIndex([p('sales', { withheld: true }), p('fin')])).toBe(1);
    expect(loneAnswerIndex([p('sales', { fromEarlier: true }), p('fin')])).toBe(1);
    expect(loneAnswerIndex([p('sales', { answered: false }), p('fin')])).toBe(1);
  });

  it('[owner bug] several answered calls to one workspace: the last one is the answer', () => {
    const parts = [p('logfood'), p('logfood')];
    expect(loneAnswerIndex(parts)).toBe(1);
    expect(isLoneAnswer(parts)).toBe(true);
    expect([...foldedUnderLone(parts)]).toEqual([0]);
    // A withheld part of another workspace does not count, and stays in view.
    const mixed = [p('logfood'), p('fin', { withheld: true }), p('logfood')];
    expect(loneAnswerIndex(mixed)).toBe(2);
    expect([...foldedUnderLone(mixed)]).toEqual([0]);
  });

  it('[owner rule] answered calls to two workspaces, or none: the text of the model is the answer', () => {
    expect(loneAnswerIndex([p('sales'), p('fin')])).toBe(-1);
    expect(loneAnswerIndex([p('sales'), p('fin'), p('sales')])).toBe(-1);
    expect(loneAnswerIndex([])).toBe(-1);
    expect(loneAnswerIndex([p('sales', { answered: false })])).toBe(-1);
    expect(foldedUnderLone([p('sales'), p('fin')]).size).toBe(0);
  });
});

describe('genieOneLinks', () => {
  const host = 'sales.cloud.databricks.test';
  const dl = (path: string, h = host) => `https://${h}/one/chat/threads/${path}`;

  it('[owner bug] one link for each workspace, in first-seen order: its newest trusted link', () => {
    const got = genieOneLinks([
      { name: 'Sales', deepLink: dl('a'), workspaceHost: host },
      { name: 'Finance', deepLink: dl('f', 'fin.test'), workspaceHost: 'fin.test' },
      { name: 'Sales', deepLink: dl('b'), workspaceHost: host },
      { name: 'Sales', deepLink: 'https://evil.example/x', workspaceHost: host },
    ]);
    expect(got.map((l) => [l.name, l.url])).toEqual([
      ['Sales', dl('b')],
      ['Finance', dl('f', 'fin.test')],
    ]);
  });

  it('gives no link to a workspace with no trusted deep link (no URL is built from ids)', () => {
    expect(genieOneLinks([{ name: 'Sales', workspaceHost: host }])).toEqual([]);
    expect(genieOneLinks([{ name: 'Sales', deepLink: dl('a') }])).toEqual([]);
  });

  it('names the link "Open Genie One in <workspace>"', () => {
    expect(openGenieOneText('logfood')).toBe('Open Genie One in logfood');
  });
});
