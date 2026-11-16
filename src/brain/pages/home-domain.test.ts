import { core } from '../test-core';
import { listDomains, PageDomains, parseDomainInput, workspaceKey } from './home-domain';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);

const MOG = '00000000-0000-4000-8000-000000000001';
const OTHER = '00000000-0000-4000-8000-000000000002';
const HOST = 'https://acme.cloud.databricks.com';
const HOST_B = 'https://beta.cloud.databricks.com';
const D1 = '7d3c1f2a-1b2c-4d5e-8f90-0a1b2c3d4e5f';
const D2 = '0f9e8d7c-6b5a-4c3d-9e2f-1a0b9c8d7e6f';

/** The recorded `GET /api/2.0/domains` shape of SP-20 (spikes/sp-20/tests/test_sp_20_run.py). */
const listed = (ids: string[], next?: string) => ({
  status: 200,
  json: {
    domains: ids.map((id) => ({ name: `domains/${id}`, domain_id: id, display_name: `D ${id}` })),
    ...(next ? { next_page_token: next } : {}),
  },
});

describe('home domain input (PG-20)', () => {
  it('[PG-20] accepts a pasted domain ID, a domains/<id> name or a Catalog Explorer URL', () => {
    expect(parseDomainInput(` ${D1} `)).toBe(D1);
    expect(parseDomainInput(`domains/${D1}`)).toBe(D1);
    expect(parseDomainInput(`${HOST}/explore/domains/${D1}?o=1234`)).toBe(D1);
    expect(parseDomainInput(`${HOST}/explore/discover/domains/${D1}/pages#x`)).toBe(D1);
  });

  it('[PG-20] [PG-06] rejects empty text, a name with spaces and a URL without a domain', () => {
    expect(parseDomainInput('')).toBeUndefined();
    expect(parseDomainInput('   ')).toBeUndefined();
    expect(parseDomainInput('Finance team')).toBeUndefined();
    expect(parseDomainInput(`${HOST}/explore/data`)).toBeUndefined();
    expect(parseDomainInput('domains/')).toBeUndefined();
    expect(parseDomainInput('domains/%E0%A4%A')).toBeUndefined();
    expect(parseDomainInput('domains/%E0')).toBeUndefined();
    expect(parseDomainInput(`${HOST}/explore/domains/%ZZ`)).toBeUndefined();
  });

  it('[PG-18] keys a workspace by its host, in one form', () => {
    expect(workspaceKey('https://ACME.cloud.databricks.com/')).toBe('acme.cloud.databricks.com');
    expect(workspaceKey('acme.cloud.databricks.com')).toBe('acme.cloud.databricks.com');
    // The same workspace identity as the CLI profile host match of select-databricks-profile
    // (antigravity-bricks-plugin genie_bridge.py: urlsplit(base_url(host)).hostname).
    expect(workspaceKey('https://acme.cloud.databricks.com:443/')).toBe(
      'acme.cloud.databricks.com',
    );
    expect(workspaceKey('https://acme.cloud.databricks.com/?o=1234')).toBe(
      'acme.cloud.databricks.com',
    );
    expect(workspaceKey('https://Adb-1.2.azuredatabricks.net/browse#x')).toBe(
      'adb-1.2.azuredatabricks.net',
    );
  });
});

describe('PageDomains: one home domain per MOG per workspace (PG-18, PG-19)', () => {
  it('[PG-18] [AT-41] a draft goes to the MOG home domain with no domain prompt', () => {
    const d = new PageDomains();
    expect(d.setHome(MOG, HOST, D1)).toEqual({ ok: true, domainId: D1 });
    expect(d.homeDomain(MOG, HOST)).toBe(D1);
    expect(d.draftDomain('r1', MOG, HOST)).toBe(D1);
  });

  it('[PG-18] keeps one home domain for each workspace and each MOG', () => {
    const d = new PageDomains();
    d.setHome(MOG, HOST, D1);
    d.setHome(MOG, HOST_B, D2);
    expect(d.homeDomain(MOG, HOST)).toBe(D1);
    expect(d.homeDomain(MOG, HOST_B)).toBe(D2);
    expect(d.homeDomain(OTHER, HOST)).toBeUndefined();
    expect(d.config(MOG)).toEqual({
      homeDomains: { 'acme.cloud.databricks.com': D1, 'beta.cloud.databricks.com': D2 },
    });
  });

  it('[PG-18] loads the saved MOG config and falls back to the admin PageDomainId', () => {
    const d = new PageDomains(D2);
    d.load(MOG, { homeDomains: { 'acme.cloud.databricks.com': D1 } });
    expect(d.homeDomain(MOG, HOST)).toBe(D1);
    expect(d.homeDomain(MOG, HOST_B)).toBe(D2);
    expect(d.homeDomain(OTHER, HOST)).toBe(D2);
  });

  it('[PG-18] [PG-06] refuses a home domain that is not an ID, and keeps the old one', () => {
    const d = new PageDomains();
    d.setHome(MOG, HOST, D1);
    const r = d.setHome(MOG, HOST, 'Finance team');
    expect(r.ok).toBe(false);
    expect(r.ok ? '' : r.text).toMatch(/domain ID or a Catalog Explorer URL/);
    expect(d.homeDomain(MOG, HOST)).toBe(D1);
  });

  it('[PG-19] a domain changed in the review card applies to that draft only', () => {
    const d = new PageDomains();
    d.setHome(MOG, HOST, D1);
    expect(d.changeDraft('r1', `domains/${D2}`)).toEqual({ ok: true, domainId: D2 });
    expect(d.draftDomain('r1', MOG, HOST)).toBe(D2);
    expect(d.draftDomain('r2', MOG, HOST)).toBe(D1);
    expect(d.homeDomain(MOG, HOST)).toBe(D1);
    d.closeDraft('r1');
    expect(d.draftDomain('r1', MOG, HOST)).toBe(D1);
  });

  it('[PG-19] [PG-20] takes picker input from view_emit for the home domain and for one draft', () => {
    const d = new PageDomains();
    expect(d.onViewInput(MOG, HOST, { target: 'domain', scope: 'home', domainId: D1 })).toEqual({
      ok: true,
      domainId: D1,
    });
    expect(
      d.onViewInput(MOG, HOST, {
        target: 'domain_paste',
        scope: 'draft',
        reviewId: 'r1',
        text: `${HOST}/explore/domains/${D2}`,
      }),
    ).toEqual({ ok: true, domainId: D2 });
    expect(d.homeDomain(MOG, HOST)).toBe(D1);
    expect(d.draftDomain('r1', MOG, HOST)).toBe(D2);
    for (const text of ['domains/%E0%A4%A', 'domains/%E0']) {
      const bad = d.onViewInput(MOG, HOST, { target: 'domain_paste', scope: 'home', text });
      expect(bad?.ok).toBe(false);
    }
    expect(d.homeDomain(MOG, HOST)).toBe(D1);
    expect(d.onViewInput(MOG, HOST, { target: 'pin', blurbId: 'b' })).toBeUndefined();
    expect(d.onViewInput(MOG, HOST, { target: 'domain', scope: 'draft' })).toBeUndefined();
  });
});

describe('domain picker list (PG-20)', () => {
  beforeEach(() => core.reset());

  it('[PG-20] lists domains with GET /api/2.0/domains through dbx_http and follows pages', async () => {
    core.invoke.mockResolvedValueOnce(listed([D1], 'next-1')).mockResolvedValueOnce(listed([D2]));
    const r = await listDomains('p', MOG);
    expect(r).toEqual({
      kind: 'list',
      truncated: false,
      domains: [
        { id: D1, name: `D ${D1}` },
        { id: D2, name: `D ${D2}` },
      ],
    });
    const calls = core.invoke.mock.calls.map(([cmd, a]) => [cmd, a]);
    expect(calls).toEqual([
      [
        'dbx_http',
        {
          args: {
            profile: 'p',
            routeId: 'knowledge.list_domains',
            params: { page_size: 50 },
            tags: { app: 'bottle', mog: MOG, op: 'page' },
          },
        },
      ],
      [
        'dbx_http',
        {
          args: {
            profile: 'p',
            routeId: 'knowledge.list_domains',
            params: { page_size: 50, page_token: 'next-1' },
            tags: { app: 'bottle', mog: MOG, op: 'page' },
          },
        },
      ],
    ]);
  });

  it('[PG-20] without MANAGE DISCOVER (403) the picker asks for a pasted ID or URL', async () => {
    core.invoke.mockResolvedValueOnce({
      status: 403,
      json: { error_code: 'PERMISSION_DENIED', message: 'MANAGE DISCOVER' },
    });
    const r = await listDomains('p', MOG);
    expect(r.kind).toBe('paste');
    expect(r.kind === 'paste' ? r.text : '').toMatch(
      /Paste the domain ID or its Catalog Explorer URL/,
    );
  });

  it('[PG-20] a 401 on the list asks the user to sign in again, not for MANAGE DISCOVER', async () => {
    core.invoke.mockResolvedValueOnce({ status: 401, json: { error_code: 'UNAUTHENTICATED' } });
    const r = await listDomains('p', MOG);
    expect(r.kind === 'paste' ? r.reason : '').toBe('auth');
    expect(r.kind === 'paste' ? r.text : '').toMatch(/sign-in .*expired/i);
    expect(r.kind === 'paste' ? r.text : '').not.toMatch(/MANAGE DISCOVER/);
  });

  it('[PG-20] a list longer than 20 pages comes back marked truncated', async () => {
    core.invoke.mockImplementation(async () => listed([D1], 'more'));
    const r = await listDomains('p', MOG);
    expect(core.invoke).toHaveBeenCalledTimes(20);
    expect(r.kind === 'list' ? r.truncated : undefined).toBe(true);
    core.reset();
    core.invoke.mockResolvedValueOnce(listed([D1]));
    const one = await listDomains('p', MOG);
    expect(one.kind === 'list' ? one.truncated : undefined).toBe(false);
  });

  it('[PG-20] a failed call or an odd body also falls back to paste', async () => {
    core.invoke.mockRejectedValueOnce({ error: 'not_implemented', command: 'dbx_http' });
    expect((await listDomains('p', MOG)).kind).toBe('paste');
    core.invoke.mockResolvedValueOnce({ status: 200, json: { domains: 'x' } });
    expect((await listDomains('p', MOG)).kind).toBe('paste');
  });

  it('[PG-20] an empty list also offers paste, since no domain can be picked', async () => {
    core.invoke.mockResolvedValueOnce({ status: 200, json: {} });
    expect((await listDomains('p', MOG)).kind).toBe('paste');
  });
});
