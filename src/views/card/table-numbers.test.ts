/** #523: number display in table columns. A value never changes its meaning. */
import { formatColumn, isIdHeader } from './table-numbers';

const col = (cells: string[], header = 'amount', locale = 'en-US') =>
  formatColumn(header, cells, locale);
const texts = (cells: string[], header?: string) => col(cells, header).map((c) => c.text);

describe('formatColumn (#523)', () => {
  it('rounds a value of 1 or more with more than two decimals to two', () => {
    expect(texts(['12345.6789', '1.5'])).toEqual(['12,345.68', '1.5']);
    expect(texts(['1000.999'])).toEqual(['1,001.00']);
  });

  it('shows scientific notation as a plain number only when its plain form is exact', () => {
    expect(texts(['3.62387909843917E8'])).toEqual(['362,387,909.84']);
    expect(texts(['1.5e3', '2E2'])).toEqual(['1,500', '200']);
    expect(texts(['1.2E-7'])).toEqual(['0.00000012']);
    // Too many digits, or too large to be exact: as written.
    expect(texts(['1E30'])).toEqual(['1E30']);
    expect(texts(['1.2345678901234567E5'])).toEqual(['1.2345678901234567E5']);
    expect(texts(['9E300'])).toEqual(['9E300']);
  });

  it('never rounds a value below 1: 0.0072, 0.0149 and 0.006 stay distinct and as written', () => {
    const out = texts(['0.0072', '0.0149', '0.006']);
    expect(out).toEqual(['0.0072', '0.0149', '0.006']);
    expect(new Set(out).size).toBe(3);
    expect(texts(['0.5', '0.123456', '-0.0072'])).toEqual(['0.5', '0.123456', '-0.0072']);
  });

  it('leaves a value above MAX_SAFE_INTEGER or with more than 15 significant digits as written', () => {
    expect(texts(['12345678901234567.89'])).toEqual(['12345678901234567.89']);
    expect(texts(['9007199254740993.5'])).toEqual(['9007199254740993.5']);
    expect(texts(['9007199254740993'])).toEqual(['9007199254740993']);
    expect(texts(['1234567890123456.5'])).toEqual(['1234567890123456.5']);
    expect(texts(['1.2345678901234567E5'])).toEqual(['1.2345678901234567E5']);
  });

  it('rounds away long decimals when the integer part is safe: the user example', () => {
    expect(texts(['1220042.3509332137'])).toEqual(['1,220,042.35']);
    expect(col(['1220042.3509332137'])[0]?.changed).toBe(true);
    expect(texts(['123456789012345.5678'])).toEqual(['123,456,789,012,345.57']);
    expect(texts(['12345678901234567.89'])).toEqual(['12345678901234567.89']);
  });

  it('keeps the trailing zeros the source wrote', () => {
    expect(texts(['1.10', '12.10', '2.00', '1234.50'])).toEqual([
      '1.10',
      '12.10',
      '2.00',
      '1,234.50',
    ]);
  });

  it('handles negatives and a written plus sign', () => {
    expect(texts(['-1234.5678', '-5', '-0.5'])).toEqual(['-1,234.57', '-5', '-0.5']);
    expect(texts(['+12.345', '+7'])).toEqual(['+12.35', '+7']);
  });

  it('gives integers thousands separators in a numeric column, so the column reads the same', () => {
    expect(texts(['362387909.84', '1200', '0', '-45000'])).toEqual([
      '362,387,909.84',
      '1,200',
      '0',
      '-45,000',
    ]);
    expect(texts(['1200', '34567'])).toEqual(['1,200', '34,567']);
  });

  it('does not touch ids, codes or years', () => {
    expect(texts(['1234567', '7654321'], 'account_id')).toEqual(['1234567', '7654321']);
    expect(texts(['10001', '94105'], 'zip')).toEqual(['10001', '94105']);
    expect(texts(['2024', '2025', '1999'], 'period')).toEqual(['2024', '2025', '1999']);
    expect(texts(['2024', '2025'], 'Year')).toEqual(['2024', '2025']);
    expect(texts(['00123', '00456'])).toEqual(['00123', '00456']);
    expect(texts(['007.5', '12.5'])).toEqual(['007.5', '12.5']);
    for (const h of [
      'id',
      'ID',
      'user id',
      'sfdc_id',
      'order_key',
      'customerId',
      'phone',
      'orderNo',
      'invoiceNumber',
      'order_no',
      'invoice number',
      'po_num',
      'ticketNo',
    ])
      expect(isIdHeader(h), h).toBe(true);
    for (const h of ['amount', 'total_dbu_dollars', 'count', 'valid', 'width'])
      expect(isIdHeader(h), h).toBe(false);
  });

  it('leaves a column alone when any cell is not a plain number', () => {
    // A mixed column: nothing is rewritten, not even the clean numbers.
    expect(texts(['1220042.35', 'n/a', '1200'])).toEqual(['1220042.35', 'n/a', '1200']);
    expect(texts(['12345.6789', 'Apple'])).toEqual(['12345.6789', 'Apple']);
    expect(col(['12345.6789', 'Apple']).every((c) => !c.numeric && !c.changed)).toBe(true);
  });

  it.each([
    [['12.5%', '3.25%']],
    [['$1,234.50', '$99.00']],
    [['1,234.5', '2,000']],
    [['2026-11-18', '2026-11-19']],
    [['12:30', '1.2.3']],
    [['NaN', 'Infinity']],
    [['0x1F', '1e']],
    [['a1b2c3d4-0000-4000-8000-000000000001']],
    [['', '  ']],
  ])('leaves %j exactly as written', (cells) => {
    const out = col(cells);
    expect(out.map((c) => c.text)).toEqual(cells);
    expect(out.every((c) => !c.changed)).toBe(true);
  });

  it('keeps blanks as blanks inside a numeric column', () => {
    expect(texts(['1200', '', '3400.567'])).toEqual(['1,200', '', '3,400.57']);
  });

  it('marks changed only when the text differs, and numeric only for a numeric column', () => {
    const out = col(['98.5', '1200']);
    expect(out[0]).toEqual({ text: '98.5', changed: false, numeric: true });
    expect(out[1]).toEqual({ text: '1,200', changed: true, numeric: true });
  });

  it('uses the locale of the user', () => {
    expect(formatColumn('amount', ['1220042.3509'], 'de-DE')[0]?.text).toBe('1.220.042,35');
    expect(formatColumn('amount', ['1220042.3509'], 'fr-FR')[0]?.text.replace(/\s/g, ' ')).toBe(
      '1 220 042,35',
    );
    expect(formatColumn('amount', ['1.10'], 'de-DE')[0]?.text).toBe('1,10');
  });

  it('leaves a very long cell alone', () => {
    const long = `1.${'1'.repeat(100)}`;
    expect(texts([long])).toEqual([long]);
  });
});
