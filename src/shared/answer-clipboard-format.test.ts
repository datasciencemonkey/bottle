/** ADR 0124 amendment: Markdown -> HTML/plain text for `clipboard_write_text`. Fake values. */
import {
  formatAnswerForClipboard,
  markdownToHtml,
  markdownToPlainText,
} from './answer-clipboard-format';
import { NO_LINKS, type LinkTrust } from './link-trust';

const TRUST: LinkTrust = { profileHost: 'sales.cloud.databricks.com', allowedAppUrls: [] };

describe('markdownToHtml', () => {
  it('[owner request] a table becomes a real <table> with <th>/<td>', () => {
    const md = '| A | B |\n| --- | --- |\n| 1 | 2 |';
    const html = markdownToHtml(md, NO_LINKS);
    expect(html).toContain('<table');
    expect(html).toContain('<th scope="col"');
    expect(html).toMatch(/<th[^>]*>A<\/th>/);
    expect(html).toMatch(/<th[^>]*>B<\/th>/);
    expect(html).toMatch(/<td[^>]*>1<\/td>/);
    expect(html).toMatch(/<td[^>]*>2<\/td>/);
    // Minimal inline styles so Gmail, Outlook, Docs and Word draw borders and padding.
    expect(html).toContain('border:1px solid');
  });

  it('bold and a trusted link render as <strong> and <a href>', () => {
    const md =
      'Revenue grew **a lot** this quarter: [the report](https://sales.cloud.databricks.com/x).';
    const html = markdownToHtml(md, TRUST);
    expect(html).toContain('<strong>a lot</strong>');
    expect(html).toContain('<a href="https://sales.cloud.databricks.com/x">the report</a>');
  });

  it('italic and inline code render as <em> and <code>', () => {
    const html = markdownToHtml('It is *likely* that `SELECT 1` ran.', NO_LINKS);
    expect(html).toContain('<em>likely</em>');
    expect(html).toContain('<code>SELECT 1</code>');
  });

  it('headings become real <h1>-<h6>, not a bold paragraph', () => {
    const html = markdownToHtml('# Title\n\n## Subtitle', NO_LINKS);
    expect(html).toContain('<h1');
    expect(html).toContain('>Title</h1>');
    expect(html).toContain('<h2');
    expect(html).toContain('>Subtitle</h2>');
  });

  it('lists become <ul>/<ol> with <li>', () => {
    const ul = markdownToHtml('- one\n- two', NO_LINKS);
    expect(ul).toContain('<ul');
    expect(ul.match(/<li>/g)).toHaveLength(2);
    const ol = markdownToHtml('1. one\n2. two', NO_LINKS);
    expect(ol).toContain('<ol');
  });

  it('a fenced code block becomes <pre><code>', () => {
    const html = markdownToHtml('```\nSELECT 1;\n```', NO_LINKS);
    expect(html).toContain('<pre');
    expect(html).toContain('<code>SELECT 1;</code>');
  });

  it('[owner request] an unsafe link (javascript:) becomes plain text, never an <a>', () => {
    const html = markdownToHtml('[click me](javascript:alert(1))', TRUST);
    expect(html).not.toContain('<a ');
    expect(html).toContain('click me');
  });

  it('[owner request] an untrusted-host link becomes plain text, never an <a>', () => {
    const html = markdownToHtml('[click me](https://evil.example.com/x)', TRUST);
    expect(html).not.toContain('<a ');
    expect(html).toContain('click me');
  });

  it('[owner request] a script or HTML injection in the answer text is escaped', () => {
    const html = markdownToHtml(
      '<script>alert(1)</script> and <img src=x onerror=alert(1)>',
      NO_LINKS,
    );
    expect(html).not.toContain('<script>');
    expect(html).not.toContain('<img ');
    expect(html).not.toContain('<img>');
    expect(html).toContain('&lt;script&gt;');
    // The attacker's text survives only as harmless escaped text, never as a live attribute.
    expect(html).toContain('&lt;img src=x onerror=alert(1)&gt;');
  });

  it('never emits an <img>: an image markdown token keeps only its alt text', () => {
    const html = markdownToHtml('![a chart](https://sales.cloud.databricks.com/chart.png)', TRUST);
    expect(html).not.toContain('<img');
    expect(html).toContain('a chart');
  });

  it('escapes every literal character, including quotes and ampersands', () => {
    const html = markdownToHtml('A "quoted" value & more', NO_LINKS);
    expect(html).toContain('&quot;quoted&quot;');
    expect(html).toContain('&amp;');
  });
});

describe('markdownToPlainText', () => {
  it('[owner request] a table becomes TAB-separated rows, header row included', () => {
    const md = '| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |';
    expect(markdownToPlainText(md)).toBe('A\tB\n1\t2\n3\t4');
  });

  it('[owner request] removes **, # and table pipes', () => {
    expect(markdownToPlainText('**bold** and *italic*')).toBe('bold and italic');
    expect(markdownToPlainText('# Heading')).toBe('Heading');
  });

  it('[owner request] a link becomes "text (url)"', () => {
    expect(markdownToPlainText('[the report](https://example.com/x)')).toBe(
      'the report (https://example.com/x)',
    );
  });

  it('a bare URL stays a bare URL, with no duplicated parens', () => {
    expect(markdownToPlainText('See https://example.com/x for more.')).toBe(
      'See https://example.com/x for more.',
    );
  });

  it('an ordinary sentence with no markdown passes through unchanged', () => {
    expect(markdownToPlainText('The answer.')).toBe('The answer.');
    expect(markdownToPlainText('Revenue grew.')).toBe('Revenue grew.');
  });

  it('a code block keeps its raw text, with no backticks', () => {
    expect(markdownToPlainText('```\nSELECT 1;\n```')).toBe('SELECT 1;');
  });

  it('plain text keeps every link, trusted host or not (no clickability risk)', () => {
    expect(markdownToPlainText('[x](javascript:alert(1))')).toBe('x (javascript:alert(1))');
  });
});

describe('formatAnswerForClipboard', () => {
  it('returns both forms of the same answer', () => {
    const out = formatAnswerForClipboard('**Hi**', NO_LINKS);
    expect(out.text).toBe('Hi');
    expect(out.html).toContain('<strong>Hi</strong>');
  });
});
