import { core } from '../test-core';
import { call } from '../../shared/ipc';
import { classifyPagesResponse, pagesPreflight, withPagesPreflight } from './preflight';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);

const MOG = '00000000-0000-4000-8000-000000000001';
const HOST = 'https://acme.cloud.databricks.com';
const D1 = '7d3c1f2a-1b2c-4d5e-8f90-0a1b2c3d4e5f';
const D2 = '0f9e8d7c-6b5a-4c3d-9e2f-1a0b9c8d7e6f';
const base = { profile: 'p', host: HOST };

/** A stand-in for the #203 create call: the only path to `knowledge.create_page`. */
const create = (domainId: string) =>
  call('dbx_http', {
    profile: 'p',
    routeId: 'knowledge.create_page',
    params: {},
    body: { domain: `domains/${domainId}`, state: 'DRAFT' },
    tags: { app: 'bottle', mog: MOG, op: 'page' },
  });
const createCalls = () =>
  core.invoke.mock.calls.filter(
    ([, a]) => (a as { args?: { routeId?: string } }).args?.routeId === 'knowledge.create_page',
  );

describe('Pages preflight (PG-11, PG-12, ARC-F15)', () => {
  beforeEach(() => core.reset());

  it('[PG-11] [ADM-20] with no domain ID it stops and shows the admin checklist item 10', () => {
    const r = pagesPreflight({ ...base, scopes: ['all-apis'] });
    expect(r.ok).toBe(false);
    if (r.ok) return;
    expect(r.failure.code).toBe('NO_DOMAIN');
    expect(r.failure.checklist).toEqual([10]);
    expect(r.failure.text).toContain('acme.cloud.databricks.com');
    expect(r.failure.text).toMatch(/admin/);
  });

  it('[PG-12] [ADM-15] a profile without knowledge or all-apis shows checklist item 18', () => {
    const r = pagesPreflight({ ...base, domainId: D1, scopes: ['ai-gateway', 'offline_access'] });
    expect(r.ok).toBe(false);
    if (r.ok) return;
    expect(r.failure.code).toBe('NO_SCOPE');
    expect(r.failure.checklist).toEqual([18]);
    expect(r.failure.text).toContain('profile p');
    expect(r.failure.text).toContain('knowledge');
    expect(r.failure.text).toContain('all-apis');
  });

  it('[PG-12] passes with the knowledge scope or with all-apis', () => {
    for (const scopes of [['knowledge'], ['all-apis'], ['ai-gateway', 'knowledge']]) {
      expect(pagesPreflight({ ...base, domainId: D1, scopes })).toEqual({ ok: true, domainId: D1 });
    }
  });

  it('[ADM-20] a domain ID that the domain list does not have shows checklist item 10', () => {
    const domains = {
      kind: 'list',
      domains: [{ id: D2, name: 'Sales' }],
      truncated: false,
    } as const;
    const r = pagesPreflight({ ...base, domainId: D1, scopes: ['knowledge'], domains });
    expect(r.ok ? '' : r.failure.code).toBe('DOMAIN_NOT_FOUND');
    expect(r.ok ? [] : r.failure.checklist).toEqual([10]);
  });

  it('[PG-20] a pasted ID passes when the domain list cannot be read', () => {
    const domains = { kind: 'paste', reason: 'no_permission', text: 'x' } as const;
    expect(pagesPreflight({ ...base, domainId: D1, scopes: ['knowledge'], domains })).toEqual({
      ok: true,
      domainId: D1,
    });
  });

  it('[ARC-F15] [PG-12] [ADM-21] maps a Pages API status to a checklist item', () => {
    expect(classifyPagesResponse({ status: 200 })).toBe('ok');
    expect(classifyPagesResponse({ status: 403 })).toBe('NO_SCOPE');
    expect(classifyPagesResponse({ status: 401, json: { message: 'invalid scope' } })).toBe(
      'NO_SCOPE',
    );
    expect(classifyPagesResponse({ status: 404, json: { error_code: 'FEATURE_DISABLED' } })).toBe(
      'PAGES_BETA_OFF',
    );
    expect(classifyPagesResponse({ status: 404, json: { error_code: 'ENDPOINT_NOT_FOUND' } })).toBe(
      'PAGES_BETA_OFF',
    );
    expect(classifyPagesResponse({ status: 500 })).toBe('other');
  });

  it('[ARC-F15] [PG-07] a 409 counts as created only with the same request_id', () => {
    const same = { status: 409, json: { error_code: 'ALREADY_EXISTS', request_id: 'rq-1' } };
    expect(classifyPagesResponse(same, { requestId: 'rq-1' })).toBe('ok');
    expect(classifyPagesResponse(same, { requestId: 'rq-2' })).toBe('other');
    expect(classifyPagesResponse(same)).toBe('other');
    const other = { status: 409, json: { error_code: 'ALREADY_EXISTS', message: 'name taken' } };
    expect(classifyPagesResponse(other, { requestId: 'rq-1' })).toBe('other');
    expect(classifyPagesResponse({ status: 409 }, { requestId: 'rq-1' })).toBe('other');
  });

  it('[PG-11] [ADM-20] a reply that the domain does not exist maps to checklist item 10', () => {
    expect(
      classifyPagesResponse({ status: 404, json: { error_code: 'RESOURCE_DOES_NOT_EXIST' } }),
    ).toBe('DOMAIN_NOT_FOUND');
    expect(
      classifyPagesResponse({
        status: 400,
        json: { error_code: 'INVALID_PARAMETER_VALUE', message: 'Domain domains/x not found' },
      }),
    ).toBe('DOMAIN_NOT_FOUND');
    expect(
      classifyPagesResponse({
        status: 400,
        json: { error_code: 'INVALID_PARAMETER_VALUE', message: 'synonyms too long' },
      }),
    ).toBe('other');
  });

  it('[ADM-20] a cut domain list (truncated) skips the membership check', () => {
    const domains = {
      kind: 'list',
      domains: [{ id: D2, name: 'Sales' }],
      truncated: true,
    } as const;
    expect(pagesPreflight({ ...base, domainId: D1, scopes: ['knowledge'], domains })).toEqual({
      ok: true,
      domainId: D1,
    });
  });

  it('[ADM-15] item 18 names every Bottle scope of Table ADM-SCOPE', () => {
    const r = pagesPreflight({ ...base, domainId: D1, scopes: ['ai-gateway'] });
    const item = r.ok ? '' : (r.failure.items[0] ?? '');
    for (const s of ['ai-gateway', 'offline_access', 'knowledge', 'genie', 'postgres', 'all-apis'])
      expect(item).toContain(s);
  });

  it('[PG-11] [AT-41] no create call occurs without a domain ID', async () => {
    const r = await withPagesPreflight({ ...base, scopes: ['all-apis'] }, create);
    expect(r.ok ? '' : r.failure.code).toBe('NO_DOMAIN');
    expect(createCalls()).toHaveLength(0);
  });

  it('[PG-12] [AT-41] no create call occurs with a profile that lacks the scope', async () => {
    const r = await withPagesPreflight({ ...base, domainId: D1, scopes: ['ai-gateway'] }, create);
    expect(r.ok ? '' : r.failure.code).toBe('NO_SCOPE');
    expect(createCalls()).toHaveLength(0);
  });

  it('[PG-18] [PG-06] with the checks met it sends once, to the domain ID', async () => {
    core.invoke.mockResolvedValueOnce({ status: 200, json: { name: 'pages/1', state: 'DRAFT' } });
    const r = await withPagesPreflight({ ...base, domainId: D1, scopes: ['knowledge'] }, create);
    expect(r).toEqual({
      ok: true,
      created: true,
      res: { status: 200, json: { name: 'pages/1', state: 'DRAFT' } },
    });
    expect(createCalls()).toHaveLength(1);
    const args = (createCalls()[0]?.[1] as { args: { body: { domain: string } } }).args;
    expect(args.body.domain).toBe(`domains/${D1}`);
  });

  it('[ARC-F15] [ADM-21] a 403 or Beta-off reply shows the admin checklist', async () => {
    core.invoke.mockResolvedValueOnce({ status: 403, json: { error_code: 'PERMISSION_DENIED' } });
    const r403 = await withPagesPreflight({ ...base, domainId: D1 }, create);
    expect(r403.ok ? [] : r403.failure.checklist).toEqual([18]);
    core.invoke.mockResolvedValueOnce({ status: 404, json: { error_code: 'FEATURE_DISABLED' } });
    const off = await withPagesPreflight({ ...base, domainId: D1 }, create);
    expect(off.ok ? '' : off.failure.code).toBe('PAGES_BETA_OFF');
    expect(off.ok ? [] : off.failure.checklist).toEqual([10]);
    expect(off.ok ? '' : off.failure.text).toMatch(/Unity Catalog Pages \(Beta\) is off/);
  });

  it('[ARC-F15] [PG-07] a 409 with the sent request_id is created; another 409 keeps the draft', async () => {
    const conflict = { status: 409, json: { error_code: 'ALREADY_EXISTS', request_id: 'rq-1' } };
    core.invoke.mockResolvedValueOnce(conflict);
    const same = await withPagesPreflight({ ...base, domainId: D1, requestId: 'rq-1' }, create);
    expect(same).toEqual({ ok: true, created: true, res: conflict });
    core.invoke.mockResolvedValueOnce({ status: 409, json: { error_code: 'ALREADY_EXISTS' } });
    const other = await withPagesPreflight({ ...base, domainId: D1, requestId: 'rq-1' }, create);
    expect(other).toEqual({
      ok: true,
      created: false,
      res: { status: 409, json: { error_code: 'ALREADY_EXISTS' } },
    });
  });

  it('[PG-11] [ADM-20] a create reply that the domain does not exist shows checklist item 10', async () => {
    core.invoke.mockResolvedValueOnce({
      status: 404,
      json: { error_code: 'RESOURCE_DOES_NOT_EXIST' },
    });
    const r = await withPagesPreflight({ ...base, domainId: D1 }, create);
    expect(r.ok ? '' : r.failure.code).toBe('DOMAIN_NOT_FOUND');
    expect(r.ok ? [] : r.failure.checklist).toEqual([10]);
  });

  it('[ARC-F15] other replies go back to the caller, which keeps the draft for Retry', async () => {
    core.invoke.mockResolvedValueOnce({ status: 500 });
    expect(await withPagesPreflight({ ...base, domainId: D1 }, create)).toEqual({
      ok: true,
      created: false,
      res: { status: 500 },
    });
  });
});
