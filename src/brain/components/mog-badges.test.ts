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
      label:
        'prod-finance · Genie One. Profile prod-finance on adb-111.azuredatabricks.net, backend Genie One',
      detail: 'adb-111.azuredatabricks.net',
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
    expect(b?.detail).toBe('adb-222.azuredatabricks.net');
  });

  it('[MOG-07] [W-11a] a hop asks Genie One in the target unless the hop names its backend', () => {
    const agentHome: BadgeSources = {
      ...home,
      backend: { kind: 'genie_agent', profile: 'prod-finance', spaceId: 's1', mode: 'chat' },
      backendName: 'Finance',
    };
    expect(byKind(mapBadges(agentHome), 'identity')?.text).toBe('prod-finance · Genie: Finance');
    const hop = { profile: 'sales-ws', host: 'h', scope: 'session' as const };
    expect(byKind(mapBadges({ ...agentHome, hop }), 'identity')?.text).toBe('sales-ws · Genie One');
    const named = {
      ...hop,
      backend: {
        kind: 'genie_agent' as const,
        profile: 'sales-ws',
        spaceId: 's2',
        mode: 'chat' as const,
      },
      backendName: 'Pipeline',
    };
    expect(byKind(mapBadges({ ...agentHome, hop: named }), 'identity')?.text).toBe(
      'sales-ws · Genie: Pipeline',
    );
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
      label: 'STT: local. Speech to text runs on this device',
      hoverOnly: true,
      action: 'open_data_disclosures',
    });
    const cloud = byKind(
      mapBadges({ ...home, stt: { provider: 'dbx_gemini', model: 'm1' } }),
      'stt',
    );
    expect(cloud?.text).toBe('STT: Databricks m1');
    expect(cloud?.label).toBe('STT: Databricks m1. Speech to text runs on Databricks, model m1');
    // No model name yet: no trailing space.
    expect(sttBadgeText({ provider: 'dbx_gemini', model: '' })).toBe('STT: Databricks');
  });

  it('[UX-38] [CAP-57c] with no STT provider known yet, the badge shows the local default', () => {
    const noStt: BadgeSources = { ...home };
    delete noStt.stt;
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
      label: 'Hop: sales-ws (this question). Asking in sales-ws for this question',
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
      label: 'Offline. Databricks is not reachable',
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
      label: 'Sync paused. All data stays on this device',
      hoverOnly: false,
      action: 'open_setup',
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

  it.each(['processing', 'answering'] as const)(
    '[UX-39b] [AT-12] going offline and back keeps the agent mood "%s" (a real transition)',
    (m) => {
      const said = { mood: m, at: T - 100 };
      const ready: MoodSources = {
        mog: { lifecycle: 'ready', since: 0 },
        capture: noCapture,
        requests: [],
        agentMood: said,
      };
      const before = mapMood(A, ready, T).mood;
      expect(before).toBe(m);
      // The MOG goes offline after the agent spoke, then comes back: `since` moves each time,
      // `moodSince` (the last change that is not ready <-> offline) does not.
      const offline: MoodSources = {
        ...ready,
        mog: { lifecycle: 'offline', since: T - 50, moodSince: 0 },
      };
      expect(mapMood(A, offline, T).mood).toBe(before);
      const back: MoodSources = {
        ...ready,
        mog: { lifecycle: 'ready', since: T - 10, moodSince: 0 },
      };
      expect(mapMood(A, back, T).mood).toBe(before);
      expect(byKind(mapBadges({ ...home, lifecycle: 'offline' }), 'offline')).toBeDefined();
    },
  );

  it.each(['paused', 'login_required', 'dozing', 'starting'] as const)(
    '[UX-41] [UX-39b] a %s -> ready change after set_mood still ends the agent mood',
    (l) => {
      const said = { mood: 'processing' as const, at: T - 5000 };
      const base = { capture: noCapture, requests: [], agentMood: said };
      // The MOG went `l` at T-4 s and back to ready at T-1 s: both are real lifecycle changes.
      const went: MoodSources = { ...base, mog: { lifecycle: l, since: T - 4000 } };
      expect(mapMood(A, went, T).mood).not.toBe('processing');
      const back: MoodSources = {
        ...base,
        mog: { lifecycle: 'ready', since: T - 1000, moodSince: T - 1000 },
      };
      expect(mapMood(A, back, T).mood).toBe('idle');
      // From `l` to offline is not a ready <-> offline move either, so it ends the agent mood.
      const off: MoodSources = {
        ...base,
        mog: { lifecycle: 'offline', since: T - 1000, moodSince: T - 1000 },
      };
      expect(mapMood(A, off, T).mood).toBe('idle');
    },
  );

  it('[UX-39b] [AT-12] the mood sources have no field for the STT, identity or sync badge', () => {
    // A compile-time check: these badge inputs are not part of MoodSources.
    const src: MoodSources = {
      mog: { lifecycle: 'ready', since: 0 },
      capture: noCapture,
      requests: [],
    };
    const typed = (s: MoodSources) => s;
    // @ts-expect-error sync is a badge input only
    typed({ ...src, sync: 'paused' });
    // @ts-expect-error stt is a badge input only
    typed({ ...src, stt: home.stt });
    expect(mapMood(A, src, T).mood).toBe('idle');
  });

  it('[ADR-0118] [AC-6.15] "Call only" shows while the talk key is held in a recording, with a label', () => {
    expect(kinds(mapBadges(home))).not.toContain('call_only');
    const b = byKind(mapBadges({ ...home, callOnly: true }), 'call_only');
    expect(b).toMatchObject({ text: 'Call only', hoverOnly: false });
    expect(b?.label.startsWith(b.text)).toBe(true);
  });
});
