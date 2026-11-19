/**
 * The hub Settings model of the Databricks CLI (#16; W-02a, W-02c, W-02d). The brain builds it
 * from T7 `cliInfo()` (`cli_status`, R5) and sends it to the hub only; the thin hub view
 * `CliSettings` shows it. It holds the CLI path and version only: no profile, host or token.
 * It is not a contract file. TODO(#570): `cli_status` routing in the host and the launch
 * warning codes in its reply; TODO(#525): the event names move into the contract with the other
 * hub events.
 */

/** W-02c: the recommended CLI version. R5 blocks anything below 1.17. */
export const RECOMMENDED_CLI_VERSION = '1.25.0';
export const CLI_RECOMMENDED_TEXT = 'Version 1.25 or later is recommended.';

/** The R5 launch warning codes (W-02d, `cli_runner::CliWarning::code`) and their texts. */
export const CLI_WARNING_TEXT = {
  CLI_PATH_CHANGED:
    'The Databricks CLI is at a different path since the last launch. Check that you expect this CLI.',
  CLI_VERSION_CHANGED:
    'The Databricks CLI changed since the last launch. Check that you expect this version.',
  CLI_BELOW_RECOMMENDED: CLI_RECOMMENDED_TEXT,
} as const;
export type CliWarningCode = keyof typeof CLI_WARNING_TEXT;

/** The brain-to-hub event of the model. TODO(#525). */
export const CLI_SETTINGS_EVENT = 'hub:cli_settings';
/** The `view_emit` id and click target with which the hub asks for the model. */
export const CLI_VIEW_ID = 'cli';
export const CLI_SETTINGS_TARGET = 'cli_settings';

/** The CLI that R5 verified, or `found: false`. */
export interface CliStatusModel {
  found: boolean;
  path: string;
  version: string;
  /** W-02c: the version is below {@link RECOMMENDED_CLI_VERSION}. */
  belowRecommended: boolean;
  /** W-02d: the launch warning texts. */
  warnings: string[];
}

/** The status, or `unknown` when the brain could not read `cli_status`. */
export type CliSettingsModel = CliStatusModel | { unknown: true };

function parts(v: string): number[] | undefined {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(v.trim());
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : undefined;
}

/** True if `version` is below 1.25.0, or cannot be read. */
export function isBelowRecommended(version: string): boolean {
  const v = parts(version);
  const r = parts(RECOMMENDED_CLI_VERSION) ?? [];
  if (v === undefined) return true;
  for (let i = 0; i < 3; i += 1) {
    if ((v[i] ?? 0) !== (r[i] ?? 0)) return (v[i] ?? 0) < (r[i] ?? 0);
  }
  return false;
}

/** The model for a `cliInfo()` reply and the launch warning codes (unknown codes are dropped). */
export function cliSettingsModel(
  info: { path: string; version: string },
  warningCodes: readonly string[] = [],
): CliStatusModel {
  const found = info.path !== '';
  const warnings = warningCodes
    .filter((c): c is CliWarningCode => Object.hasOwn(CLI_WARNING_TEXT, c))
    // The recommended text shows on its own below.
    .filter((c) => c !== 'CLI_BELOW_RECOMMENDED')
    .map((c) => CLI_WARNING_TEXT[c]);
  return {
    found,
    path: info.path,
    version: info.version,
    belowRecommended: found && isBelowRecommended(info.version),
    warnings,
  };
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

/** The hub's shape check of the model. */
export function asCliSettingsModel(p: unknown): CliSettingsModel | undefined {
  if (!isObj(p)) return undefined;
  if (p['unknown'] === true) return { unknown: true };
  const { found, path, version, belowRecommended, warnings } = p;
  if (
    typeof found !== 'boolean' ||
    typeof path !== 'string' ||
    typeof version !== 'string' ||
    typeof belowRecommended !== 'boolean' ||
    !Array.isArray(warnings) ||
    !warnings.every((w) => typeof w === 'string')
  ) {
    return undefined;
  }
  return { found, path, version, belowRecommended, warnings: [...warnings] };
}

// ---- #161: the first-run step "Install the Databricks CLI" (W-17b, W-17c, W-17e, W-17f, U16) ----

/** The brain-to-hub event of the install step. TODO(#525). */
export const CLI_INSTALL_EVENT = 'hub:cli_install';
/** The hub asks for the step when it opens (`view_emit`, id {@link CLI_VIEW_ID}). */
export const CLI_SETUP_TARGET = 'cli_setup';
/** The user selects "Install the Databricks CLI": this click is the consent (W-17b). */
export const CLI_INSTALL_TARGET = 'cli_install';
/** The pinned release of `cli_runner::install::PINNED_CLI_VERSION` (a test checks they match). */
export const CLI_INSTALL_VERSION = '1.26.0';
/** The source the step shows (`cli_runner::install::RELEASES_PAGE`). */
export const CLI_INSTALL_SOURCE = 'https://github.com/databricks/cli/releases';

export type CliPlatform = 'macos' | 'windows' | 'other';

/** The OS of a user agent string. */
export function cliPlatform(userAgent: string): CliPlatform {
  if (/Windows/i.test(userAgent)) return 'windows';
  if (/Mac/i.test(userAgent)) return 'macos';
  return 'other';
}

/** W-17e: the user-only install folder of `cli_runner::install::bottle_bin_dir_for`. */
export function cliInstallFolder(platform: CliPlatform): string {
  switch (platform) {
    case 'windows':
      return '%LOCALAPPDATA%\\Bottle\\bin';
    case 'macos':
      return '~/Library/Application Support/Bottle/bin';
    default:
      return '~/.local/share/Bottle/bin';
  }
}

/** What the step shows before and during the install. */
export interface CliInstallOffer {
  source: string;
  version: string;
  folder: string;
}

/**
 * `none`: R5 verified a CLI, so no step. `offer`, `installing`, `failed`: the step. `installed`:
 * the path and version of the CLI that R5 now uses (W-17f).
 */
export type CliInstallModel =
  | { step: 'none'; path: string; version: string }
  | ({ step: 'offer' | 'installing' | 'failed' } & CliInstallOffer)
  | { step: 'installed'; path: string; version: string };

/** The hub's shape check of the install model. */
export function asCliInstallModel(p: unknown): CliInstallModel | undefined {
  if (!isObj(p)) return undefined;
  const str = (k: string) => typeof p[k] === 'string';
  switch (p['step']) {
    case 'none':
    case 'installed':
      return str('path') && str('version')
        ? { step: p['step'], path: p['path'] as string, version: p['version'] as string }
        : undefined;
    case 'offer':
    case 'installing':
    case 'failed':
      return str('source') && str('version') && str('folder')
        ? {
            step: p['step'],
            source: p['source'] as string,
            version: p['version'] as string,
            folder: p['folder'] as string,
          }
        : undefined;
    default:
      return undefined;
  }
}
