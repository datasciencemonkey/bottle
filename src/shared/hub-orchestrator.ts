/**
 * The orchestrator model in Settings → Workspace and in the agent threads (ADR 0122).
 *
 * The brain (`src/brain/orchestrator.ts`) and the hub view both read this file. It holds names
 * and shape checks only, no state. The view draws what `hub:orchestrator` carries and sends one
 * `OrchestratorAction` for each user action (ARC-2, ARC-4). The `tested` flag of a model never
 * goes to the view: the owner decided that the UI shows no test label.
 */
import { HUB_SCOPE } from './hub-notice';
import type { OrchestratorState } from './ipc';

/** The brain-to-view event that carries the orchestrator view to the hub. */
export const HUB_ORCHESTRATOR_EVENT = 'hub:orchestrator';

export { HUB_SCOPE };

/** One model of the drop-down: its id and its plain label. */
export interface HubOrchestratorModel {
  id: string;
  label: string;
}

/** What the orchestrator controls draw. */
export interface HubOrchestratorView {
  models: HubOrchestratorModel[];
  /** The selected model id. It is in `models`. */
  selected: string;
  /** "Clean up dictation". */
  cleanDictation: boolean;
  /** ADR 0123: "Dictation engine": true is Databricks, false is this Mac. */
  cloudDictation: boolean;
  /** The last known status. Undefined until the brain has read it. */
  state?: OrchestratorState;
  /** The name of the home workspace, or null when there is none. */
  homeName: string | null;
}

/** The orchestrator actions, sent as `view_emit` kind `click` with mogId `HUB_SCOPE`. */
export type OrchestratorAction =
  | { action: 'orchestrator.ready' }
  | { action: 'orchestrator.model'; model: string }
  | { action: 'orchestrator.clean_dictation'; on: boolean }
  | { action: 'orchestrator.cloud_dictation'; on: boolean }
  | { action: 'orchestrator.recheck' };

const isObj = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);
const isStr = (v: unknown): v is string => typeof v === 'string';
const STATES: readonly OrchestratorState[] = [
  'ready',
  'no_home',
  'signed_out',
  'no_model',
  'unreachable',
];
/** A model id: `system.ai.` and 1 to 128 more characters. Rust checks the characters. */
const isModelId = (v: unknown): v is string =>
  isStr(v) && v.startsWith('system.ai.') && v.length > 10 && v.length <= 138;

/** The action of one hub click, or undefined. Unknown fields are dropped. */
export function asOrchestratorAction(p: unknown): OrchestratorAction | undefined {
  if (!isObj(p)) return undefined;
  switch (p['action']) {
    case 'orchestrator.ready':
      return { action: 'orchestrator.ready' };
    case 'orchestrator.recheck':
      return { action: 'orchestrator.recheck' };
    case 'orchestrator.model':
      return isModelId(p['model'])
        ? { action: 'orchestrator.model', model: p['model'] }
        : undefined;
    case 'orchestrator.clean_dictation':
      return typeof p['on'] === 'boolean'
        ? { action: 'orchestrator.clean_dictation', on: p['on'] }
        : undefined;
    case 'orchestrator.cloud_dictation':
      return typeof p['on'] === 'boolean'
        ? { action: 'orchestrator.cloud_dictation', on: p['on'] }
        : undefined;
    default:
      return undefined;
  }
}

/** The view from one `hub:orchestrator` event, or undefined. Unknown fields are dropped. */
export function asHubOrchestratorView(p: unknown): HubOrchestratorView | undefined {
  if (!isObj(p) || !Array.isArray(p['models'])) return undefined;
  const models: HubOrchestratorModel[] = [];
  for (const m of p['models']) {
    if (!isObj(m) || !isModelId(m['id']) || !isStr(m['label'])) return undefined;
    models.push({ id: m['id'], label: m['label'] });
  }
  if (!isModelId(p['selected']) || typeof p['cleanDictation'] !== 'boolean') return undefined;
  if (typeof p['cloudDictation'] !== 'boolean') return undefined;
  const home = p['homeName'];
  if (home !== null && !isStr(home)) return undefined;
  const state = p['state'];
  if (state !== undefined && !STATES.includes(state as OrchestratorState)) return undefined;
  return {
    models,
    selected: p['selected'],
    cleanDictation: p['cleanDictation'],
    cloudDictation: p['cloudDictation'],
    ...(state !== undefined ? { state: state as OrchestratorState } : {}),
    homeName: home,
  };
}
