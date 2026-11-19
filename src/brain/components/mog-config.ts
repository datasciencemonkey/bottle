/**
 * The MOG config JSON that `config_read` returns and `config_write` takes (#25, MB-02, ADR 0035).
 *
 * The fields are the `mog.*` keys of Appendix F without the `mog.` prefix. The Rust core
 * (`mog_core::mog_config`) owns the schema: it rejects unknown fields, values out of bounds,
 * token-like values and keys that name a secret, and it fills in the defaults below.
 * The config holds no token, password, key or memory content.
 */
import type {
  BackendBinding,
  CaptureMode,
  HarnessKind,
  ProfileName,
  Transcript,
} from '../../shared/interfaces';

export const MOG_CONFIG_SCHEMA_VERSION = 1;

export interface MogConfig {
  schemaVersion: typeof MOG_CONFIG_SCHEMA_VERSION;
  name: string; // 1-64 chars
  ui: { variant: string; size: 'S' | 'M' | 'L'; dozeAfterMin: number };
  binding: {
    home: ProfileName;
    backend: BackendBinding; // core fills `genie_one_mcp` on the home profile if missing
    genieCompanion: boolean;
    profileAliases: Record<string, ProfileName>;
  };
  capture: { modes: CaptureMode[]; clickTogglesListening: boolean; vadEndMs: number };
  hotkeys: { chord?: number }; // v1, digit 0-9
  genie: {
    /**
     * `autoSendPtt` and `previewMs` have no reader in the MVP: ⌥G sends at once (ADR 0098). They
     * stay in the stored schema for the v1 preview (RTR-04a), because the core rejects unknown
     * fields, so removing them would break MOG configs that are already saved.
     */
    autoSendPtt: boolean;
    previewMs: number;
    shareContext: boolean;
    shareContextCrossWorkspace: boolean;
  };
  agent: { harness: HarnessKind; model?: string };
  pages: { domainId?: string };
  memory: { enabled: boolean; lakebaseSync: boolean; transcriptSync: boolean };
  persona: string; // <= 2,000 chars
  stt: { provider?: Transcript['provider'] };
  autoStart: boolean;
  card: { autoHideSec: number; maxBlurbs: number };
}

/** The Appendix F defaults. `name` and `binding` come from the wizard. */
export const MOG_CONFIG_DEFAULTS: Omit<MogConfig, 'name' | 'binding'> = {
  schemaVersion: MOG_CONFIG_SCHEMA_VERSION,
  ui: { variant: 'auto', size: 'M', dozeAfterMin: 20 },
  capture: { modes: ['ptt', 'hands_free'], clickTogglesListening: true, vadEndMs: 1500 },
  hotkeys: {},
  genie: {
    autoSendPtt: true,
    previewMs: 1500,
    shareContext: false,
    shareContextCrossWorkspace: false,
  },
  agent: { harness: 'claude' },
  pages: {},
  memory: { enabled: true, lakebaseSync: false, transcriptSync: false },
  persona: '',
  stt: {},
  autoStart: false,
  card: { autoHideSec: 60, maxBlurbs: 3 },
};
