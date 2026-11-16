import { describe, expect, it } from 'vitest';
import type { BackendBinding, BackendKind } from '../../shared/interfaces';
import { NotImplementedError } from '../errors';
import { BACKEND_KINDS, BackendNotAvailableError, createGenieBridge, isBackendKind } from './genie-bridge';

describe('backend kinds (#28)', () => {
  it('[G-01] the TS backend enum is exactly genie_one_mcp, genie_agent and custom_agent', () => {
    expect([...BACKEND_KINDS]).toEqual(['genie_one_mcp', 'genie_agent', 'custom_agent']);
    // Compile-time: the list covers the BackendKind union and nothing else.
    const all: Record<BackendKind, true> = { genie_one_mcp: true, genie_agent: true, custom_agent: true };
    expect(Object.keys(all).sort()).toEqual([...BACKEND_KINDS].sort());
  });

  it('[G-01] [AT-11] other kind values are rejected', () => {
    for (const k of BACKEND_KINDS) expect(isBackendKind(k)).toBe(true);
    for (const k of ['genie_mcp', 'GENIE_ONE_MCP', '', 'custom', 'genie']) {
      expect(isBackendKind(k)).toBe(false);
    }
  });

  it('[G-02] backendFor gives the Genie One MCP backend for the default binding', () => {
    const b = createGenieBridge().backendFor({ kind: 'genie_one_mcp', profile: 'p' });
    expect(b.kind).toBe('genie_one_mcp');
    // The ask path is #33; it fails loudly until then.
    expect(() => b.ask({} as never, new AbortController().signal)).toThrow(NotImplementedError);
  });

  it('[G-01] [AT-11] v1 backends are not available in the MVP build', () => {
    const v1: BackendBinding[] = [
      { kind: 'genie_agent', profile: 'p', spaceId: 's', mode: 'chat' },
      { kind: 'custom_agent', profile: 'p', appUrl: 'https://a.aws.databricksapps.com', protocol: 'invocations' },
    ];
    for (const b of v1) {
      const err = (() => {
        try {
          createGenieBridge().backendFor(b);
        } catch (e) {
          return e;
        }
        return undefined;
      })();
      expect(err).toBeInstanceOf(BackendNotAvailableError);
      expect((err as BackendNotAvailableError).kind).toBe(b.kind);
    }
  });
});
