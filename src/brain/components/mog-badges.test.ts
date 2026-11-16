import type { CaptureState } from '../../shared/interfaces';
import {
  backendLabel,
  mapBadges,
  sttBadgeText,
  type BadgeSources,
  type MogBadge,
} from './mog-badges';
import { mapMood, type MoodSources, type RequestPhase } from './mood-mapper';

const A = '00000000-0000-4000-8000-000000000001';
const T = 1_000_000;

/** Fixtures for inputs whose owners are not built yet (see the TODO markers in mog-badges.ts). */
const home: BadgeSources = {
  profile: 'prod-finance',
  host: 'adb-111.azuredatabricks.net',
  backend: { kind: 'genie_one_mcp', profile: 'prod-finance' },
  stt: { provider: 'local_parakeet', model: 'parakeet-tdt-0.6b-v3' },
  lifecycle: 'ready',
};

const byKind = (bs: MogBadge[], kind: MogBadge['kind']) => bs.find((b) => b.kind === kind);
const kinds = (bs: MogBadge[]) => bs.map((b) => b.kind);

describe('MOG status badges (#50)', () => {
  it('[MOG-07] [AT-12] the hover badge reads "<profile> · <backend label>"', () => {
    const b = byKind(mapBadges(home), 'identity');
    expect(b).toEqual({
      kind: 'identity',
      text: 'prod-finance · Genie One',
      label: 'Profile prod-finance on adb-111.azuredatabricks.net, backend Genie One',
      hoverOnly: true,
    });
  });

  it('[MOG-07] [UX-15] the backend label is "Genie One", "Genie: <agent>" or "<app name>"', () => {
    expect(backendLabel({ kind: 'genie_one_mcp', profile: 'p' })).toBe('Genie One');
    expect(
      backendLabel(
        { kind: 'genie_agent', profile: 'p', spaceId: 's1', mode: 'chat' },
        'Sales Pipeline',
      ),
    ).toBe('Genie: Sales Pipeline');
    expect(
      backendLabel(
        { kind: 'custom_agent', profile: 'p', appUrl: 'https://x', protocol: 'responses' },
        'Forecast Bot',
      ),
    ).toBe('Forecast Bot');
    // No display name yet: a generic label, never the space ID or the app URL.
    expect(backendLabel({ kind: 'genie_agent', profile: 'p', spaceId: 's1', mode: 'chat' })).toBe(
      'Genie agent',
    );
    expect(
      backendLabel({
        kind: 'custom_agent',
        profile: 'p',
        appUrl: 'https://x',
        protocol: 'responses',
      }),
    ).toBe('Custom agent');
  });

  it('[MOG-07] [W-11a] [AT-12] during a hop the hover badge names the hop target profile', () => {
    const src: BadgeSources = {
      ...home,
      hop: { profile: 'sales-ws', host: 'adb-222.azuredatabricks.net', scope: 'session' },
    };
    const b = byKind(mapBadges(src), 'identity');
    expect(b?.text).toBe('sales-ws · Genie One');
    expect(b?.label).toContain('adb-222.azuredatabricks.net');
  });

  it('[UX-38] [AT-36] the STT badge matches the provider that STT used', () => {
    expect(sttBadgeText({ provider: 'local_parakeet', model: 'parakeet-tdt-0.6b-v3' })).toBe(
      'STT: local',
    );
    expect(sttBadgeText({ provider: 'dbx_gemini', model: 'system.ai.gemini-3-1-flash-lite' })).toBe(
      'STT: Databricks system.ai.gemini-3-1-flash-lite',
    );
    expect(sttBadgeText({ provider: 'dbx_whisper', model: 'whisper-large-v3' })).toBe(
      'STT: Databricks whisper-large-v3',
    );
    const local = byKind(mapBadges(home), 'stt');
    expect(local).toEqual({
      kind: 'stt',
      text: 'STT: local',
      label: 'Speech to text runs on this device',
      hoverOnly: true,
    });
    const cloud = byKind(
      mapBadges({ ...home, stt: { provider: 'dbx_gemini', model: 'm1' } }),
      'stt',
    );
    expect(cloud?.text).toBe('STT: Databricks m1');
    expect(cloud?.label).toBe('Speech to text runs on Databricks, model m1');
  });

  it('[UX-38] [CAP-57c] with no STT provider known yet, the badge shows the local default', () => {
    const { stt: _unused, ...noStt } = home;
    expect(byKind(mapBadges(noStt), 'stt')?.text).toBe('STT: local');
  });

  it('[UX-39a] [AT-12] a quiet MOG shows only the hover badges', () => {
    const bs = mapBadges(home);
    expect(kinds(bs)).toEqual(['identity', 'stt']);
    expect(bs.every((b) => b.hoverOnly)).toBe(true);
  });

  it('[UX-39a] [W-11a] [AT-12] the hop badge shows the target and the scope while a hop applies', () => {
    const q = mapBadges({ ...home, hop: { profile: 'sales-ws', host: 'h', scope: 'question' } });
    expect(byKind(q, 'hop')).toEqual({
      kind: 'hop',
      text: 'Hop: sales-ws (this question)',
      label: 'Asking in sales-ws for this question',
      hoverOnly: false,
    });
    const s = mapBadges({ ...home, hop: { profile: 'sales-ws', host: 'h', scope: 'session' } });
    expect(byKind(s, 'hop')?.text).toBe('Hop: sales-ws (this session)');
    expect(byKind(mapBadges(home), 'hop')).toBeUndefined();
  });

  it('[UX-39a] [UX-33] [AT-12] the offline badge shows with text while the MOG is offline', () => {
    const b = byKind(mapBadges({ ...home, lifecycle: 'offline' }), 'offline');
    expect(b).toEqual({
      kind: 'offline',
      text: 'Offline',
      label: 'Offline: Databricks is not reachable',
      hoverOnly: false,
    });
    for (const l of ['ready', 'dozing', 'paused', 'starting'] as const) {
      expect(byKind(mapBadges({ ...home, lifecycle: l }), 'offline'), l).toBeUndefined();
    }
  });

  it('[UX-39a] [MEM-24b] [AT-12] the sync badge shows "Sync paused" only while sync is paused', () => {
    expect(byKind(mapBadges({ ...home, sync: 'paused' }), 'sync')).toEqual({
      kind: 'sync',
      text: 'Sync paused',
      label: 'Sync paused: all data stays on this device',
      hoverOnly: false,
    });
    expect(byKind(mapBadges({ ...home, sync: 'on' }), 'sync')).toBeUndefined();
    expect(byKind(mapBadges({ ...home, sync: 'off' }), 'sync')).toBeUndefined();
    expect(byKind(mapBadges(home), 'sync')).toBeUndefined();
  });

  it('[UX-39a] badges come in a fixed order: identity, STT, hop, offline, sync', () => {
    const all = mapBadges({
      ...home,
      hop: { profile: 'sales-ws', host: 'h', scope: 'session' },
      lifecycle: 'offline',
      sync: 'paused',
    });
    expect(kinds(all)).toEqual(['identity', 'stt', 'hop', 'offline', 'sync']);
  });

  it('[UX-39b] mapBadges is pure and returns no mood', () => {
    const src = { ...home, sync: 'paused' as const };
    const copy = structuredClone(src);
    const out = mapBadges(src);
    expect(mapBadges(src)).toEqual(out);
    expect(src).toEqual(copy);
    expect(JSON.stringify(out)).not.toMatch(/"mood"/);
  });
});

describe('UX-39b: badges are orthogonal to the mood', () => {
  const noCapture: CaptureState = { recording: false, paused: false, othersTrack: 'off' };
  const ptt: CaptureState = { ...noCapture, owner: A, mode: 'ptt', recording: true };
  const req = (phase: RequestPhase) => ({ phase, at: T - 10 });
  /** One scenario per mood, plus the agent mood slot (UX-41). */
  const SCENARIOS: { name: string; src: MoodSources }[] = [
    {
      name: 'idle',
      src: { mog: { lifecycle: 'ready', since: 0 }, capture: noCapture, requests: [] },
    },
    {
      name: 'listening',
      src: { mog: { lifecycle: 'ready', since: 0 }, capture: ptt, requests: [] },
    },
    {
      name: 'processing',
      src: {
        mog: { lifecycle: 'ready', since: 0 },
        capture: noCapture,
        requests: [req('working')],
      },
    },
    {
      name: 'answering',
      src: {
        mog: { lifecycle: 'ready', since: 0 },
        capture: noCapture,
        requests: [req('partial')],
      },
    },
    {
      name: 'success',
      src: {
        mog: { lifecycle: 'ready', since: 0 },
        capture: noCapture,
        requests: [req('completed')],
      },
    },
    {
      name: 'error',
      src: { mog: { lifecycle: 'ready', since: 0 }, capture: noCapture, requests: [req('failed')] },
    },
    {
      name: 'sleeping',
      src: { mog: { lifecycle: 'paused', since: 0 }, capture: noCapture, requests: [] },
    },
    {
      name: 'agent mood',
      src: {
        mog: { lifecycle: 'ready', since: 0 },
        capture: noCapture,
        requests: [],
        agentMood: { mood: 'processing', at: T - 5 },
      },
    },
  ];

  it.each(SCENARIOS)(
    '[UX-39b] [AT-12] hop, offline and local-only badge inputs leave the "$name" mood unchanged',
    ({ src }) => {
      const mood = mapMood(A, src, T).mood;
      const withBadges: MoodSources = {
        ...src,
        mog: {
          ...src.mog,
          // Offline is a badge, not a mood row (4.8): it keeps the time of the state it replaces.
          lifecycle: src.mog.lifecycle === 'ready' ? 'offline' : src.mog.lifecycle,
          hop: 'sales-ws',
          localOnly: true,
        },
      };
      expect(mapMood(A, withBadges, T).mood).toBe(mood);
    },
  );

  it('[UX-39b] [AT-12] the mood sources have no field for the STT, identity or sync badge', () => {
    // A compile-time check: these badge inputs are not part of MoodSources.
    const src: MoodSources = SCENARIOS[0]!.src;
    // @ts-expect-error sync is a badge input only
    const _sync: MoodSources = { ...src, sync: 'paused' };
    // @ts-expect-error stt is a badge input only
    const _stt: MoodSources = { ...src, stt: home.stt };
    expect(mapMood(A, src, T).mood).toBe('idle');
  });
});
