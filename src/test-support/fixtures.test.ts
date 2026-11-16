import { FIXTURE_ENV, deviceFixture, hidRigFixture, liveFixture } from './fixtures';

describe('gate fixtures (#229)', () => {
  it('[AT-22] the live fixture is off with a clear reason when BOTTLE_TEST_PROFILE is unset', () => {
    const f = liveFixture({});
    expect(f.enabled).toBe(false);
    if (!f.enabled) {
      expect(f.reason).toContain('BOTTLE_TEST_PROFILE is not set');
      expect(f.reason).toContain('docs/tests/README.md');
    }
  });

  it('[AT-22] the live fixture is off when BOTTLE_TEST_PROFILE is blank', () => {
    expect(liveFixture({ BOTTLE_TEST_PROFILE: '  ' }).enabled).toBe(false);
  });

  it('[AT-22] the live fixture reads the profile name and the optional warehouse', () => {
    expect(liveFixture({ BOTTLE_TEST_PROFILE: 'bottle-test' })).toEqual({ enabled: true, profile: 'bottle-test' });
    expect(liveFixture({ BOTTLE_TEST_PROFILE: 'bottle-test', BOTTLE_TEST_WAREHOUSE_ID: 'abc123' })).toEqual({
      enabled: true,
      profile: 'bottle-test',
      warehouseId: 'abc123',
    });
  });

  it('[AT-22] a malformed profile name is an error, not a silent skip', () => {
    expect(() => liveFixture({ BOTTLE_TEST_PROFILE: 'a b; rm -rf' })).toThrow(/BOTTLE_TEST_PROFILE/);
  });

  it('[AT-22] the device fixture accepts only the reference devices', () => {
    expect(deviceFixture({}).enabled).toBe(false);
    expect(deviceFixture({ BOTTLE_TEST_DEVICE: 'mac-m1-8gb-14.4' })).toEqual({
      enabled: true,
      device: 'mac-m1-8gb-14.4',
    });
    expect(deviceFixture({ BOTTLE_TEST_DEVICE: 'win11-i5-16gb' }).enabled).toBe(true);
    expect(() => deviceFixture({ BOTTLE_TEST_DEVICE: 'raspberry' })).toThrow(/BOTTLE_TEST_DEVICE/);
  });

  it('[AT-22] the HID rig fixture needs a device and a port', () => {
    const off = hidRigFixture({ BOTTLE_TEST_DEVICE: 'mac-m1-current' });
    expect(off.enabled).toBe(false);
    if (!off.enabled) expect(off.reason).toContain('BOTTLE_TEST_HID_PORT');
    expect(hidRigFixture({ BOTTLE_TEST_DEVICE: 'mac-m1-current', BOTTLE_TEST_HID_PORT: '/dev/cu.usbmodem1' })).toEqual({
      enabled: true,
      device: 'mac-m1-current',
      port: '/dev/cu.usbmodem1',
    });
  });

  it('[AT-22] the env var names match the documented interface', () => {
    expect(FIXTURE_ENV).toEqual({
      profile: 'BOTTLE_TEST_PROFILE',
      warehouseId: 'BOTTLE_TEST_WAREHOUSE_ID',
      device: 'BOTTLE_TEST_DEVICE',
      hidPort: 'BOTTLE_TEST_HID_PORT',
    });
  });
});
