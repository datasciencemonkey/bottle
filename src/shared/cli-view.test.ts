/** #16: the Settings model of the Databricks CLI (W-02a, W-02c, W-02d). Every value is fake. */
import {
  asCliSettingsModel,
  CLI_RECOMMENDED_TEXT,
  CLI_WARNING_TEXT,
  cliSettingsModel,
  isBelowRecommended,
  RECOMMENDED_CLI_VERSION,
} from './cli-view';

const PATH = '/usr/local/bin/databricks';

describe('CLI settings model (#16)', () => {
  it('[W-02c] 1.25 or later is recommended; an older or unreadable version is below', () => {
    expect(RECOMMENDED_CLI_VERSION).toBe('1.25.0');
    expect(CLI_RECOMMENDED_TEXT).toBe('Version 1.25 or later is recommended.');
    expect(isBelowRecommended('1.24.9')).toBe(true);
    expect(isBelowRecommended('1.17.0')).toBe(true);
    expect(isBelowRecommended('1.25.0')).toBe(false);
    expect(isBelowRecommended('1.100.2')).toBe(false);
    expect(isBelowRecommended('2.0.0')).toBe(false);
    expect(isBelowRecommended('not-a-version')).toBe(true);
  });

  it('[W-02a] [W-02c] a found CLI shows its path and version', () => {
    expect(cliSettingsModel({ path: PATH, version: '1.25.0' })).toEqual({
      found: true,
      path: PATH,
      version: '1.25.0',
      belowRecommended: false,
      warnings: [],
    });
    expect(cliSettingsModel({ path: '', version: '' })).toMatchObject({ found: false });
  });

  it('[W-02d] launch warning codes become texts; an unknown code is dropped', () => {
    const m = cliSettingsModel({ path: PATH, version: '1.25.0' }, [
      'CLI_PATH_CHANGED',
      'CLI_VERSION_CHANGED',
      'SOMETHING_ELSE',
    ]);
    expect(m.warnings).toEqual([
      CLI_WARNING_TEXT.CLI_PATH_CHANGED,
      CLI_WARNING_TEXT.CLI_VERSION_CHANGED,
    ]);
    expect(CLI_WARNING_TEXT.CLI_VERSION_CHANGED).toBe(
      'The Databricks CLI changed since the last launch. Check that you expect this version.',
    );
  });

  it('[W-02a] the hub accepts only a well-formed model', () => {
    const m = cliSettingsModel({ path: PATH, version: '1.25.0' });
    expect(asCliSettingsModel(m)).toEqual(m);
    expect(asCliSettingsModel({ unknown: true })).toEqual({ unknown: true });
    expect(asCliSettingsModel(null)).toBeUndefined();
    expect(asCliSettingsModel({ ...m, path: 7 })).toBeUndefined();
    expect(asCliSettingsModel({ ...m, warnings: [1] })).toBeUndefined();
  });
});
