import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { MOG_CONFIG_DEFAULTS, MOG_CONFIG_SCHEMA_VERSION, type MogConfig } from './mog-config';

/** The default config that `mog_core::mog_config` writes (its test checks the same file). */
function fixture(): unknown {
  return JSON.parse(
    readFileSync(
      join(import.meta.dirname, '../../../crates/mog-core/tests/fixtures/mog-config-v1.json'),
      'utf8',
    ),
  );
}

describe('MogConfig (#25)', () => {
  it('[MB-02] the TS defaults equal the Rust default config', () => {
    const cfg: MogConfig = {
      ...MOG_CONFIG_DEFAULTS,
      name: 'MOG 1',
      binding: {
        home: 'dev',
        backend: { kind: 'genie_one_mcp', profile: 'dev' },
        genieCompanion: true,
        profileAliases: {},
      },
    };
    expect(cfg).toEqual(fixture());
    expect(cfg.schemaVersion).toBe(MOG_CONFIG_SCHEMA_VERSION);
  });
});
