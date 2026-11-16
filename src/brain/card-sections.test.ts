import {
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
    const ids = buildSections({ ...full, sentToGenie: undefined, canResend: false }).map(
      (s) => s.id,
    );
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
    const rows = Array.from({ length: 30 }, (_, i) => [`r${i}`, `${i}`]);
    const s = buildSections({ table: { columns: ['a', 'b'], rows }, dataFlowText: 'd' }).find(
      (x) => x.id === 'tablePreview',
    );
    expect(s).toMatchObject({ rows: rows.slice(0, 10), moreRows: 20 });
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
});
