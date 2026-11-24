/**
 * ADR 0122: the brain side of the orchestrator model controls (Settings → Workspace, the agent
 * threads, and the ⌥G route).
 *
 * The brain keeps the last `orchestrator_status` and the model list in memory. It sends a
 * `HubOrchestratorView` to the hub window only (ARC-5) when the hub asks (`orchestrator.ready`)
 * and after each change. The hub sends one action for each click: pick a model, switch dictation
 * cleanup, check again. The selected model and the dictation switch live in the workspace
 * settings (`workspace_settings_write`); this file never stores them. The `tested` flag of a
 * model stays here: the UI shows no test label. Nothing here logs a question or a reply.
 */
import { emitTo } from '@tauri-apps/api/event';
import {
  asOrchestratorAction,
  HUB_ORCHESTRATOR_EVENT,
  HUB_SCOPE,
  type HubOrchestratorView,
} from '../shared/hub-orchestrator';
import {
  call,
  type OrchestratorModels,
  type OrchestratorStatus,
  type WorkspaceList,
} from '../shared/ipc';
import { onViewInput, type BrainBus } from './bus';

export interface OrchestratorDeps {
  /** `orchestrator_models`. */
  models(): Promise<OrchestratorModels>;
  /** `orchestrator_status`. `recheck` asks the gateway again. */
  status(recheck?: boolean): Promise<OrchestratorStatus>;
  /** The agent settings of the last workspace list, if any. */
  settings(): Pick<WorkspaceList['agent'], 'endpoint' | 'cleanDictation'> | undefined;
  /** The name of the home workspace, or null. */
  homeName(): string | null;
  /** `workspace_settings_write` for the model or the switch. It sends the workspace list. */
  write(args: { agentEndpoint?: string; cleanDictation?: boolean }): Promise<void>;
  /** Send the view to the hub window only. */
  send(view: HubOrchestratorView): Promise<unknown>;
}

export interface Orchestrator {
  /** The last known status, or undefined before the first read. */
  status(): OrchestratorStatus | undefined;
  /** Read the status again and send the view. Never rejects. `recheck` is for a user click. */
  refresh(recheck?: boolean): Promise<void>;
  /** The "Clean up dictation" setting. On until the list says otherwise. */
  cleanDictation(): boolean;
}

export function wireOrchestrator(bus: BrainBus, deps: OrchestratorDeps): Orchestrator {
  let status: OrchestratorStatus | undefined;
  let models: OrchestratorModels | undefined;

  const readStatus = async (recheck?: boolean): Promise<void> => {
    try {
      status = await deps.status(recheck);
    } catch {
      // The status stays as it was.
    }
  };
  const readModels = async (): Promise<void> => {
    try {
      models = await deps.models();
    } catch {
      // The list stays as it was.
    }
  };

  const build = (): HubOrchestratorView => {
    const s = deps.settings();
    const selected = s?.endpoint ?? models?.selected ?? models?.defaultModel ?? status?.model;
    const list = (models?.models ?? []).map((m) => ({ id: m.id, label: m.label }));
    // The selected model shows even when the gateway does not list it (an old or removed id).
    if (selected !== undefined && !list.some((m) => m.id === selected)) {
      list.unshift({ id: selected, label: selected.replace(/^system\.ai\./, '') });
    }
    return {
      models: list,
      selected: selected ?? '',
      cleanDictation: s?.cleanDictation ?? true,
      ...(status ? { state: status.state } : {}),
      homeName: deps.homeName(),
    };
  };
  const push = async (): Promise<void> => {
    const v = build();
    // The view needs a model id; before the first read there is none to draw.
    if (v.selected === '') return;
    try {
      await deps.send(v);
    } catch {
      // No hub is open. The hub asks again with `orchestrator.ready` when it opens.
    }
  };

  onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.mogId !== HUB_SCOPE || input.kind !== 'click') return;
    const a = asOrchestratorAction(input.payload);
    if (!a) return;
    switch (a.action) {
      case 'orchestrator.ready':
        return void Promise.all([readModels(), readStatus()]).then(push);
      case 'orchestrator.recheck':
        return void readStatus(true).then(push);
      case 'orchestrator.model':
        return void deps
          .write({ agentEndpoint: a.model })
          .then(() => readStatus())
          .then(push);
      case 'orchestrator.clean_dictation':
        return void deps.write({ cleanDictation: a.on }).then(push);
    }
  });

  return {
    status: () => status,
    refresh: async (recheck) => {
      await readStatus(recheck);
      await push();
    },
    cleanDictation: () => deps.settings()?.cleanDictation ?? true,
  };
}

/** The production deps: `call` and `emitTo('hub', …)`. */
export function ipcOrchestratorDeps(
  parts: Pick<OrchestratorDeps, 'settings' | 'homeName' | 'write'>,
): OrchestratorDeps {
  return {
    ...parts,
    models: () => call('orchestrator_models', {}),
    status: (recheck) => call('orchestrator_status', recheck ? { recheck } : {}),
    send: (view) => emitTo('hub', HUB_ORCHESTRATOR_EVENT, view),
  };
}
