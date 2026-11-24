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
 * - "Combine across workspaces" (D1, slice 2b): off by default. It sends `workspace.cross`. When the
 *   admin locks it, it is off, disabled, and says "Set by your admin".
 * - The disclosure line: where a cross-workspace question goes.
 */
import { LockIcon } from 'lucide-react';
import { useId, useState } from 'react';
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
export const COMBINE_LABEL = 'Combine across workspaces';
export const COMBINE_LOCKED = 'Set by your admin';
export const COMBINE_APPLIES = 'Applies to your next question.';
export const COMBINE_OFF_HELP = 'When off, you see each answer. The agent sees only a receipt.';
/** The help of the model row: where it runs and what it does. */
export const orchestratorHelp = (homeName: string): string =>
  `Runs on ${homeName}'s AI Gateway. Cleans up dictation and answers ⌥G by asking your workspaces' Genies.`;
/** The help of the switch D1 (plan 2b): what the agent can see, and what it can do with it. */
export const combineHelp = (homeName: string): string =>
  `Answers from your other workspaces go to the orchestrator model in ${homeName}. When on, it can read them, combine them, and use one in a question to another workspace.`;
export const BUDGET_HELP = 'The most Genie calls your MOG makes for one question.';
export const BUDGETS: readonly HubGenieCallBudget[] = [5, 10, 20];

/** The disclosure line (section 5 of the plan): where the question goes. */
export const disclosureLines = (homeName: string): string[] => [
  `When you press ⌥G, Bottle sends your question to the orchestrator model in ${homeName}.`,
  'The model decides which of your workspaces to ask. It asks only workspaces you turned on.',
  'Answers from your other workspaces stay out of it unless you turn on Combine across workspaces.',
];

/** A 24px hit area for a control that is smaller than that. */
const HIT = "relative before:absolute before:inset-x-0 before:-inset-y-1 before:content-['']";

export function AgentSettings({
  list,
  orchestrator,
  busy = false,
}: {
  list: HubWorkspaceList | undefined;
  /** The orchestrator view. Undefined until the brain sends it. */
  orchestrator?: HubOrchestratorView | undefined;
  /** A turn runs: a change of the combine switch applies after it. */
  busy?: boolean;
}) {
  // The user changed the switch while a turn ran: only then "Applies to your next question".
  const [changed, setChanged] = useState(false);
  // The turn ended: the note goes (a state reset during render, not in an effect).
  if (!busy && changed) setChanged(false);
  const budgetId = useId();
  const combineId = useId();
  const modelId = useId();
  const cleanId = useId();
  const engineId = useId();
  if (!list) return null;
  const home = list.rows.find((r) => r.isHome);
  const homeName = home?.name ?? 'your home workspace';
  const locked = list.agent.crossWorkspaceLocked;
  const combineOn = list.agent.crossWorkspaceData && !locked;
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
        <SettingsRow
          className="items-start"
          descriptionOutsideLabel
          label={<span id={`${combineId}-label`}>{COMBINE_LABEL}</span>}
          {...(locked ? {} : { htmlFor: combineId })}
          description={
            <span
              id={`${combineId}-help`}
              data-agent-combine-help
              className="flex flex-col gap-[var(--mog-space-1)]"
            >
              <span>{combineHelp(homeName)}</span>
              <span>{COMBINE_OFF_HELP}</span>
            </span>
          }
          control={
            <span className="flex flex-col items-end gap-[var(--mog-space-1)]">
              <Switch
                id={combineId}
                data-agent-combine
                checked={combineOn}
                disabled={locked}
                aria-labelledby={`${combineId}-label`}
                aria-describedby={`${combineId}-help${locked ? ` ${combineId}-locked` : ''}`}
                className={HIT}
                onCheckedChange={(on) => {
                  if (busy) setChanged(true);
                  send({ action: 'workspace.cross', crossWorkspaceData: on });
                }}
              />
              {locked && (
                <span
                  id={`${combineId}-locked`}
                  data-agent-combine-locked
                  className="flex items-center gap-[var(--mog-space-1)] text-[var(--mog-text-xs)] text-foreground"
                >
                  <LockIcon className="size-3" aria-hidden="true" />
                  {COMBINE_LOCKED}
                </span>
              )}
              {/* The live region is always there, so a screen reader hears the text when it fills. */}
              <span
                role="status"
                data-agent-combine-applies
                className="min-h-[calc(var(--mog-text-xs)*var(--mog-leading))] text-[var(--mog-text-xs)] text-muted-foreground"
              >
                {busy && changed && !locked ? COMBINE_APPLIES : ''}
              </span>
            </span>
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
