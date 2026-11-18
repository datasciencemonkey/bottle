/**
 * The brain log with the SEC-16c content gate (#184, D42, ADR 0079).
 *
 * `event` writes a content-free line: a name from `SPAN_NAMES` and whole counts under keys from
 * `SPAN_ATTR_KEYS`. `content` writes text only when the admin key `ContentTelemetryAllowed`
 * and the user opt-in are both on. This is the only file in `src/` that may use `console`
 * (a test scans for it). The brain holds no token (W-01b, 4.6 rule 9); the Rust `AppLog`
 * applies the rule-7 scrub to its own content lines.
 *
 * TODO(#502): `userOptIn` reads the `app.log.contentOptIn` app setting.
 */
import { SPAN_ATTR_KEYS, SPAN_NAMES, type SpanAttrKey, type SpanName } from './telemetry';

/** Appendix C: only a boolean `true` counts as the admin opt-in. */
export function contentTelemetryAllowedFrom(mdm: unknown): boolean {
  if (typeof mdm !== 'object' || mdm === null || Array.isArray(mdm)) return false;
  return (mdm as Record<string, unknown>)['ContentTelemetryAllowed'] === true;
}

export interface BrainLogDeps {
  /** Where lines go. Default: the WebView console of the hidden brain window. */
  sink?: (line: string) => void;
  /** The admin opt-in, for example `contentTelemetryAllowedFrom(policy_get)`. Default: off. */
  adminAllowed?: () => boolean;
  /** The user opt-in. Default: off. */
  userOptIn?: () => boolean;
}

export interface BrainLog {
  /** A content-free line. Returns false, and writes nothing, for a name or field outside the sets. */
  event(name: SpanName, fields?: Partial<Record<SpanAttrKey, number>>): boolean;
  /** A content line, only with both opt-ins. Returns true if the line was written. */
  content(name: SpanName, text: string): boolean;
  contentAllowed(): boolean;
}

const has = <T extends string>(list: readonly T[], v: unknown): v is T =>
  typeof v === 'string' && (list as readonly string[]).includes(v);

const off = () => false;

export function createBrainLog(deps: BrainLogDeps = {}): BrainLog {
  const sink = deps.sink ?? ((line: string) => console.info(line));
  const adminAllowed = deps.adminAllowed ?? off;
  const userOptIn = deps.userOptIn ?? off;
  const contentAllowed = () => userOptIn() === true && adminAllowed() === true;
  return {
    event(name, fields = {}) {
      if (!has(SPAN_NAMES, name)) return false;
      let line = `event=${name}`;
      for (const [k, v] of Object.entries(fields)) {
        if (!has(SPAN_ATTR_KEYS, k)) return false;
        if (typeof v !== 'number' || !Number.isSafeInteger(v) || v < 0) return false;
        line += ` ${k}=${v}`;
      }
      sink(line);
      return true;
    },
    content(name, text) {
      if (!has(SPAN_NAMES, name) || !contentAllowed()) return false;
      // One line per call: a control character cannot forge a second line.
      // eslint-disable-next-line no-control-regex
      sink(`content=${name} text=${String(text).replace(/[\u0000-\u001f\u007f]/g, ' ')}`);
      return true;
    },
    contentAllowed,
  };
}
