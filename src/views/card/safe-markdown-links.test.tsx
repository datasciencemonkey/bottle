// @vitest-environment jsdom
/** #523: link labels, the icon and tooltip, bare URLs, blocked links, and the click path. */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { renderToStaticMarkup } from 'react-dom/server';
import { SafeMarkdown } from './safe-markdown';

const HOST = 'acme-prod.cloud.databricks.test';
const trust = { profileHost: `https://${HOST}`, allowedAppUrls: [] };
const html = (md: string) => renderToStaticMarkup(<SafeMarkdown text={md} trust={trust} />);

const textOf = (out: string) => out.replace(/<[^>]*>/g, '');

describe('links in safe markdown (#523)', () => {
  it('keeps the icon with the last characters of the label, so it never wraps alone', () => {
    const out = html(`[the quarterly dashboard](https://${HOST}/a)`);
    expect(out).toMatch(/<span class="md-link-tail">(<!-- -->)?hboard<svg[^>]*aria-hidden="true"/);
    // A short label is all tail.
    expect(html(`[ab](https://${HOST}/a)`)).toMatch(/<span class="md-link-tail">(<!-- -->)?ab<svg/);
  });

  it('[MEDIUM] a hostile run of closing brackets is trimmed in linear time (18 KB)', () => {
    const bad = `https://${HOST}/x${')'.repeat(2000)} `.repeat(9);
    expect(bad.length).toBeGreaterThan(18_000);
    const t = performance.now();
    html(bad);
    // Quadratic code took about 190 ms here; linear takes a few ms. A generous bound.
    expect(performance.now() - t).toBeLessThan(100);
  });

  it('the label is the clickable text, with an icon and the full URL as the tooltip', () => {
    const out = html(`[the Genie chat](https://${HOST}/one/chat/threads/c1?o=1)`);
    expect(textOf(out)).toBe('the Genie chat');
    expect(out).toContain(`title="https://${HOST}/one/chat/threads/c1?o=1"`);
    expect(out).toContain('lucide-external-link');
    expect(out).toContain('aria-hidden="true"');
    // The raw URL is not the link text.
    expect(out).not.toContain(`>https://${HOST}`);
  });

  it('an empty label shows the host plus a short path', () => {
    const out = html(`[](https://${HOST}/one/chat/threads/c1?o=1&utm_source=x)`);
    expect(textOf(out)).toContain(`${HOST}/one/chat/threads/c1`);
    expect(out).toContain(
      'href="https://acme-prod.cloud.databricks.test/one/chat/threads/c1?o=1&amp;utm_source=x"',
    );
  });

  it('a label that is the URL itself also shows the short form', () => {
    const out = html(`[https://${HOST}/sql/dashboards/9f2](https://${HOST}/sql/dashboards/9f2)`);
    expect(textOf(out)).toBe(`${HOST}/sql/dashboards/9f2`);
  });

  it('a bare URL in the text becomes a link under the same trust rule', () => {
    const out = html(`See https://${HOST}/sql/dashboards/9f2 now.`);
    expect(out).toContain(`href="https://${HOST}/sql/dashboards/9f2"`);
    expect(textOf(out)).toContain(`${HOST}/sql/dashboards/9f2 now.`);
    // The full stop belongs to the sentence, not to the URL.
    expect(out).toMatch(/<\/a> now\.<\/p>$/);
  });

  it('trailing punctuation and an unmatched bracket stay outside a bare URL', () => {
    const out = html(
      `(see https://${HOST}/a/b), or https://${HOST}/c; also https://${HOST}/d?x=(1)!`,
    );
    expect(out).toContain(`href="https://${HOST}/a/b"`);
    expect(out).toContain(`href="https://${HOST}/c"`);
    expect(out).toContain(`href="https://${HOST}/d?x=(1)"`);
    expect(out).toContain('), or ');
  });

  it('a bare URL to another host is plain text with its host visible, never a link', () => {
    const out = html('Read https://evil.test/phish?x=1 first.');
    expect(out).not.toContain('<a');
    expect(out).not.toContain('href');
    expect(out).toContain('md-link--blocked');
    expect(out).toContain('evil.test/phish');
    expect(out).toContain('title="Not opened: evil.test is not your workspace"');
  });

  it('a link with a label to another host shows the label and the host, not clickable', () => {
    const out = html('[our docs](https://evil.test/docs)');
    expect(out).not.toContain('<a');
    expect(out).toContain('our docs');
    expect(out).toContain('<span class="md-link-host"> (evil.test)</span>');
  });

  it.each([
    `[x](https://${HOST}.evil.test/a)`,
    `[x](https://evil.test/@${HOST})`,
    `[x](https://${HOST}@evil.test/a)`,
    `https://${HOST}.evil.test/a`,
    `https://evil.test/@${HOST}`,
    `[x](http://${HOST}/a)`,
  ])('spoofed %s is never a link', (md) => {
    const out = html(md);
    expect(out).not.toContain('<a');
    expect(out).not.toContain('href');
  });

  it('a javascript: link shows only its label', () => {
    const out = html('[click me](javascript:alert(1))');
    expect(out).not.toContain('<a');
    expect(out).not.toContain('href');
    expect(out).toContain('click me');
    expect(out).not.toContain('javascript');
  });

  it('inline code and bold never turn a URL inside them into a link', () => {
    const out = html(`\`https://${HOST}/x\` and **https://${HOST}/y**`);
    expect(out).not.toContain('<a');
  });

  it('still emits no raw HTML from a label', () => {
    const out = html(`[<img src=x onerror=alert(1)>](https://${HOST}/a)`);
    expect(out).not.toContain('<img');
    expect(out).toContain('&lt;img');
  });
});

async function mount(text: string, onOpenLink?: (u: string) => void) {
  const el = document.createElement('div');
  document.body.append(el);
  const root = createRoot(el);
  await act(async () => {
    root.render(<SafeMarkdown text={text} trust={trust} {...(onOpenLink ? { onOpenLink } : {})} />);
  });
  return { el, root };
}
const click = (el: Element, detail = 1) => {
  const e = new MouseEvent('click', { bubbles: true, cancelable: true, detail });
  act(() => void el.dispatchEvent(e));
  return e;
};

describe('a link click (#523)', () => {
  it('goes to onOpenLink with the normalised URL, and the window never navigates', async () => {
    const open = vi.fn();
    const { el } = await mount(`[chat](https://${HOST}/one/chat/threads/c1?o=1)`, open);
    const e = click(el.querySelector('a') as Element);
    expect(open).toHaveBeenCalledExactlyOnceWith(`https://${HOST}/one/chat/threads/c1?o=1`);
    expect(e.defaultPrevented).toBe(true);
  });

  it('the second click of a double-click opens nothing more', async () => {
    const open = vi.fn();
    const { el } = await mount(`[chat](https://${HOST}/a)`, open);
    click(el.querySelector('a') as Element, 2);
    expect(open).not.toHaveBeenCalled();
  });

  it('without a handler a click still never navigates', async () => {
    const { el } = await mount(`[chat](https://${HOST}/a)`);
    expect(click(el.querySelector('a') as Element).defaultPrevented).toBe(true);
  });

  it('a blocked link has no anchor to click', async () => {
    const open = vi.fn();
    const { el } = await mount('[docs](https://evil.test/a)', open);
    expect(el.querySelector('a')).toBeNull();
    click(el.querySelector('.md-link--blocked') as Element);
    expect(open).not.toHaveBeenCalled();
  });
});
