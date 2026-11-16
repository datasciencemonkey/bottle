// Gate fixtures for live-workspace and device tests (#229, AT-22, NFR-23b). The TS twin of
// mog_core::testkit. A test reads its fixture and skips with the reason when it is not set:
//
//   const live = liveFixture();
//   it('[AT-07] ...', (ctx) => { if (!live.enabled) return ctx.skip(live.reason); ... });
//
// A malformed value throws, so a wrong setup cannot look like a skip. The fixtures themselves
// (devices, the bottle-test U2M profile, the HID rig) need a person: TODO(#270).
// This module reads a profile name only, never a token.

export const FIXTURE_ENV = {
  profile: 'BOTTLE_TEST_PROFILE',
  warehouseId: 'BOTTLE_TEST_WAREHOUSE_ID',
  device: 'BOTTLE_TEST_DEVICE',
  hidPort: 'BOTTLE_TEST_HID_PORT',
} as const;

export const REFERENCE_DEVICES = ['mac-m1-8gb-14.4', 'mac-m1-current', 'win11-i5-16gb'] as const;
export type ReferenceDevice = (typeof REFERENCE_DEVICES)[number];

export type Env = Readonly<Record<string, string | undefined>>;
export type Skip = { enabled: false; reason: string };
export type LiveFixture = { enabled: true; profile: string; warehouseId?: string } | Skip;
export type DeviceFixture = { enabled: true; device: ReferenceDevice } | Skip;
export type HidRigFixture = { enabled: true; device: ReferenceDevice; port: string } | Skip;

const HELP = 'see docs/tests/README.md';
const PROFILE_RE = /^[A-Za-z0-9._-]{1,128}$/;

function read(env: Env, key: string): string | undefined {
  const v = env[key]?.trim();
  return v ? v : undefined;
}

function notSet(key: string, what: string): Skip {
  return {
    enabled: false,
    reason: `skipped: ${key} is not set, so ${what} tests do not run (${HELP})`,
  };
}

export function liveFixture(env: Env = process.env): LiveFixture {
  const profile = read(env, FIXTURE_ENV.profile);
  if (!profile) return notSet(FIXTURE_ENV.profile, 'live-workspace');
  if (!PROFILE_RE.test(profile)) {
    throw new Error(
      `${FIXTURE_ENV.profile} must be a CLI profile name (letters, digits, '-', '_', '.'); ${HELP}`,
    );
  }
  const warehouseId = read(env, FIXTURE_ENV.warehouseId);
  return warehouseId ? { enabled: true, profile, warehouseId } : { enabled: true, profile };
}

export function deviceFixture(env: Env = process.env): DeviceFixture {
  const raw = read(env, FIXTURE_ENV.device);
  if (!raw) return notSet(FIXTURE_ENV.device, 'device');
  const device = REFERENCE_DEVICES.find((d) => d === raw);
  if (!device)
    throw new Error(
      `${FIXTURE_ENV.device} must be one of ${REFERENCE_DEVICES.join(', ')}; ${HELP}`,
    );
  return { enabled: true, device };
}

export function hidRigFixture(env: Env = process.env): HidRigFixture {
  const d = deviceFixture(env);
  if (!d.enabled) return d;
  const port = read(env, FIXTURE_ENV.hidPort);
  if (!port) return notSet(FIXTURE_ENV.hidPort, 'HID rig');
  return { enabled: true, device: d.device, port };
}
