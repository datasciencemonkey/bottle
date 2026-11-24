/**
 * Settings → Workspace → "MOG agent" (slice 2a, plan Amendment 2 A2). Thin view (ARC-2, ARC-4): it
 * draws what the brain sends in `HubWorkspaceList.agent` and sends one `WorkspaceAction` for each
 * user action. The brain checks every value.
 *
 * - "Genie Code runs on [▾]": the workspace where the one live Genie Code session runs. The
 *   default is home. A profile that Bottle cannot use is disabled with its reason. While a turn
 *   runs, a change applies after the current question.
 * - "Genie calls per question [5 | 10 | 20]": D2, the most Genie calls for one question.
 * - "Genie Code": the status of the pinned binary and of the agent profile, with Install… (after
 *   the user's consent), Check again and a busy state.
 * - "Combine across workspaces" (D1, slice 2b): off by default. It sends `workspace.cross`. When the
 *   admin locks it, it is off, disabled, and says "Set by your admin".
 * - The disclosure line: where a cross-workspace question goes, and what it costs.
 */
import { ChevronDownIcon, Loader2Icon, LockIcon } from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';
import {
  HUB_SCOPE,
  type HubGenieCallBudget,
  type HubWorkspaceList,
  type HubWorkspaceRow,
  type WorkspaceAction,
} from '../../shared/hub-workspaces';
import type { GenieCodeStatus } from '../../shared/ipc';
import { Button } from '../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { RadioGroup, RadioGroupItem } from '../ui/radio-group';
import { Switch } from '../ui/switch';
import { emitView } from '../view-ipc';
import { SettingsGroup, SettingsRow } from './SettingsGroup';

function send(action: WorkspaceAction): void {
  void emitView(HUB_SCOPE, 'click', action).catch(() => undefined);
}

export const APPLIES_AFTER = 'Applies after the current question';
export const COMBINE_LABEL = 'Combine across workspaces';
export const COMBINE_LOCKED = 'Set by your admin';
export const COMBINE_APPLIES = 'Applies to your next question.';
export const COMBINE_OFF_HELP =
  "When off, each workspace's answer is shown to you, and the agent sees only a receipt.";
/** The help of the switch D1 (plan 2b): what the agent can see, and what it can do with it. */
export const combineHelp = (agentName: string): string =>
  `Answers from your other workspaces go to Genie Code in ${agentName}, which runs on that workspace's AI model. With the switch on, Genie Code can see them and combine them. Genie Code can put an answer from one workspace into a question to another workspace.`;
export const BUDGET_HELP = 'The most Genie calls your MOG makes for one question.';
export const BUDGETS: readonly HubGenieCallBudget[] = [5, 10, 20];

/** Why a profile cannot be the agent workspace, or undefined when it can. */
export function unusableReason(r: HubWorkspaceRow): string | undefined {
  if (r.signIn === 'unsupported') return 'Bottle cannot use this profile type.';
  if (r.signIn === 'missing') return 'This profile is not in your CLI config.';
  return undefined;
}

/** The Genie Code status line, and the action that goes with it. */
export function genieCodeLine(
  g: GenieCodeStatus | undefined,
  agentName: string,
): { text: string; action?: 'install' | 'install_copy' | 'recheck'; busy?: boolean } {
  if (g === undefined) return { text: 'Checking Genie Code…', busy: true };
  switch (g.binary) {
    case 'not_installed':
      return { text: 'Not installed', action: 'install' };
    case 'wrong_version':
      return {
        text: `Bottle needs ${g.pinned} (found ${g.version ?? 'another version'})`,
        action: 'install_copy',
      };
    case 'installing':
      return { text: 'Installing Genie Code…', busy: true };
    case 'unsupported_os':
      return { text: 'Genie Code is not available on this Mac yet' };
    case 'ready':
      break;
  }
  switch (g.agentProfileState) {
    case 'ready':
      return { text: `Installed ${g.version ?? g.pinned}` };
    case 'not_eligible':
      return { text: 'Not available in this workspace', action: 'recheck' };
    case 'signed_out':
      return { text: `Sign in to ${agentName} to use Genie Code there` };
    case 'not_u2m':
      return { text: 'This profile type cannot run Genie Code' };
    case 'missing':
      return { text: 'This profile is not in your CLI config' };
    case 'no_home':
      return { text: 'Choose a home workspace first' };
  }
}

/** The disclosure line of the plan (section 5): where the question goes and what it costs. */
export const disclosureLines = (agentName: string): string[] => [
  `When a question needs more than one workspace, Bottle sends it to Genie Code in ${agentName}.`,
  "Genie Code uses that workspace's AI model.",
  "It bills about 30,000 to 50,000 tokens per question to that workspace's Genie budget.",
  'Answers from your other workspaces stay out of it unless you turn on Combine across workspaces.',
];

function AgentWorkspaceMenu({
  list,
  busy,
  current,
}: {
  list: HubWorkspaceList;
  busy: boolean;
  current: HubWorkspaceRow | undefined;
}) {
  const home = list.rows.find((r) => r.isHome);
  const rows = list.rows;
  return (
    <div className="flex flex-col items-end gap-[var(--mog-space-1)]">
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            type="button"
            variant="outline"
            size="sm"
            data-agent-profile
            aria-label={`Genie Code runs on: ${current?.name ?? 'home'}`}
            disabled={rows.length === 0}
          >
            <span className="max-w-48 truncate">{current?.name ?? 'Home'}</span>
            {current?.isHome && <span className="text-muted-foreground">(home)</span>}
            <ChevronDownIcon className="size-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" aria-label="Genie Code runs on">
          <DropdownMenuRadioGroup
            value={current?.profile ?? ''}
            onValueChange={(profile) =>
              // The home workspace is the default: choosing it clears the setting.
              send({
                action: 'workspace.agent_profile',
                profile: profile === home?.profile ? null : profile,
              })
            }
          >
            {rows.map((r) => {
              const reason = unusableReason(r);
              return (
                <DropdownMenuRadioItem
                  key={r.profile}
                  value={r.profile}
                  disabled={reason !== undefined}
                  data-agent-profile-item={r.profile}
                >
                  <span className="flex flex-col">
                    <span>
                      {r.name}
                      {r.isHome && <span className="text-muted-foreground"> (home)</span>}
                    </span>
                    {reason !== undefined && (
                      <span className="text-[var(--mog-text-xs)] text-muted-foreground">
                        {reason}
                      </span>
                    )}
                  </span>
                </DropdownMenuRadioItem>
              );
            })}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
      {busy && (
        <span
          role="status"
          data-agent-applies
          className="text-[var(--mog-text-xs)] text-muted-foreground"
        >
          {APPLIES_AFTER}
        </span>
      )}
    </div>
  );
}

function InstallConfirm({
  pinned,
  copy,
  onDone,
}: {
  pinned: string;
  copy: boolean;
  onDone: () => void;
}) {
  const id = useId();
  const cancel = useRef<HTMLButtonElement>(null);
  // The safe choice has the focus when the question opens.
  useEffect(() => cancel.current?.focus(), []);
  return (
    <div
      role="group"
      aria-labelledby={id}
      data-agent-install-confirm
      className="flex flex-col gap-[var(--mog-space-3)] px-[var(--mog-space-4)] py-[var(--mog-space-3)]"
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation();
          onDone();
        }
      }}
    >
      <p id={id} className="m-0 text-[var(--mog-text-md)]">
        {`Bottle downloads Genie Code ${pinned} and checks its SHA-256 before it runs it. Install it?`}
      </p>
      <div className="flex gap-[var(--mog-space-2)]">
        <Button
          type="button"
          size="sm"
          data-confirm-install
          onClick={() => {
            send({ action: 'genie_code.install', consent: true });
            onDone();
          }}
        >
          {copy ? "Install Bottle's copy" : 'Install'}
        </Button>
        <Button ref={cancel} type="button" size="sm" variant="outline" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

export function AgentSettings({
  list,
  busy = false,
}: {
  list: HubWorkspaceList | undefined;
  /** A turn runs: a change of the agent workspace applies after it. */
  busy?: boolean;
}) {
  const [confirm, setConfirm] = useState(false);
  const [checkingFor, setCheckingFor] = useState<HubWorkspaceList | undefined>(undefined);
  const budgetId = useId();
  const combineId = useId();
  const install = useRef<HTMLButtonElement>(null);
  if (!list) return null;
  // A new list ends the "Check again" wait.
  const checking = checkingFor === list;
  const runProfile = list.agent.runProfile ?? null;
  const home = list.rows.find((r) => r.isHome);
  const current =
    (runProfile ? list.rows.find((r) => r.profile === runProfile) : undefined) ?? home;
  const agentName = current?.name ?? 'your home workspace';
  const locked = list.agent.crossWorkspaceLocked;
  const combineOn = list.agent.crossWorkspaceData && !locked;
  const g = genieCodeLine(list.agent.genieCode, agentName);
  const installing = list.agent.genieCode?.binary === 'installing';
  const closeConfirm = () => {
    setConfirm(false);
    requestAnimationFrame(() => install.current?.focus());
  };
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
        <SettingsRow
          label="Genie Code runs on"
          description="Your MOG plans cross-workspace questions here."
          control={<AgentWorkspaceMenu list={list} busy={busy} current={current} />}
        />
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
          label={<span id={`${combineId}-label`}>{COMBINE_LABEL}</span>}
          htmlFor={locked ? undefined : combineId}
          description={
            <span
              id={`${combineId}-help`}
              data-agent-combine-help
              className="flex flex-col gap-[var(--mog-space-1)]"
            >
              <span>{combineHelp(agentName)}</span>
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
                onCheckedChange={(on) =>
                  send({ action: 'workspace.cross', crossWorkspaceData: on })
                }
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
                className="text-[var(--mog-text-xs)] text-muted-foreground"
              >
                {busy && !locked ? COMBINE_APPLIES : ''}
              </span>
            </span>
          }
        />
        <SettingsRow
          label="Genie Code"
          description={
            <span data-agent-genie-status aria-busy={g.busy === true}>
              {g.text}
            </span>
          }
          control={
            <span className="flex items-center gap-[var(--mog-space-2)]">
              {(g.busy || checking) && (
                <span role="status" aria-label="Working" data-agent-busy>
                  <Loader2Icon
                    className="size-4 animate-spin text-muted-foreground motion-reduce:animate-none"
                    aria-hidden="true"
                  />
                </span>
              )}
              {(g.action === 'install' || g.action === 'install_copy') && (
                <Button
                  ref={install}
                  type="button"
                  variant="outline"
                  size="sm"
                  data-agent-install
                  disabled={installing || confirm}
                  aria-expanded={confirm}
                  onClick={() => setConfirm(true)}
                >
                  {g.action === 'install_copy' ? "Install Bottle's copy…" : 'Install…'}
                </Button>
              )}
              {g.action === 'recheck' && (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  data-agent-recheck
                  disabled={checking}
                  onClick={() => {
                    setCheckingFor(list);
                    send({ action: 'genie_code.recheck' });
                  }}
                >
                  Check again
                </Button>
              )}
            </span>
          }
        />
        {confirm && (
          <InstallConfirm
            pinned={list.agent.genieCode?.pinned ?? ''}
            copy={g.action === 'install_copy'}
            onDone={closeConfirm}
          />
        )}
      </SettingsGroup>
      <p
        data-agent-disclosure
        className="mt-[var(--mog-space-2)] text-[var(--mog-text-xs)] text-muted-foreground"
      >
        {disclosureLines(agentName).join(' ')}
      </p>
    </section>
  );
}
