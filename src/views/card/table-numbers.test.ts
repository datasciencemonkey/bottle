/** #523: number display in table cells only. */
import { formatCellNumber } from './table-numbers';

const fmt = (raw: string, locale = 'en-US') => formatCellNumber(raw, locale);

describe('formatCellNumber (#523)', () => {
  it('shows a long float with thousands separators and at most two decimals', () => {
    expect(fmt('1220042.3509332137')).toEqual({
      text: '1,220,042.35',
      changed: true,
      numeric: true,
    });
  });

  it('shows scientific notation as a plain number', () => {
    expect(fmt('3.62387909843917E8').text).toBe('362,387,909.84');
    expect(fmt('1.5e3').text).toBe('1,500');
    expect(fmt('2E2').text).toBe('200');
  });

  it('handles negatives and zero', () => {
    expect(fmt('-1234.5678').text).toBe('-1,234.57');
    expect(fmt('+12.345').text).toBe('12.35');
    expect(fmt('0.0').text).toBe('0');
    expect(fmt('-0.0').text).toBe('0');
    expect(fmt('0').changed).toBe(false);
  });

  it('keeps a short decimal readable and marks it changed only if the text differs', () => {
    expect(fmt('98.5')).toEqual({ text: '98.5', changed: false, numeric: true });
    expect(fmt('1234.5').text).toBe('1,234.5');
  });

  it('uses the locale of the user', () => {
    expect(fmt('1220042.3509', 'de-DE').text).toBe('1.220.042,35');
    expect(fmt('1220042.3509', 'fr-FR').text.replace(/\s/g, ' ')).toBe('1 220 042,35');
  });

  it.each([
    '1200',
    '-5',
    '0',
    '2026-11-18',
    '2026-11-18T10:00:00Z',
    '1.2.3',
    '00123',
    '007.5',
    '12:30',
    '12.5%',
    '$1,234.50',
    '1,234.5',
    'abc',
    'NaN',
    'Infinity',
    '-Infinity',
    '-',
    '.',
    'e5',
    '1e',
    '0x1F',
    '',
    '   ',
    'a1b2c3d4-0000-4000-8000-000000000001',
  ])('leaves %j as it is', (raw) => {
    const n = fmt(raw);
    expect(n.text).toBe(raw);
    expect(n.changed).toBe(false);
  });

  it('marks integers as numeric (they align right) and text as not numeric', () => {
    expect(fmt('1200').numeric).toBe(true);
    expect(fmt('Apple').numeric).toBe(false);
    expect(fmt('2026-11-18').numeric).toBe(false);
    expect(fmt('NaN').numeric).toBe(false);
  });

  it('never turns a tiny non-zero value into 0, and never rewrites a huge one', () => {
    expect(fmt('1.2E-7').text).toBe('1.2E-7');
    expect(fmt('0.001').text).toBe('0.001');
    expect(fmt('1E30').text).toBe('1E30');
    expect(fmt('1E400').text).toBe('1E400');
  });

  it('leaves a very long cell alone', () => {
    const long = `1.${'1'.repeat(100)}`;
    expect(fmt(long).text).toBe(long);
  });
});
