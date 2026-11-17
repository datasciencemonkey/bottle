/**
 * MOG status badges (#50, ADR 0029): a pure function from the MOG's profile, backend, STT,
 * hop, lifecycle and sync state to the badges the MOG view renders.
 *
 * - `identity` (MOG-07): "<profile> · <backend label>", on hover. During a hop it names the hop
 *   target (SC-5 step 3). The host is in the accessible label.
 * - `stt` (UX-38): "STT: local" or "STT: Databricks <model>", on hover and in the card footer.
 * - `hop`, `offline`, `sync` (UX-39a): shown while their state applies.
 *
 * Badges are orthogonal to the mood (UX-39b): this function takes no mood source and returns
 * no mood, and `mapMood` takes none of these inputs. Each badge has text and an accessible
 * label that starts with the text (WCAG 2.5.3), so no state is colour only (UX-33). A badge holds names only, never a token.
 *
 * The `mog.view_state` payload has the `badges` field (contract `MogViewState`, #285).
 * TODO(#374): the brain sends this list in `mog:view_state` (with the mood) and routes the
 * `click` input `{ target: 'badge', badge }` to the badge `action`.
 */
import type { BackendBinding, ProfileName, Transcript } from '../../shared/interfaces';
import type { Lifecycle } from './mood-mapper';

export type BadgeKind = 'identity' | 'stt' | 'hop' | 'offline' | 'sync';

/** One badge. The view renders `text` with `label` as its accessible name. */
export interface MogBadge {
  kind: BadgeKind;
  text: string;
  label: string;
  /** A second visible line, for example the host of the identity badge (MOG-07). */
  detail?: string;
  /** True: shown only while the pointer is on the MOG. False: shown while the state applies. */
  hoverOnly: boolean;
  /** Appendix E: chip 21 opens "Where your data goes", chip 20 opens setup. */
  action?: BadgeAction;
}

export type BadgeAction = 'open_data_disclosures' | 'open_setup';

/** The STT provider and model of a `Transcript`. */
export type SttUse = Pick<Transcript, 'provider' | 'model'>;

/** ProfileService hop scope (W-11a). */
export type HopScope = 'question' | 'session';

/**
 * Everything the badges depend on.
 * TODO(#17): `profile` and `host` come from ProfileService (`home(mogId)` and `list()`).
 * TODO(#374): `backend` comes from `MogManager.list()` (`MogSummary.binding.backend`, #26);
 *   the brain fills these sources and sends the badges in `mog.view_state`.
 * TODO(#156, #158): `backendName` comes from the genie_agent and custom_agent backends (v1).
 * TODO(#52): `stt` is the provider of the last Transcript, or the selected provider before one.
 * TODO(#164): `hop` comes from the ProfileService hop target and scope.
 * TODO(#98): `lifecycle` comes from the MogManager lifecycle.
 * TODO(#195): `sync` comes from the SyncWorker (v1).
 */
export interface BadgeSources {
  profile: ProfileName;
  host: string;
  backend: BackendBinding;
  /** The Genie agent name or the app name (UX-15). Never the space ID or the app URL. */
  backendName?: string;
  stt?: SttUse;
  /** A hop asks Genie One in the target (SC-5) unless it names its own backend. */
  hop?: {
    profile: ProfileName;
    host: string;
    scope: HopScope;
    backend?: BackendBinding;
    backendName?: string;
  };
  lifecycle: Lifecycle;
  sync?: 'off' | 'on' | 'paused';
}

/** UX-15: "Genie One", "Genie: <agent>" or "<app name>". */
export function backendLabel(b: BackendBinding, name?: string): string {
  switch (b.kind) {
    case 'genie_one_mcp':
      return 'Genie One';
    case 'genie_agent':
      return name ? `Genie: ${name}` : 'Genie agent';
    case 'custom_agent':
      return name || 'Custom agent';
  }
}

/** CAP-57c: local STT is the default before any transcript. */
const LOCAL_DEFAULT: SttUse = { provider: 'local_parakeet', model: '' };

/** UX-38: the STT badge text for the provider that STT used. */
export function sttBadgeText(stt: SttUse): string {
  if (stt.provider === 'local_parakeet') return 'STT: local';
  return stt.model ? `STT: Databricks ${stt.model}` : 'STT: Databricks';
}

function sttLabel(stt: SttUse): string {
  // CAP-63 calls the local provider "On this device".
  return stt.provider === 'local_parakeet'
    ? 'Speech to text runs on this device'
    : stt.model
      ? `Speech to text runs on Databricks, model ${stt.model}`
      : 'Speech to text runs on Databricks';
}

/** The badges of one MOG, in a fixed order: identity, STT, hop, offline, sync. */
export function mapBadges(src: BadgeSources): MogBadge[] {
  const hop = src.hop;
  const profile = hop?.profile ?? src.profile;
  const host = hop?.host ?? src.host;
  const backend = hop
    ? hop.backend
      ? backendLabel(hop.backend, hop.backendName)
      : 'Genie One'
    : backendLabel(src.backend, src.backendName);
  const stt = src.stt ?? LOCAL_DEFAULT;
  const out: MogBadge[] = [
    {
      kind: 'identity',
      text: `${profile} · ${backend}`,
      label: `${profile} · ${backend}. Profile ${profile} on ${host}, backend ${backend}`,
      detail: host,
      hoverOnly: true,
    },
    {
      kind: 'stt',
      text: sttBadgeText(stt),
      label: `${sttBadgeText(stt)}. ${sttLabel(stt)}`,
      hoverOnly: true,
      action: 'open_data_disclosures',
    },
  ];
  if (hop) {
    const scope = `this ${hop.scope}`;
    out.push({
      kind: 'hop',
      text: `Hop: ${hop.profile} (${scope})`,
      label: `Hop: ${hop.profile} (${scope}). Asking in ${hop.profile} for ${scope}`,
      hoverOnly: false,
    });
  }
  if (src.lifecycle === 'offline') {
    out.push({
      kind: 'offline',
      text: 'Offline',
      label: 'Offline. Databricks is not reachable',
      hoverOnly: false,
    });
  }
  if (src.sync === 'paused') {
    out.push({
      kind: 'sync',
      text: 'Sync paused',
      label: 'Sync paused. All data stays on this device',
      hoverOnly: false,
      action: 'open_setup',
    });
  }
  return out;
}
