/** #523: the one link trust rule, shared by the views and the brain. Fake hosts only. */
import cases from './link-cases.json';
import {
  asLinkOpenClick,
  bareHost,
  displayHost,
  LINK_OPEN,
  MAX_URL_CHARS,
  openInDatabricksUrl,
  shortUrl,
  trustedUrl,
} from './link-trust';

const HOST = 'acme-prod.cloud.databricks.test';
const trust = { profileHost: HOST, allowedAppUrls: ['https://app.example.test/base'] };

describe('trustedUrl (#523, SEC-25b)', () => {
  it('allows an https URL on the workspace host and normalises it', () => {
    expect(trustedUrl(`https://${HOST}/one/chat/threads/c1?o=1`, trust)).toBe(
      `https://${HOST}/one/chat/threads/c1?o=1`,
    );
    expect(trustedUrl(`HTTPS://${HOST.toUpperCase()}/x`, trust)).toBe(`https://${HOST}/x`);
  });

  it('takes the profile host as a bare host or as https://host', () => {
    const u = `https://${HOST}/x`;
    expect(trustedUrl(u, { profileHost: `https://${HOST}`, allowedAppUrls: [] })).toBe(u);
    expect(trustedUrl(u, { profileHost: `https://${HOST}/`, allowedAppUrls: [] })).toBe(u);
    expect(bareHost('https://A.B.test/')).toBe('a.b.test');
    expect(bareHost('not a host')).toBe('');
  });

  it('allows an AllowedAppUrls entry by host only, like Rust', () => {
    expect(trustedUrl('https://app.example.test/base/x', trust)).toBe(
      'https://app.example.test/base/x',
    );
    expect(trustedUrl('https://app.example.test/other', trust)).toBe(
      'https://app.example.test/other',
    );
    expect(trustedUrl('https://app.example.test:8443/x', trust)).toBeUndefined();
    expect(trustedUrl('https://other.example.test/x', trust)).toBeUndefined();
  });

  it('matches the shared cases that Rust checks too (link-cases.json)', () => {
    const t = { profileHost: cases.workspace, allowedAppUrls: cases.allowedAppUrls };
    for (const u of cases.ok) expect(trustedUrl(u, t), u).toBeDefined();
    for (const u of cases.refused) expect(trustedUrl(u, t), u.slice(0, 80)).toBeUndefined();
  });

  it.each([
    // Host spoofing.
    `https://${HOST}.evil.test/x`,
    `https://host.evil.test/${HOST}`,
    `https://evil.test/@${HOST}`,
    `https://evil.test/?u=https://${HOST}`,
    `https://${HOST}@evil.test/x`,
    `https://evil.test@${HOST}/x`,
    `https://user:pw@${HOST}/x`,
    `https://${HOST}:8443/x`,
    `https://${HOST}:443/x`,
    `https://${HOST}%2e/x`,
    'https://[::1]/x',
    `https://${HOST}./x`,
    `https://evil${HOST}/x`,
    `https://x.${HOST}/x`,
    `https://xn--cme-prod-9ve.cloud.databricks.test/x`,
    `https://аcme-prod.cloud.databricks.test/x`,
    `https://${HOST}\\@evil.test/x`,
    'https://acme-prod.cloud.databricks.test.evil.test/',
    // Scheme tricks.
    `http://${HOST}/x`,
    `HTTP://${HOST}/x`,
    ' https://' + HOST + '/x',
    `javascript:alert(1)//${HOST}`,
    `data:text/html,https://${HOST}`,
    `file:///${HOST}`,
    `//${HOST}/x`,
    `/${HOST}`,
    `https:/${HOST}/x`,
    `https:${HOST}/x`,
    `ftp://${HOST}/x`,
    // Control and markup characters.
    `https://${HOST}/a b`,
    `https://${HOST}/a\nb`,
    `https://${HOST}/"x`,
    `https://${HOST}/<x>`,
    `https://${HOST}/../x`,
    '',
  ])('rejects %j', (raw) => {
    expect(trustedUrl(raw, trust)).toBeUndefined();
  });

  it('trusts nothing before the profile host is known', () => {
    expect(
      trustedUrl(`https://${HOST}/x`, { profileHost: '', allowedAppUrls: [] }),
    ).toBeUndefined();
  });

  it('rejects a URL longer than the bound', () => {
    const long = `https://${HOST}/${'a'.repeat(MAX_URL_CHARS)}`;
    expect(trustedUrl(long, trust)).toBeUndefined();
  });

  it('a bare AllowedAppUrls entry with no scheme trusts nothing', () => {
    expect(
      trustedUrl('https://evil.test/x', { profileHost: 'p.test', allowedAppUrls: ['evil.test'] }),
    ).toBeUndefined();
  });
});

describe('short URL and display host (#523)', () => {
  it('shows the host plus a short path, with no scheme, query or fragment', () => {
    expect(shortUrl(`https://${HOST}/one/chat/threads/abc?o=1#x`)).toBe(
      `${HOST}/one/chat/threads/abc`,
    );
    expect(shortUrl(`https://${HOST}/`)).toBe(HOST);
    expect(shortUrl(`https://${HOST}/${'p'.repeat(60)}`).length).toBeLessThanOrEqual(
      HOST.length + 25,
    );
    expect(shortUrl(`https://${HOST}/${'p'.repeat(60)}`).endsWith('…')).toBe(true);
    expect(shortUrl('not a url')).toBe('not a url');
  });

  it('gives a host only for an http(s) URL', () => {
    expect(displayHost('https://evil.test/x')).toBe('evil.test');
    expect(displayHost('http://evil.test:81/x')).toBe('evil.test:81');
    expect(displayHost('javascript:alert(1)')).toBeUndefined();
    expect(displayHost('')).toBeUndefined();
  });
});

describe('openInDatabricksUrl (#523)', () => {
  const DEEP = `https://${HOST}/one/chat/threads/c1?o=1234&utm_source=genie_chat_mcp`;

  it('uses the deep_link that Genie One returned, when it is on the workspace host', () => {
    expect(openInDatabricksUrl(DEEP, HOST)).toBe(DEEP);
    expect(openInDatabricksUrl(DEEP, `https://${HOST}`)).toBe(DEEP);
  });

  it('gives nothing for a missing, empty, foreign or hostile deep_link', () => {
    expect(openInDatabricksUrl(undefined, HOST)).toBeUndefined();
    expect(openInDatabricksUrl('', HOST)).toBeUndefined();
    expect(openInDatabricksUrl(DEEP, undefined)).toBeUndefined();
    expect(openInDatabricksUrl(DEEP, '')).toBeUndefined();
    expect(openInDatabricksUrl('https://evil.test/one/chat/threads/c1', HOST)).toBeUndefined();
    expect(openInDatabricksUrl(`http://${HOST}/x`, HOST)).toBeUndefined();
    expect(openInDatabricksUrl('javascript:alert(1)', HOST)).toBeUndefined();
  });

  it('never builds a URL from ids (the route is not a documented contract)', () => {
    // Only a returned link opens. There is no argument for a conversation or space id.
    expect(openInDatabricksUrl.length).toBe(2);
  });
});

describe('asLinkOpenClick (#523)', () => {
  it('accepts a link click and drops unknown fields', () => {
    expect(asLinkOpenClick({ target: LINK_OPEN, url: 'https://a.test/x', extra: 1 })).toEqual({
      target: LINK_OPEN,
      url: 'https://a.test/x',
    });
    expect(
      asLinkOpenClick({ target: LINK_OPEN, url: 'https://a.test/x', requestId: 'op-1' }),
    ).toEqual({ target: LINK_OPEN, url: 'https://a.test/x', requestId: 'op-1' });
  });

  it.each([
    undefined,
    null,
    'x',
    {},
    { target: 'open_conversations', url: 'https://a.test' },
    { target: LINK_OPEN },
    { target: LINK_OPEN, url: '' },
    { target: LINK_OPEN, url: 5 },
    { target: LINK_OPEN, url: 'x'.repeat(MAX_URL_CHARS + 1) },
    { target: LINK_OPEN, url: 'https://a.test', requestId: 'has space' },
    { target: LINK_OPEN, url: 'https://a.test', requestId: 5 },
  ])('rejects %j', (p) => {
    expect(asLinkOpenClick(p)).toBeUndefined();
  });
});
