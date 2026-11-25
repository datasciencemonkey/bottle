/**
 * Settings → Workspace → "MOG agent" (ADR 0122). Thin view (ARC-2, ARC-4): it draws what the
 * brain sends in `HubWorkspaceList.agent` and in `HubOrchestratorView`, and it sends one action
 * for each user action. The brain checks every value.
 *
 * - "Orchestrator model [▾]": the chat model that cleans dictation and answers ⌥G. It lists the
 *   models of the home AI Gateway by their plain label. It shows no test label.
 * - "Clean up dictation": a switch, on by default. It sends `orchestrator.clean_dictation`.
 * - "Dictation engine [On this Mac | Databricks]" (ADR 0123): On this Mac by default. It sends
 *   `orchestrator.cloud_dictation`. With Databricks it shows the disclosure line.
 * - A quiet line with the reason when the orchestrator model is not ready.
 * - "Genie calls per question [5 | 10 | 20]": D2, the most Genie calls for one question.
 * - D1 "Combine across workspaces" is removed (ADR 0122 amendment, owner decision 2026-11-24):
 *   cross-workspace data is always on. There is no switch for it.
 * - The disclosure line: where a cross-workspace question goes.
 */
import { useId } from 'react';
import type { HubOrchestratorView, OrchestratorAction } from '../../shared/hub-orchestrator';
import {
  HUB_SCOPE,
  type HubGenieCallBudget,
  type HubWorkspaceList,
  type WorkspaceAction,
} from '../../shared/hub-workspaces';
import { Button } from '../ui/button';
import { RadioGroup, RadioGroupItem } from '../ui/radio-group';
import { Switch } from '../ui/switch';
import { emitView } from '../view-ipc';
import { notReadyReason } from './orchestrator-text';
import { SettingsGroup, SettingsRow } from './SettingsGroup';

function send(action: WorkspaceAction | OrchestratorAction): void {
  void emitView(HUB_SCOPE, 'click', action).catch(() => undefined);
}

export const ORCHESTRATOR_LABEL = 'Orchestrator model';
export const CLEAN_DICTATION_LABEL = 'Clean up dictation';
export const DICTATION_ENGINE_LABEL = 'Dictation engine';
/** ADR 0123: shown when "Databricks" is chosen. */
export const DICTATION_ENGINE_DISCLOSURE =
  "Databricks: your voice is sent to your home workspace's AI Gateway to transcribe it.";
export const DICTATION_ENGINE_LOCAL = 'On this Mac: your voice stays on this Mac.';
/** The help of the model row: where it runs and what it does. */
export const orchestratorHelp = (homeName: string): string =>
  `Runs on ${homeName}'s AI Gateway. Cleans up dictation and answers ⌥G by asking your workspaces' Genies.`;
export const BUDGET_HELP = 'The most Genie calls your MOG makes for one question.';
export const BUDGETS: readonly HubGenieCallBudget[] = [5, 10, 20];

/** The disclosure line (section 5 of the plan): where the question goes. */
export const disclosureLines = (homeName: string): string[] => [
  `When you press ⌥G, Bottle sends your question to the orchestrator model in ${homeName}.`,
  'The model decides which of your workspaces to ask. It asks only workspaces you turned on.',
  'It may combine answers from more than one of your workspaces into one answer.',
];

/** A 24px hit area for a control that is smaller than that. */
const HIT = "relative before:absolute before:inset-x-0 before:-inset-y-1 before:content-['']";

export function AgentSettings({
  list,
  orchestrator,
}: {
  list: HubWorkspaceList | undefined;
  /** The orchestrator view. Undefined until the brain sends it. */
  orchestrator?: HubOrchestratorView | undefined;
}) {
  const budgetId = useId();
  const modelId = useId();
  const cleanId = useId();
  const engineId = useId();
  if (!list) return null;
  const home = list.rows.find((r) => r.isHome);
  const homeName = home?.name ?? 'your home workspace';
  const state = orchestrator?.state;
  const reason =
    state !== undefined && state !== 'ready'
      ? notReadyReason(state, orchestrator?.homeName ?? home?.name ?? null)
      : undefined;
  return (
    <section
      data-view="agent-settings"
      aria-labelledby="agent-settings-title"
      className="mt-[var(--mog-space-4)]"
    >
      <h3
        id="agent-settings-title"
        className="m-0 mb-[var(--mog-space-2)] text-[var(--mog-text-md)]"
      >
        MOG agent
      </h3>
      <SettingsGroup>
        {orchestrator && (
          <>
            <SettingsRow
              className="items-start"
              descriptionOutsideLabel
              label={<span id={`${modelId}-label`}>{ORCHESTRATOR_LABEL}</span>}
              htmlFor={modelId}
              description={
                <span id={`${modelId}-help`} data-orchestrator-help>
                  {orchestratorHelp(orchestrator.homeName ?? homeName)}
                </span>
              }
              control={
                <select
                  id={modelId}
                  data-orchestrator-model
                  aria-labelledby={`${modelId}-label`}
                  aria-describedby={`${modelId}-help${reason ? ` ${modelId}-status` : ''}`}
                  value={orchestrator.selected}
                  className="min-h-6 max-w-64 rounded-[var(--radius-md)] border border-border bg-card px-[var(--mog-space-2)] text-[var(--mog-text-md)] text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 outline-none"
                  onChange={(e) => send({ action: 'orchestrator.model', model: e.target.value })}
                >
                  {orchestrator.models.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.label}
                    </option>
                  ))}
                </select>
              }
            />
            <SettingsRow
              className="items-start"
              descriptionOutsideLabel
              label={<span id={`${cleanId}-label`}>{CLEAN_DICTATION_LABEL}</span>}
              htmlFor={cleanId}
              description={
                <span id={`${cleanId}-help`}>
                  Bottle tidies your dictation with the orchestrator model before it pastes. If that
                  takes too long, it pastes your words as they are.
                </span>
              }
              control={
                <Switch
                  id={cleanId}
                  data-orchestrator-clean
                  checked={orchestrator.cleanDictation}
                  aria-labelledby={`${cleanId}-label`}
                  aria-describedby={`${cleanId}-help`}
                  className={HIT}
                  onCheckedChange={(on) => send({ action: 'orchestrator.clean_dictation', on })}
                />
              }
            />
            <SettingsRow
              className="items-start"
              descriptionOutsideLabel
              label={<span id={`${engineId}-label`}>{DICTATION_ENGINE_LABEL}</span>}
              description={
                <span id={`${engineId}-help`} data-dictation-engine-help>
                  {orchestrator.cloudDictation
                    ? DICTATION_ENGINE_DISCLOSURE
                    : DICTATION_ENGINE_LOCAL}
                </span>
              }
              control={
                <RadioGroup
                  aria-labelledby={`${engineId}-label`}
                  aria-describedby={`${engineId}-help`}
                  value={orchestrator.cloudDictation ? 'databricks' : 'local'}
                  orientation="horizontal"
                  className="flex gap-[var(--mog-space-4)]"
                  data-dictation-engine
                  onValueChange={(v) =>
                    send({ action: 'orchestrator.cloud_dictation', on: v === 'databricks' })
                  }
                >
                  <label className="flex items-center gap-[var(--mog-space-2)] text-[var(--mog-text-md)]">
                    <RadioGroupItem value="local" data-engine="local" />
                    On this Mac
                  </label>
                  <label className="flex items-center gap-[var(--mog-space-2)] text-[var(--mog-text-md)]">
                    <RadioGroupItem value="databricks" data-engine="databricks" />
                    Databricks
                  </label>
                </RadioGroup>
              }
            />
            {reason !== undefined && (
              <div
                className="flex items-center justify-between gap-[var(--mog-space-3)] px-[var(--mog-space-4)] py-[var(--mog-space-2)]"
                data-orchestrator-status
              >
                <span
                  id={`${modelId}-status`}
                  role="status"
                  className="text-[var(--mog-text-xs)] text-muted-foreground"
                >
                  {reason}
                </span>
                {(state === 'unreachable' || state === 'no_model') && (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="min-h-6"
                    data-orchestrator-recheck
                    onClick={() => send({ action: 'orchestrator.recheck' })}
                  >
                    Check again
                  </Button>
                )}
              </div>
            )}
          </>
        )}
        <SettingsRow
          label={<span id={budgetId}>Genie calls per question</span>}
          description={BUDGET_HELP}
          control={
            <RadioGroup
              aria-labelledby={budgetId}
              value={String(list.agent.genieCallBudget)}
              orientation="horizontal"
              className="flex gap-[var(--mog-space-4)]"
              data-agent-budget
              onValueChange={(v) =>
                send({
                  action: 'workspace.budget',
                  genieCallBudget: Number(v) as HubGenieCallBudget,
                })
              }
            >
              {BUDGETS.map((n) => (
                <label
                  key={n}
                  className="flex items-center gap-[var(--mog-space-2)] text-[var(--mog-text-md)]"
                >
                  <RadioGroupItem value={String(n)} data-budget={n} />
                  {n}
                </label>
              ))}
            </RadioGroup>
          }
        />
      </SettingsGroup>
      <p
        data-agent-disclosure
        className="mt-[var(--mog-space-2)] text-[var(--mog-text-xs)] text-muted-foreground"
      >
        {disclosureLines(homeName).join(' ')}
      </p>
    </section>
  );
}
