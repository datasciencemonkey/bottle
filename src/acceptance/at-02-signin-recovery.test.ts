/**
 * AT-02 "Recover expired sign-in" (#233), section 15: the brain side. Gate: live.
 *
 * The AT script: expire a U2M CLI profile, start sign-in from the chip and complete it. Repeat
 * with a PAT and a service-principal profile, an old CLI and plaintext token storage. Pass: the
 * MOG config is unchanged, paused polls resume with the same IDs, PAT and SP profiles are
 * rejected, and the old CLI and plaintext storage show blocking messages.
 *
 * The parts under test are not built: T7 ProfileService (#17), the "Connect a workspace" dialog
 * (#162), sign-out (#163) and the Table W-ERR texts with held work (#42). So every test here is
 * `it.skip` with the feature issue, written against `src/shared/interfaces.ts`. They never count
 * as a pass. The R5 / R6 side and the one part that runs today (W-18c) are in
 * `crates/mog-core/tests/at_02_signin_recovery.rs`. The live run needs a person:
 * docs/tests/AT-SIGNIN-WORKSPACE-HOP.md. No test here reads or stores a token.
 */
import type { BackendErrorCode } from '../shared/interfaces';
import { createGenieBridge } from '../brain/components/genie-bridge';
import { createProfileService } from '../brain/components/profile-service';
import { createGenieOneMcpDouble, scenarios } from '../test-support/genie-one-mcp';

vi.mock('@tauri-apps/api/core', async () => (await import('../brain/test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../brain/test-core')).eventModule);

const MOG = '00000000-0000-4000-8000-0000000000a2';
const HOST = 'https://home.example.test';

// ---------- Pending: profile rules (W-03a, W-03b, W-03c, W-10a, W-10b) ----------

describe('AT-02 profile rules (ProfileService)', () => {
  it.skip('[AT-02] [W-03a] [W-03b] PAT and service-principal profiles are not supported and never call Genie TODO(#17)', async () => {
    // TODO(#17): a `databricks auth profiles` double with one databricks-cli, one pat and one
    // oauth-m2m profile.
    const t7 = createProfileService();
    const list = await t7.list();
    expect(list.filter((p) => p.supported).map((p) => p.authType)).toEqual(['databricks-cli']);
    expect(list.filter((p) => !p.supported)).toHaveLength(2);
    const mcp = createGenieOneMcpDouble({ host: HOST, workspaceId: '1001' });
    expect(mcp.calls()).toEqual([]);
    throw new Error(
      'TODO(#17): assert each unsupported row shows "Not supported (PAT or service principal)"',
    );
  });

  it.skip('[AT-02] [W-03c] a new profile uses the databricks-cli published OAuth app TODO(#17)', async () => {
    const t7 = createProfileService();
    await t7.login(HOST, 'bottle-home');
    throw new Error('TODO(#17): assert the new profile shows the databricks-cli client ID');
  });

  it.skip('[AT-02] [W-10a] [W-10b] plaintext token storage blocks the profile; under MDM it is an error TODO(#17)', async () => {
    // TODO(#17): a `databricks auth describe` double that reports plaintext storage.
    const t7 = createProfileService();
    const cli = await t7.cliInfo();
    expect(cli.plaintextStorage).toBe(true);
    await expect(t7.status('bottle-home')).resolves.toMatchObject({ state: 'login_required' });
    throw new Error(
      'TODO(#17): assert the hub warning blocks the profile, and with RequireSecureTokenStorage=true it stays unusable until the storage changes',
    );
  });
});

// ---------- Pending: "Connect a workspace" (W-07a, W-07b, W-07c, W-08, W-15) ----------

describe('AT-02 sign-in from the chip (Connect a workspace)', () => {
  it.skip('[AT-02] [W-07a] [W-07b] [W-07c] the dialog asks for the URL, refuses other domains and signs in with a 5 min limit TODO(#162)', async () => {
    const t7 = createProfileService();
    await expect(t7.login('https://example.com', 'bottle-x')).rejects.toThrow();
    const { loginId } = await t7.login(HOST, 'bottle-home');
    expect(loginId).not.toBe('');
    throw new Error(
      'TODO(#162): assert `databricks auth login --host <url> --profile <name>` runs in the background and times out after 5 min on the sim clock',
    );
  });

  it.skip('[AT-02] [W-15] a custom public OAuth app signs in with --client-id and the two scopes TODO(#162)', async () => {
    const t7 = createProfileService();
    await t7.login(HOST, 'bottle-custom', { clientId: 'custom-app-id' });
    throw new Error(
      'TODO(#162): assert the CLI args hold `--client-id custom-app-id --scopes all-apis,offline_access`',
    );
  });

  it.skip('[AT-02] [W-08] sign-in and setup never change an existing profile or its host TODO(#17) TODO(#163)', async () => {
    const t7 = createProfileService();
    const before = await t7.list();
    await t7.login(HOST, 'bottle-home');
    expect(await t7.list()).toEqual(before);
    throw new Error(
      'TODO(#17) TODO(#163): assert a `~/.databrickscfg` double is byte-for-byte unchanged',
    );
  });
});

// ---------- Pending: errors and held work (W-14, W-16a, W-16b, W-16c) ----------

describe('AT-02 expired sign-in: errors and held work', () => {
  it.skip('[AT-02] [W-14] each Table W-ERR error shows clear text and one action, never upstream text TODO(#42)', () => {
    const bridge = createGenieBridge();
    const upstream = 'upstream-detail-that-must-not-show';
    const cases: { e: Parameters<typeof bridge.mapError>[0]; code: BackendErrorCode }[] = [
      { e: { httpStatus: 401, isErrorText: upstream }, code: 'AUTH_EXPIRED' },
      { e: { httpStatus: 403, isErrorText: upstream }, code: 'NO_GRANT' },
      { e: { httpStatus: 404, isErrorText: upstream }, code: 'GENIE_ONE_OFF' },
    ];
    for (const { e, code } of cases) {
      const m = bridge.mapError(e);
      expect(m.code).toBe(code);
      expect(m.userText).not.toContain(upstream);
    }
    throw new Error(
      'TODO(#42): add unsupported_auth, profile_exists_unreadable, IP access list, non-JSON page and timeouts',
    );
  });

  it.skip('[AT-02] [W-16a] [W-16b] [W-16c] an expired profile holds its asks, pauses its polls and resumes them with the same IDs TODO(#42)', () => {
    const mcp = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: '1001',
      responses: [scenarios.normalAnswer()],
    });
    const bridge = createGenieBridge();
    // TODO(#42): ask, the refresh fails, the poll pauses; a second ask waits in the outbox; the
    // chip offers sign-in; after sign-in the poll resumes.
    const { events } = bridge.submit({
      opId: 'op-1',
      mogId: MOG,
      binding: { kind: 'genie_one_mcp', profile: 'bottle-home' },
      question: 'What was revenue last quarter?',
      tags: { app: 'bottle', mog: MOG, op: 'genie' },
    });
    expect(events).toBeDefined();
    const polled = mcp.polls().map((p) => [p.args.conversation_id, p.args.response_id]);
    expect(new Set(polled.map((x) => x.join('/'))).size).toBe(1);
    throw new Error(
      'TODO(#42): assert other profiles keep working and the chip opens sign-in (W-16c)',
    );
  });
});

// ---------- Pending: sign-out (W-18b) ----------

describe('AT-02 sign-out', () => {
  it.skip('[AT-02] [W-18b] sign-out asks first when other tools use the profile; Cancel keeps it TODO(#163)', async () => {
    const t7 = createProfileService();
    expect((await t7.list()).map((p) => p.name)).toContain('bottle-home');
    throw new Error(
      'TODO(#163): assert the confirm shows and Cancel keeps the profile and runs no `auth logout`',
    );
  });
});
