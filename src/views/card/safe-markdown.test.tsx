import { renderToStaticMarkup } from 'react-dom/server';
import { MAX_ANSWER_CHARS, SafeMarkdown } from './safe-markdown';

const trust = {
  profileHost: 'adb-1.azuredatabricks.net',
  allowedAppUrls: ['https://app.example.com/base'],
};
const html = (md: string) => renderToStaticMarkup(<SafeMarkdown text={md} trust={trust} />);

// SEC-25a/AT-17 growth check (#443). A wall-clock bound on a shared Mac is flaky, so compare the
// cost of each hostile input at three sizes instead. Each size gets a warm-up run, then the
// minimum of SAMPLES runs (load only ever adds time). The divisor has a floor, so sub-millisecond
// renders do not turn timer noise into a large ratio.
const SIZES = [2_000, 20_000, 200_000] as const;
const SAMPLES = 5;
const FLOOR_MS = 0.5;
// 20 KB -> 200 KB crosses MAX_ANSWER_CHARS: linear is about 10x (about 1x with the cap); n^2 is 100x.
const LARGE_STEP_MAX = 20;
// 2 KB -> 20 KB stays under the cap. The real renderer measures 10-19x here, because at 2 KB its
// `{0,2048}` link bound is not reached yet. An unbounded INLINE regex measures 70-170x; n^2 is 100x.
const SMALL_STEP_MAX = 40;
// Backstop only: the real renderer takes about 40 ms at 200 KB.
const BACKSTOP_MS = 1_000;

let sink: unknown;
function minMs(render: (s: string) => unknown, input: string): number {
  sink = render(input);
  let best = Infinity;
  for (let i = 0; i < SAMPLES; i++) {
    const t = performance.now();
    sink = render(input);
    best = Math.min(best, performance.now() - t);
  }
  return best;
}

function growth(render: (s: string) => unknown, unit: string) {
  const [t2, t20, t200] = SIZES.map((n) => minMs(render, unit.repeat(Math.ceil(n / unit.length))));
  return {
    small: t20 / Math.max(t2, FLOOR_MS),
    large: t200 / Math.max(t20, FLOOR_MS),
    t200,
  };
}

function expectNearLinear(render: (s: string) => unknown, unit: string) {
  const g = growth(render, unit);
  expect(g.small, `${JSON.stringify(unit)} 2 KB -> 20 KB`).toBeLessThan(SMALL_STEP_MAX);
  expect(g.large, `${JSON.stringify(unit)} 20 KB -> 200 KB`).toBeLessThan(LARGE_STEP_MAX);
  expect(g.t200, `${JSON.stringify(unit)} 200 KB backstop`).toBeLessThan(BACKSTOP_MS);
}

/** Test stand-in for a catastrophic parser: work grows as (length / scale)^2. */
function quadratic(scale: number, cap = Infinity) {
  return (s: string) => {
    const n = Math.ceil(Math.min(s.length, cap) / scale);
    let acc = 0;
    for (let i = 0; i < n; i++) for (let j = 0; j < n; j++) acc += (i ^ j) & 1;
    return acc;
  };
}

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
    const out = html(
      '![x](https://evil.example/p.png) ![y](https://adb-1.azuredatabricks.net/p.png)',
    );
    expect(out).not.toMatch(/<img|src=|<a /);
    expect(out).toContain('x');
  });

  it('[SEC-25b] links to the profile host show the full URL', () => {
    const out = html('[open](https://adb-1.azuredatabricks.net/genie/1?a=b)');
    expect(out).toContain('href="https://adb-1.azuredatabricks.net/genie/1?a=b"');
    expect(out).toContain('https://adb-1.azuredatabricks.net/genie/1?a=b</');
  });

  it('[SEC-25b] links to an AllowedAppUrls host are allowed', () => {
    expect(html('[a](https://app.example.com/base/x)')).toContain(
      'href="https://app.example.com/base/x"',
    );
  });

  it.each([
    'https://evil.example/x',
    'HTTP://adb-1.azuredatabricks.net/x',
    ' https://adb-1.azuredatabricks.net/x',
    'vbscript:msgbox(1)',
    'JaVaScRiPt:alert(1)',
    'https://adb-1.azuredatabricks.net:8443/x',
    'https://adb-1.azuredatabricks.net./x',
    'https://xn--db-1-9ve.azuredatabricks.net/x',
    'https://\u0430db-1.azuredatabricks.net/x',
    'https://user:pw@adb-1.azuredatabricks.net/x',
    'https://app.example.com/base-evil/x',
    'https://app.example.com/other',
    'https://app.example.com/base/../other',
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
    const out = html('[o](https://app.example.com/base/x)');
    expect(out).toContain('rel="noopener noreferrer"');
    expect(out).not.toMatch(/onclick|onload|autofocus|<meta|<iframe|\starget=/i);
  });

  it('[SEC-25b] a mixed-case scheme on a trusted host is normalised', () => {
    expect(html('[a](HTTPS://App.Example.com/base/x)')).toContain(
      'href="https://app.example.com/base/x"',
    );
  });

  it('[SEC-25b] a bare AllowedAppUrls entry without a scheme trusts nothing', () => {
    const bare = { profileHost: 'p.example', allowedAppUrls: ['evil.example'] };
    const out = renderToStaticMarkup(
      <SafeMarkdown text="[a](https://evil.example/x)" trust={bare} />,
    );
    expect(out).not.toContain('<a');
  });

  it('[SEC-25a] [AT-17] the growth check rejects an uncapped quadratic renderer', () => {
    expectNearLinear(quadratic(20), '[');
  }, 30_000);

  it('[SEC-25a] [AT-17] the growth check rejects a quadratic renderer below the cap', () => {
    expectNearLinear(quadratic(2, MAX_ANSWER_CHARS), '[');
  }, 30_000);
});
