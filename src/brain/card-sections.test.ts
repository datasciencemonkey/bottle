import {
  PREVIEW_CELL_CHARS,
  PREVIEW_COLUMNS,
  PREVIEW_ROWS,
  SECTION_ORDER,
  buildSections,
  tableToCsv,
  tableToMarkdown,
  type SectionInput,
} from './card-sections';

const table = {
  columns: ['region', 'revenue'],
  rows: [
    ['EMEA', '4.2M'],
    ['APJ', '1.1M'],
  ],
};
const full: SectionInput = {
  answerMd: 'Revenue was **4.2M** in Q3.',
  keyNumbers: ['4.2M', '1.1M'],
  sql: 'SELECT region, revenue FROM sales',
  table,
  sentToGenie: 'Context sent: 120 characters',
  canResend: true,
  sttBadge: 'STT: local',
  deepLink: 'https://example.cloud.databricks.com/genie/rooms/1',
  dataFlowText: 'Audio stays on this Mac. The question goes to your workspace.',
};

const omitSent = ({ sentToGenie: _s, ...rest }: SectionInput): SectionInput => rest;

describe('expanded card sections', () => {
  it('[UX-18] [AT-09] shows the sections of Table UX-EXP in that order', () => {
    expect(SECTION_ORDER).toEqual([
      'answer',
      'keyNumbers',
      'sql',
      'tablePreview',
      'sentToGenie',
      'resend',
      'copy',
      'footer',
      'dataFlow',
    ]);
    expect(buildSections(full).map((s) => s.id)).toEqual([...SECTION_ORDER]);
  });

  it('[UX-18] keeps the order when sections are missing, and always has footer and data flow', () => {
    const ids = buildSections({ answerMd: 'x', dataFlowText: 'd' }).map((s) => s.id);
    expect(ids).toEqual(['answer', 'footer', 'dataFlow']);
    expect(buildSections({ dataFlowText: 'd' }).map((s) => s.id)).toEqual(['footer', 'dataFlow']);
  });

  it('[UX-18] [AT-09] the SQL section is collapsed and key numbers are verbatim', () => {
    const s = buildSections(full);
    expect(s.find((x) => x.id === 'sql')).toMatchObject({ collapsed: true, sql: full.sql });
    expect(s.find((x) => x.id === 'keyNumbers')).toMatchObject({ items: ['4.2M', '1.1M'] });
  });

  it('[UX-18] shows Sent to Genie and Resend only when context was sent', () => {
    const ids = buildSections({ ...omitSent(full), canResend: false }).map((s) => s.id);
    expect(ids).not.toContain('sentToGenie');
    expect(ids).not.toContain('resend');
  });

  it('[UX-18] the copy section offers Markdown and CSV only with a table', () => {
    expect(buildSections(full).find((x) => x.id === 'copy')).toMatchObject({
      formats: ['markdown', 'csv'],
    });
    expect(buildSections({ answerMd: 'x', dataFlowText: 'd' }).map((s) => s.id)).not.toContain(
      'copy',
    );
  });

  it('[UX-18] the table preview caps the rows and says how many are left', () => {
    const rows = Array.from({ length: 80 }, (_, i) => [`r${i}`, `${i}`]);
    const s = buildSections({ table: { columns: ['a', 'b'], rows }, dataFlowText: 'd' }).find(
      (x) => x.id === 'tablePreview',
    );
    expect(s).toMatchObject({ rows: rows.slice(0, PREVIEW_ROWS), moreRows: 80 - PREVIEW_ROWS });
  });

  it('[UX-18] copies a table as Markdown and as CSV, with every row', () => {
    expect(tableToMarkdown(table)).toBe(
      '| region | revenue |\n| --- | --- |\n| EMEA | 4.2M |\n| APJ | 1.1M |',
    );
    expect(tableToCsv(table)).toBe('region,revenue\r\nEMEA,4.2M\r\nAPJ,1.1M');
  });

  it('[UX-18] escapes pipes in Markdown, and quotes and formulas in CSV', () => {
    const t = { columns: ['a|b', 'c'], rows: [['x"y,z', '=SUM(A1)']] };
    expect(tableToMarkdown(t)).toContain('a\\|b');
    expect(tableToCsv(t)).toBe(`a|b,c\r\n"x""y,z",'=SUM(A1)`);
  });

  it('[UX-18] the preview shows 50 rows (getQueryResult maxRows 50), 20 columns, 200 characters', () => {
    expect([PREVIEW_ROWS, PREVIEW_COLUMNS, PREVIEW_CELL_CHARS]).toEqual([50, 20, 200]);
    const columns = Array.from({ length: 25 }, (_, i) => `c${i}`);
    const t = { columns, rows: [columns.map(() => 'x'.repeat(300))] };
    const s = buildSections({ table: t, dataFlowText: 'd' }).find((x) => x.id === 'tablePreview');
    expect(s).toMatchObject({ moreColumns: 5 });
    if (s?.id !== 'tablePreview') throw new Error('no preview');
    expect(s.columns).toHaveLength(20);
    expect(s.rows[0]).toHaveLength(20);
    expect(s.rows[0]?.[0]).toBe(`${'x'.repeat(199)}\u2026`);
  });

  it.each([
    ["-1+cmd|' /C calc'!A0", `'-1+cmd|' /C calc'!A0`],
    ['-cmd', "'-cmd"],
    ['+1+cmd', "'+1+cmd"],
    ['=1+1', "'=1+1"],
    ['@SUM(A1)', "'@SUM(A1)"],
    ['\tcmd', "'\tcmd"],
  ])('[UX-18] CSV guards the cell %j', (cell, want) => {
    const out = tableToCsv({ columns: ['h'], rows: [[cell]] }).split('\r\n')[1];
    expect(out).toBe(want);
  });

  it('[UX-18] CSV guards a cell that starts with CR, and quotes it', () => {
    expect(tableToCsv({ columns: ['h'], rows: [['\rcmd']] })).toBe('h\r\n"\'\rcmd"');
  });

  it('[UX-18] CSV guards a header cell too', () => {
    expect(tableToCsv({ columns: ['=HYPERLINK("x")'], rows: [] })).toBe(`"'=HYPERLINK(""x"")"`);
  });

  it.each(['-5', '-1.5', '+3', '.5', '-0.5e3', '42'])(
    '[UX-18] CSV leaves the pure number %s as it is',
    (n) => {
      expect(tableToCsv({ columns: ['h'], rows: [[n]] }).split('\r\n')[1]).toBe(n);
    },
  );

  it.each(['-5abc', '-', '+', '-1-1', '--1'])('[UX-18] CSV guards %s (not a number)', (n) => {
    expect(tableToCsv({ columns: ['h'], rows: [[n]] }).split('\r\n')[1]).toBe(`'${n}`);
  });

  it('[UX-18] Markdown escapes backslash and backtick as well as the pipe', () => {
    const t = { columns: ['a\\b', 'c`d'], rows: [['x|y', '`z`']] };
    expect(tableToMarkdown(t)).toBe('| a\\\\b | c\\`d |\n| --- | --- |\n| x\\|y | \\`z\\` |');
  });
});
