import { renderToStaticMarkup } from 'react-dom/server';
import { SafeMarkdown } from './safe-markdown';

const trust = {
  profileHost: 'adb-1.azuredatabricks.net',
  allowedAppUrls: ['https://app.example.com/base'],
};
const html = (md: string) => renderToStaticMarkup(<SafeMarkdown text={md} trust={trust} />);

describe('safe markdown', () => {
  it('[SEC-25a] [AT-17] shows raw HTML in an answer as text', () => {
    const out = html('<script>alert(1)</script> <b onclick="x()">hi</b>');
    expect(out).not.toContain('<script');
    expect(out).not.toContain('<b ');
    expect(out).toContain('&lt;script&gt;alert(1)&lt;/script&gt;');
  });

  it('[SEC-25a] renders the markdown subset', () => {
    const out = html('# T\n\n**bold** *it* `code`\n\n- a\n- b\n\n1. x\n\n```\nraw <i>\n```');
    expect(out).toContain('<strong>bold</strong>');
    expect(out).toContain('<em>it</em>');
    expect(out).toContain('<code>code</code>');
    expect(out).toContain('<ul><li>a</li><li>b</li></ul>');
    expect(out).toContain('<ol><li>x</li></ol>');
    expect(out).toContain('<pre><code>raw &lt;i&gt;</code></pre>');
  });

  it('[SEC-25a] [AT-17] never emits an image, even with a remote src', () => {
    const out = html('![x](https://evil.example/p.png) ![y](adb-1.azuredatabricks.net)');
    expect(out).not.toContain('<img');
    expect(out).not.toContain('evil.example/p.png"');
  });

  it('[SEC-25b] links to the profile host show the full URL', () => {
    const out = html('[open](https://adb-1.azuredatabricks.net/genie/1?a=b)');
    expect(out).toContain('href="https://adb-1.azuredatabricks.net/genie/1?a=b"');
    expect(out).toContain('https://adb-1.azuredatabricks.net/genie/1?a=b</');
  });

  it('[SEC-25b] links to an AllowedAppUrls host are allowed', () => {
    expect(html('[a](https://app.example.com/x)')).toContain('href="https://app.example.com/x"');
  });

  it.each([
    'https://evil.example/x',
    'http://adb-1.azuredatabricks.net/x',
    'javascript:alert(1)',
    'data:text/html,hi',
    'https://adb-1.azuredatabricks.net.evil.example/x',
    'https://adb-1.azuredatabricks.net@evil.example/x',
    'https://evil.example@adb-1.azuredatabricks.net/x',
    '//evil.example/x',
    '/relative',
  ])('[SEC-25b] [AT-17] drops the hostile link %s', (url) => {
    const out = html(`[click](${url})`);
    expect(out).not.toContain('<a');
    expect(out).not.toContain('href');
    expect(out).toContain('click');
  });

  it('[SEC-25b] a link never opens by itself and has no script hooks', () => {
    const out = html('[o](https://app.example.com/x)');
    expect(out).toContain('rel="noopener noreferrer"');
    expect(out).not.toMatch(/onclick|onload|autofocus|<meta|<iframe|target="_self"/i);
  });
});
