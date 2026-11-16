import { MEANING_CATEGORIES, MEANING_CORPUS, meaningFailures, tokens } from './meaning';

const item = (id: string) => {
  const it = MEANING_CORPUS.find((x) => x.id === id);
  if (!it) throw new Error(`no corpus item ${id}`);
  return it;
};

describe('AT-05 meaning corpus and oracle', () => {
  it('[AT-22] the corpus has 5 or more items of each AT-05 category', () => {
    expect(MEANING_CATEGORIES).toEqual([
      'numbers',
      'units',
      'negation',
      'names',
      'table_names',
      'self_corrections',
    ]);
    for (const c of MEANING_CATEGORIES) {
      expect(MEANING_CORPUS.filter((x) => x.category === c).length, c).toBeGreaterThanOrEqual(5);
    }
    const ids = MEANING_CORPUS.map((x) => x.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('[AT-22] every kept token is in its raw text, and the raw text passes the oracle', () => {
    for (const x of MEANING_CORPUS) {
      expect(x.keep.length, x.id).toBeGreaterThan(0);
      expect(meaningFailures(x, { text: x.raw, usedRaw: true }), x.id).toEqual([]);
      expect(meaningFailures(x, { text: x.raw, usedRaw: false }), x.id).toEqual([]);
    }
  });

  it('[AT-22] tokens keep decimals, percents and table names and drop thousands commas', () => {
    expect(tokens('Up 3.5% on main.sales_orders, 1,200 rows.')).toEqual([
      'up',
      '3.5%',
      'on',
      'main.sales_orders',
      '1200',
      'rows',
    ]);
  });

  it('[AT-22] the oracle names a changed number, unit, negation, name and table', () => {
    const cases: [string, string, string][] = [
      ['num-1', '1,250', '1,205'],
      ['unit-1', 'ms', 's'],
      ['neg-1', 'not', ''],
      ['name-1', 'Priya Raman', 'Priya Ramen'],
      ['table-1', 'main.finance.invoices', 'main.finance.invoice'],
    ];
    for (const [id, from, to] of cases) {
      const x = item(id);
      const out = meaningFailures(x, { text: x.raw.replace(from, to), usedRaw: false });
      expect(out.length, id).toBeGreaterThan(0);
      expect(out.join(' '), id).toContain(id);
    }
  });

  it('[AT-22] a self-correction must keep the corrected value', () => {
    const x = item('fix-1');
    expect(meaningFailures(x, { text: 'Set the limit to 500 rows.', usedRaw: false })).toEqual([]);
    expect(meaningFailures(x, { text: 'Set the limit to 50 rows.', usedRaw: false })).not.toEqual(
      [],
    );
  });

  it('[AT-22] the oracle flags a bad length ratio, a new URL, new code and a fake raw', () => {
    const x = item('num-1');
    expect(meaningFailures(x, { text: `${x.raw} ${x.raw}`, usedRaw: false }).join(' ')).toContain(
      'length',
    );
    expect(
      meaningFailures(x, { text: `${x.raw} https://evil.example/x`, usedRaw: false }).join(' '),
    ).toContain('URL');
    expect(meaningFailures(x, { text: `${x.raw} \`rm\``, usedRaw: false }).join(' ')).toContain(
      'code',
    );
    expect(meaningFailures(x, { text: 'something else', usedRaw: true }).join(' ')).toContain(
      'raw',
    );
  });

  it('[AT-22] a failure names the item and never the text', () => {
    const x = item('name-1');
    const out = meaningFailures(x, { text: 'nothing like it', usedRaw: false }).join(' ');
    expect(out).toContain('name-1');
    expect(out).not.toContain('nothing like it');
  });
});
