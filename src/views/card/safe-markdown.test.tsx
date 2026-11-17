import { renderToStaticMarkup } from 'react-dom/server';
import { INLINE, MAX_ANSWER_CHARS, SafeMarkdown } from './safe-markdown';

const trust = {
  profileHost: 'adb-1.azuredatabricks.net',
  allowedAppUrls: ['https://app.example.com/base'],
};
const html = (md: string) => renderToStaticMarkup(<SafeMarkdown text={md} trust={trust} />);

// SEC-25a/AT-17 growth check (#443). A wall-clock bound on a shared Mac is flaky, so compare the
// cost of each hostile input at 20 KB and at 200 KB instead. Round 0 is a warm-up; then each size
// keeps the minimum of SAMPLES rounds (load only ever adds time). The two sizes are timed in turn
// in each round, so they see the same machine load. The divisor has a floor, so sub-millisecond
// renders do not turn timer noise into a large ratio.
// A 2 KB -> 20 KB step was tried too (#443): under heavy load it measured 50-66x on the real
// renderer, which overlaps the 70-170x of an unbounded INLINE regex, so it cannot be a stable test.
const SAMPLES = 5;
const FLOOR_MS = 0.5;
// The step crosses MAX_ANSWER_CHARS: linear is about 10x (about 1x with the cap); n^2 is 100x.
const STEP_MAX = 20;
// Backstop only: the real renderer takes about 40 ms at 200 KB.
const BACKSTOP_MS = 1_000;

// Keeps each render result live, so the engine cannot drop the timed work.
let sink: unknown;
function growth(render: (s: string) => unknown, unit: string) {
  const sized = (n: number) => unit.repeat(Math.ceil(n / unit.length));
  const [s20, s200] = [sized(20_000), sized(200_000)];
  const time = (input: string) => {
    const t = performance.now();
    sink = render(input);
    return performance.now() - t;
  };
  let [t20, t200] = [Infinity, Infinity];
  for (let round = 0; round <= SAMPLES; round++) {
    const [a, b] = [time(s20), time(s200)];
    if (round === 0) continue;
    [t20, t200] = [Math.min(t20, a), Math.min(t200, b)];
  }
  expect(sink).toBeDefined();
  return { ratio: t200 / Math.max(t20, FLOOR_MS), t200 };
}

function expectNearLinear(render: (s: string) => unknown, unit: string) {
  const g = growth(render, unit);
  expect(g.ratio, `${JSON.stringify(unit)} 20 KB -> 200 KB`).toBeLessThan(STEP_MAX);
  expect(g.t200, `${JSON.stringify(unit)} 200 KB backstop`).toBeLessThan(BACKSTOP_MS);
}

/** Test stand-in for a catastrophic parser: work grows as (length / scale)^2. */
function quadratic(scale: number) {
  return (s: string) => {
    const n = Math.ceil(s.length / scale);
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

  // Deterministic guards (#443 review): the timing check alone cannot see a raised cap or an
  // unbounded quantifier below the cap, so check both directly.
  it('[SEC-25a] [AT-17] a 200 KB hostile input is cut to MAX_ANSWER_CHARS of at most 20 000', () => {
    expect(MAX_ANSWER_CHARS).toBeLessThanOrEqual(20_000);
    for (const bad of ['[', '![a](', '**', '*', '`', '[a](', 'a ']) {
      const big = bad.repeat(Math.ceil(200_000 / bad.length));
      expect(html(big)).toBe(html(big.slice(0, MAX_ANSWER_CHARS)));
    }
  });

  it('[SEC-25a] [AT-17] every quantifier in the inline pattern has an upper bound', () => {
    // Drop escapes, then character classes, so only real quantifiers are left.
    const bare = INLINE.source.replace(/\\./g, '').replace(/\[[^\]]*\]/g, '');
    expect(bare).not.toMatch(/[*+]|\{\d*,\}/);
    expect(bare.match(/\{\d+,\d+\}/g)?.length).toBeGreaterThanOrEqual(5);
  });

  it('[SEC-25a] [AT-17] hostile inputs up to 200 KB render in near-linear time', () => {
    for (const bad of ['[', '![a](', '**', '*', '`', '[a](']) expectNearLinear(html, bad);
  }, 30_000);

  it('[SEC-25a] [AT-17] the growth check rejects a quadratic renderer', () => {
    expect(growth(quadratic(20), '[').ratio).toBeGreaterThanOrEqual(STEP_MAX);
  }, 30_000);
});
