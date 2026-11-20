// @vitest-environment jsdom
/** #523: GFM tables in safe markdown: parse, align, cap, format numbers, stay safe. */
import { renderToStaticMarkup } from 'react-dom/server';
import { MAX_TABLE_COLS, MAX_TABLE_ROWS, SafeMarkdown, splitRow } from './safe-markdown';

const trust = { profileHost: 'h.test', allowedAppUrls: [] };
function render(md: string, extra: Partial<Parameters<typeof SafeMarkdown>[0]> = {}) {
  const el = document.createElement('div');
  el.innerHTML = renderToStaticMarkup(
    <SafeMarkdown text={md} trust={trust} locale="en-US" {...extra} />,
  );
  return el;
}
const cells = (el: Element, sel: string) => [...el.querySelectorAll(sel)].map((c) => c.textContent);

const TABLE = `| sfdc_account_name | start_date | end_date | total_dbu_dollars | all_total |
| --- | --- | --- | --- | --- |
| Apple | 2026-11-01 | 2026-11-18 | 1220042.3509332137 | 3.62387909843917E8 |`;

describe('splitRow (#523)', () => {
  it('splits on pipes and drops the edge pipes', () => {
    expect(splitRow('| a | b | c |')).toEqual(['a', 'b', 'c']);
    expect(splitRow('a | b | c')).toEqual(['a', 'b', 'c']);
    expect(splitRow('|a||c|')).toEqual(['a', '', 'c']);
  });

  it('keeps an escaped pipe in the cell', () => {
    expect(splitRow('| a \\| b | c |')).toEqual(['a | b', 'c']);
  });

  it('keeps a pipe inside an inline code span in the cell', () => {
    expect(splitRow('| `a | b` | c |')).toEqual(['`a | b`', 'c']);
  });

  it('a lone backtick is not a code span', () => {
    expect(splitRow('| it`s | b |')).toEqual(['it`s', 'b']);
  });

  it('stops after one more than the column cap', () => {
    const wide = `|${Array.from({ length: 500 }, (_, i) => `c${i}`).join('|')}|`;
    expect(splitRow(wide).length).toBe(MAX_TABLE_COLS + 1);
  });
});

describe('tables in safe markdown (#523)', () => {
  it('renders a real table with a header row and a body row', () => {
    const el = render(TABLE);
    expect(el.querySelector('table.md-table')).not.toBeNull();
    expect(cells(el, 'thead th')).toEqual([
      'sfdc_account_name',
      'start_date',
      'end_date',
      'total_dbu_dollars',
      'all_total',
    ]);
    expect(el.querySelectorAll('tbody tr')).toHaveLength(1);
    expect(el.querySelector('thead th')?.getAttribute('scope')).toBe('col');
    // A scroll box with a name, so a wide table can be reached by keyboard.
    const wrap = el.querySelector('.md-table-wrap');
    expect(wrap?.getAttribute('role')).toBe('region');
    expect(wrap?.getAttribute('tabindex')).toBe('0');
  });

  it('shows long floats and scientific notation as readable numbers, with the raw value in the title', () => {
    const el = render(TABLE);
    const tds = [...el.querySelectorAll('tbody td')];
    expect(tds.map((c) => c.textContent)).toEqual([
      'Apple',
      '2026-11-01',
      '2026-11-18',
      '1,220,042.35',
      '362,387,909.84',
    ]);
    expect(tds[3]?.getAttribute('title')).toBe('1220042.3509332137');
    expect(tds[4]?.getAttribute('title')).toBe('3.62387909843917E8');
    // Text and dates keep their value and get no tooltip.
    expect(tds[0]?.hasAttribute('title')).toBe(false);
    expect(tds[1]?.hasAttribute('title')).toBe(false);
  });

  it('keeps small rates distinct, and decides per column (a mixed column is left alone)', () => {
    const el = render(`| name | ctr | note |
| - | - | - |
| a | 0.0072 | 1200 |
| b | 0.0149 | n/a |
| c | 0.006 | 3400.5678 |`);
    expect(cells(el, 'tbody td:nth-child(2)')).toEqual(['0.0072', '0.0149', '0.006']);
    expect(cells(el, 'tbody td:nth-child(3)')).toEqual(['1200', 'n/a', '3400.5678']);
    expect(el.querySelector('tbody td:nth-child(2)')?.getAttribute('data-align')).toBe('right');
    expect(el.querySelector('tbody td:nth-child(3)')?.getAttribute('data-align')).toBe('left');
  });

  it('does not touch numbers in sentences', () => {
    const el = render(`Total was 1220042.3509332137 today.\n\n${TABLE}`);
    expect(el.querySelector('p')?.textContent).toBe('Total was 1220042.3509332137 today.');
  });

  it('aligns numeric columns right by default and honours the separator alignment', () => {
    const el = render(`| a | b | c | d |
| :-- | --: | :-: | --- |
| x | y | z | 1.5 |
| x | y | z | 2 |`);
    const ths = [...el.querySelectorAll('thead th')].map((t) => t.getAttribute('data-align'));
    // a: left, b: right, c: center, d: numeric column, so right.
    expect(ths).toEqual(['left', 'right', 'center', 'right']);
    expect(
      [...el.querySelectorAll('tbody tr:first-child td')].map((t) => t.getAttribute('data-align')),
    ).toEqual(['left', 'right', 'center', 'right']);
  });

  it('an explicit left alignment beats a numeric column', () => {
    const el = render('| n |\n| :-- |\n| 5 |');
    expect(el.querySelector('td')?.getAttribute('data-align')).toBe('left');
  });

  it('a column with text and numbers stays left', () => {
    const el = render('| n |\n| --- |\n| 5 |\n| five |');
    expect(el.querySelector('th')?.getAttribute('data-align')).toBe('left');
  });

  it('pads a short row and drops extra cells of a ragged row', () => {
    const el = render('| a | b | c |\n| - | - | - |\n| 1 |\n| 1 | 2 | 3 | 4 | 5 |');
    const rows = [...el.querySelectorAll('tbody tr')].map((r) => cells(r, 'td'));
    expect(rows).toEqual([
      ['1', '', ''],
      ['1', '2', '3'],
    ]);
  });

  it('keeps an escaped pipe and a pipe in inline code inside one cell', () => {
    const el = render('| a | b |\n| - | - |\n| x \\| y | `p | q` |');
    expect(cells(el, 'tbody td')).toEqual(['x | y', 'p | q']);
    expect(el.querySelector('tbody td code')?.textContent).toBe('p | q');
  });

  it('works without edge pipes', () => {
    const el = render('a | b\n- | -\n1 | 2');
    expect(cells(el, 'thead th')).toEqual(['a', 'b']);
    expect(cells(el, 'tbody td')).toEqual(['1', '2']);
  });

  it('needs a separator row with the same number of cells, else it is a paragraph', () => {
    expect(render('| a | b |\n| --- |\n| 1 | 2 |').querySelector('table')).toBeNull();
    expect(render('| a | b |\n| x | y |').querySelector('table')).toBeNull();
    expect(render('a | b\n--- | ---').querySelector('table')).not.toBeNull();
    // A line with no pipe is no header.
    expect(render('a\n---').querySelector('table')).toBeNull();
  });

  it('ends at the first line with no pipe, and the text after it renders', () => {
    const el = render(`${TABLE}\n\nAfter the table.`);
    expect(el.querySelectorAll('tbody tr')).toHaveLength(1);
    expect(el.querySelector('p')?.textContent).toBe('After the table.');
  });

  it('a table can follow a paragraph line with no blank line between', () => {
    const el = render('Intro line\n| a | b |\n| - | - |\n| 1 | 2 |');
    expect(el.querySelector('p')?.textContent).toBe('Intro line');
    expect(el.querySelector('table')).not.toBeNull();
  });

  it('renders inline markdown in a cell, and a trusted link in a cell opens through the handler', () => {
    const el = render('| a |\n| - |\n| **bold** and https://h.test/x |');
    expect(el.querySelector('td strong')?.textContent).toBe('bold');
    expect(el.querySelector('td a')?.getAttribute('href')).toBe('https://h.test/x');
  });

  it('shows no HTML from a cell, a header or a link label', () => {
    const el = render(
      '| <img src=x onerror=alert(1)> | <script>x</script> |\n| - | - |\n| <b onclick="x()">y</b> | [<i>l</i>](https://evil.test/) |',
    );
    expect(el.querySelector('img, script, b, i, [onclick], [onerror]')).toBeNull();
    expect(el.textContent).toContain('<img src=x onerror=alert(1)>');
    expect(el.textContent).toContain('<script>x</script>');
  });

  describe('caps', () => {
    const many = (n: number) =>
      `| a | b |\n| - | - |\n${Array.from({ length: n }, (_, i) => `| r${i} | ${i} |`).join('\n')}`;

    it('draws at most MAX_TABLE_ROWS body rows and says how many are not drawn', () => {
      const el = render(many(MAX_TABLE_ROWS + 7), {
        overflowHint: 'Open in Bottle to see them all',
      });
      expect(el.querySelectorAll('tbody tr')).toHaveLength(MAX_TABLE_ROWS);
      expect(el.querySelector('[data-table-more]')?.textContent).toBe(
        '7 more rows. Open in Bottle to see them all',
      );
    });

    it('says "1 more row" for one and draws no note at the cap', () => {
      expect(render(many(MAX_TABLE_ROWS + 1)).querySelector('[data-table-more]')?.textContent).toBe(
        '1 more row',
      );
      expect(render(many(MAX_TABLE_ROWS)).querySelector('[data-table-more]')).toBeNull();
    });

    it('a caller can raise the row cap', () => {
      const el = render(many(120), { maxTableRows: 200 });
      expect(el.querySelectorAll('tbody tr')).toHaveLength(120);
    });

    it('draws at most MAX_TABLE_COLS columns', () => {
      const n = MAX_TABLE_COLS + 10;
      const head = `| ${Array.from({ length: n }, (_, i) => `h${i}`).join(' | ')} |`;
      const sep = `| ${Array.from({ length: n }, () => '-').join(' | ')} |`;
      const row = `| ${Array.from({ length: n }, (_, i) => i).join(' | ')} |`;
      const el = render(`${head}\n${sep}\n${row}`);
      expect(el.querySelectorAll('thead th')).toHaveLength(MAX_TABLE_COLS);
      expect(el.querySelectorAll('tbody td')).toHaveLength(MAX_TABLE_COLS);
    });
  });

  it('does not run slowly on a hostile table-shaped input', () => {
    const bad = '|'.repeat(20_000);
    const t = performance.now();
    render(`${bad}\n${'-|'.repeat(10_000)}\n${bad}`);
    render(`a|b\n${'-|-'.repeat(7_000)}\n${'|`'.repeat(10_000)}`);
    expect(performance.now() - t).toBeLessThan(2_000);
  });
});
