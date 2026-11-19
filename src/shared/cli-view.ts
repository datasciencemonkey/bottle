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
