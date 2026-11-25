/** The owner rule for the answer of an agent turn, and its Genie One links. Fake values. */
import {
  finishedAnswerText,
  foldedUnderLone,
  genieOneLinks,
  isLoneAnswer,
  loneAnswerIndex,
  openGenieOneText,
  stripCitationMarkers,
  WITHHELD_TEXT,
  type AnswerPart,
} from './agent-answer-rule';
import type { AgentPartView, AgentTurnView } from './ipc';

const p = (workspace: string, o: Partial<AnswerPart> = {}): AnswerPart => ({
  answered: true,
  withheld: false,
  fromEarlier: false,
  workspace,
  ...o,
});

const part = (name: string, md: string, o: Partial<AgentPartView> = {}): AgentPartView => ({
  callId: `c-${name}`,
  workspace: { name, colorVariant: 'blue' },
  sections: [{ id: 'answer', md }],
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

describe('stripCitationMarkers', () => {
  it('removes a [n] marker and the one space before it, only when n is a known citation', () => {
    expect(stripCitationMarkers('Revenue grew 10% [1].', [1])).toBe('Revenue grew 10%.');
    expect(stripCitationMarkers('Two sources [1] [2] agree.', [1, 2])).toBe('Two sources agree.');
  });

  it('with no citation numbers, strips nothing', () => {
    expect(stripCitationMarkers('Revenue grew 10% [1].', [])).toBe('Revenue grew 10% [1].');
  });

  it('[M3] never strips real content that only looks like a marker', () => {
    // An array index right after a word character: no space, so the character before the
    // bracket is a word character, not the start of a marker.
    expect(stripCitationMarkers('See array[1] for the value.', [1])).toBe(
      'See array[1] for the value.',
    );
    // A year in brackets whose number is not an actual citation.
    expect(stripCitationMarkers('The event in [2024] mattered.', [1, 2])).toBe(
      'The event in [2024] mattered.',
    );
  });

  it('[M3] never strips inside a fenced or inline code span', () => {
    expect(stripCitationMarkers('Use `rows[1]` to read it.', [1])).toBe(
      'Use `rows[1]` to read it.',
    );
    expect(stripCitationMarkers('```\nrows[1]\n```', [1])).toBe('```\nrows[1]\n```');
    // Prose around the code still strips normally.
    expect(stripCitationMarkers('See [1]. Use `rows[1]` too. Also [1] again.', [1])).toBe(
      'See. Use `rows[1]` too. Also again.',
    );
  });

  it('keeps a real Markdown link that happens to start [n](...) or [n][ref]', () => {
    expect(stripCitationMarkers('See [1](https://x) for detail.', [1])).toBe(
      'See [1](https://x) for detail.',
    );
    expect(stripCitationMarkers('See [1][ref] for detail.', [1])).toBe('See [1][ref] for detail.');
  });
});

describe('finishedAnswerText (the owner\'s "attach Genie\'s answer to the clipboard" request)', () => {
  it('[owner rule] a lone answer copies verbatim, with no stripping', () => {
    const turn: Pick<AgentTurnView, 'summary' | 'parts'> = {
      parts: [part('sales', 'Revenue is $5M [not a marker].')],
    };
    expect(finishedAnswerText(turn)).toBe('Revenue is $5M [not a marker].');
  });

  it('a combined summary copies its md with its own [n] citation markers stripped', () => {
    const turn: Pick<AgentTurnView, 'summary' | 'parts'> = {
      summary: {
        md: 'Revenue grew [1] and headcount grew [2].',
        kind: 'combined',
        workspaces: [],
        citations: [
          { n: 1, callId: 'c-sales', workspace: { name: 'Sales', colorVariant: 'blue' } },
          { n: 2, callId: 'c-fin', workspace: { name: 'Finance', colorVariant: 'rose' } },
        ],
      },
      parts: [part('sales', 'x'), part('fin', 'y')],
    };
    expect(finishedAnswerText(turn)).toBe('Revenue grew and headcount grew.');
  });

  it('[M2] a lone answer wins over a summary that is also present', () => {
    const turn: Pick<AgentTurnView, 'summary' | 'parts'> = {
      summary: { md: 'The model text [1].', kind: 'note', workspaces: [] },
      parts: [part('sales', 'The real Genie answer.')],
    };
    expect(finishedAnswerText(turn)).toBe('The real Genie answer.');
  });

  it('never copies a withheld (quarantined) lone answer', () => {
    const turn: Pick<AgentTurnView, 'summary' | 'parts'> = {
      parts: [part('sales', WITHHELD_TEXT)],
    };
    expect(finishedAnswerText(turn)).toBeUndefined();
  });

  it('never copies when the model note was hidden and no lone answer exists', () => {
    const turn: Pick<AgentTurnView, 'summary' | 'parts'> = {
      parts: [part('sales', 'a'), part('fin', 'b')],
    };
    expect(finishedAnswerText(turn)).toBeUndefined();
  });

  it('several calls to the same workspace: the last one is the answer', () => {
    const turn: Pick<AgentTurnView, 'summary' | 'parts'> = {
      parts: [part('logfood', 'first try'), part('logfood', 'final answer')],
    };
    expect(finishedAnswerText(turn)).toBe('final answer');
  });
});
