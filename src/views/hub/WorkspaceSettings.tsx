/**
 * Settings → Workspace (ADR 0117, one MOG with workspaces; S1-C). Thin view (ARC-2, ARC-4): it
 * renders the `HubWorkspaceList` the brain sends on `hub:workspaces` and sends one
 * `WorkspaceAction` per user action with `view_emit`. The brain checks every value and writes
 * every result text. The view keeps only form state (an open menu, a rename, a confirm).
 *
 * One row for each CLI profile, home first: colour swatch, name, muted host, sign-in status. The
 * home row has a "Home" badge and no switch. Every other row has [Make home] and the switch
 * "Available to your MOG". The "Connect another workspace…" row stays at the bottom.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { EllipsisIcon, Loader2Icon } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useEffect, useId, useRef, useState, type RefObject } from 'react';
import {
  asHubWorkspaceList,
  HUB_SCOPE,
  HUB_WORKSPACES_EVENT,
  type HubSignIn,
  type HubWorkspaceList,
  type HubWorkspaceRow,
  type WorkspaceAction,
} from '../../shared/hub-workspaces';
import { Button } from '../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { Input } from '../ui/input';
import { Switch } from '../ui/switch';
import { cn } from '../ui/utils';
import { DomainPicker } from '../DomainPicker';
import { askBrain, emitView, type MogViewState } from '../view-ipc';
import { ConnectAnotherWorkspace } from './ConnectAnotherWorkspace';
import { SettingsGroup } from './SettingsGroup';
import { ColorDot, WorkspaceColor } from './WorkspaceColor';

const NAME_MAX = 64;

function send(action: WorkspaceAction): void {
  void emitView(HUB_SCOPE, 'click', action).catch(() => undefined);
}

/**
 * The list the brain sends to this window, or undefined before the brain answers. Sends
 * `workspaces.ready` once the listener is on (ARC-5: only events sent to this window).
 */
export function useHubWorkspaces(): HubWorkspaceList | undefined {
  const [list, setList] = useState<HubWorkspaceList | undefined>(undefined);
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void listen<unknown>(
      HUB_WORKSPACES_EVENT,
      (e) => {
        const l = asHubWorkspaceList(e.payload);
        if (l) setList(l);
      },
      { target: getCurrentWindow().label },
    ).then((u) => {
      if (gone) u();
      else {
        off = u;
        askBrain(HUB_SCOPE, 'click', { action: 'workspaces.ready' } satisfies WorkspaceAction);
      }
    });
    return () => {
      gone = true;
      off?.();
    };
  }, []);
  return list;
}

/** Why a row cannot be turned on or made home (M7). */
const UNUSABLE_REASON: Partial<Record<HubSignIn, string>> = {
  unsupported: 'Bottle cannot use this profile type.',
  missing: 'This profile is not in ~/.databrickscfg.',
};

const STATUS_TEXT: Record<HubSignIn, string> = {
  signed_in: 'Signed in',
  expired: 'Expired',
  unknown: 'Not checked yet',
  unsupported: 'Not supported',
  missing: 'Not in your CLI config',
};

/** A row the brain cannot use: no switch to turn on, no home to make. */
const unusable = (r: HubWorkspaceRow) => r.signIn === 'unsupported' || r.signIn === 'missing';

/** The "…" menu of a row: Rename and Colour (shadcn DropdownMenu, in a portal: never clipped). */
function RowMenu({
  name,
  disabled,
  onRename,
  onColour,
  buttonRef,
}: {
  name: string;
  disabled: boolean;
  onRename: () => void;
  onColour: () => void;
  buttonRef: RefObject<HTMLButtonElement | null>;
}) {
  // A chosen item opens a panel that takes focus itself: Radix must not return focus to the
  // trigger when the menu closes.
  const chosen = useRef(false);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          ref={buttonRef}
          type="button"
          variant="ghost"
          size="icon"
          className="size-8"
          aria-label={`Options for ${name}`}
          disabled={disabled}
          data-row-menu
        >
          <EllipsisIcon className="size-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        aria-label={`Options for ${name}`}
        onCloseAutoFocus={(e) => {
          if (chosen.current) e.preventDefault();
          chosen.current = false;
        }}
      >
        <DropdownMenuItem
          onSelect={() => {
            chosen.current = true;
            onRename();
          }}
        >
          Rename
        </DropdownMenuItem>
        <DropdownMenuItem
          onSelect={() => {
            chosen.current = true;
            onColour();
          }}
        >
          Colour
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function RenameForm({ row, onDone }: { row: HubWorkspaceRow; onDone: () => void }) {
  const [name, setName] = useState(row.name);
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  // A frame later: the menu that opened this form is still closing and would drop focus.
  useEffect(() => {
    const f = requestAnimationFrame(() => {
      input.current?.focus();
      input.current?.select();
    });
    return () => cancelAnimationFrame(f);
  }, []);
  const trimmed = name.trim();
  const valid = trimmed.length > 0 && trimmed.length <= NAME_MAX;
  return (
    <form
      aria-label={`Rename ${row.name}`}
      className="flex flex-wrap items-center gap-[var(--mog-space-2)]"
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.preventDefault();
          e.stopPropagation();
          onDone();
        }
      }}
      onSubmit={(e) => {
        e.preventDefault();
        if (!valid) return;
        if (trimmed !== row.name) {
          send({ action: 'workspace.edit', profile: row.profile, name: trimmed });
        }
        onDone();
      }}
    >
      <label htmlFor={id} className="sr-only">
        {`Name of ${row.name}`}
      </label>
      <Input
        id={id}
        value={name}
        maxLength={NAME_MAX}
        ref={input}
        className="h-8 max-w-64"
        aria-invalid={!valid}
        onChange={(e) => setName(e.target.value)}
      />
      <Button type="submit" size="sm" disabled={!valid}>
        Save
      </Button>
      <Button type="button" size="sm" variant="outline" onClick={onDone}>
        Cancel
      </Button>
    </form>
  );
}

type Panel = 'rename' | 'colour' | 'confirm' | undefined;

function WorkspaceRow({ row, busy }: { row: HubWorkspaceRow; busy: boolean }) {
  const [panel, setPanel] = useState<Panel>(undefined);
  const reduce = useReducedMotion();
  const li = useRef<HTMLLIElement>(null);
  const menuBtn = useRef<HTMLButtonElement>(null);
  const makeHomeBtn = useRef<HTMLButtonElement>(null);
  const cancelBtn = useRef<HTMLButtonElement>(null);
  const questionId = useId();

  // Focus the safe choice when the confirm opens.
  useEffect(() => {
    if (panel === 'confirm') cancelBtn.current?.focus();
  }, [panel]);
  // A Colour panel closes on a pointer press outside the row.
  useEffect(() => {
    if (panel !== 'colour') return;
    const away = (e: PointerEvent) => {
      if (li.current && !li.current.contains(e.target as Node)) setPanel(undefined);
    };
    document.addEventListener('pointerdown', away);
    return () => document.removeEventListener('pointerdown', away);
  }, [panel]);
  // The row became home (or stopped being home), or the brain started work on it: drop any panel.
  const [wasHome, setWasHome] = useState(row.isHome);
  if (wasHome !== row.isHome) {
    setWasHome(row.isHome);
    setPanel(undefined);
  }
  if (busy && panel !== undefined) setPanel(undefined);

  // WCAG 2.4.3: a closing panel gives focus back to the control that opened it.
  const closeTo = (focus: RefObject<HTMLButtonElement | null>) => {
    setPanel(undefined);
    requestAnimationFrame(() => focus.current?.focus());
  };
  const reason = UNUSABLE_REASON[row.signIn];

  return (
    <li
      ref={li}
      data-workspace-row={row.profile}
      data-home={String(row.isHome)}
      data-busy={String(busy)}
      aria-busy={busy}
      className="px-[var(--mog-space-4)] py-[var(--mog-space-3)]"
    >
      {/* One line when it fits. Else the controls wrap, as one right-aligned group, to a line of
          their own under the name and host (owner review 2026-11-24): shrink-0 controls beside
          a squeezed text column cut the host to "…", pushed its status to a second line, and
          ran past the card's edge in a narrow window. The text column never goes under 15rem. */}
      <div
        data-workspace-row-line
        className="flex flex-wrap items-center gap-x-[var(--mog-space-3)] gap-y-[var(--mog-space-2)]"
      >
        <ColorDot colorVariant={row.colorVariant} profile={row.profile} />
        <div
          data-workspace-text
          className="flex min-w-0 flex-[1_1_15rem] flex-col gap-[var(--mog-space-1)]"
        >
          {panel === 'rename' ? (
            <RenameForm row={row} onDone={() => closeTo(menuBtn)} />
          ) : (
            <span className="flex min-w-0 items-center gap-[var(--mog-space-2)]">
              <span
                data-workspace-name
                className={cn(
                  'truncate text-[var(--mog-text-md)] font-medium',
                  reason !== undefined && 'text-muted-foreground',
                )}
              >
                {row.name}
              </span>
              {row.isHome && (
                <span
                  data-home-badge
                  className="shrink-0 rounded-full border border-border px-[var(--mog-space-2)] py-px text-[var(--mog-text-xs)] font-medium text-foreground"
                >
                  Home
                </span>
              )}
            </span>
          )}
          {/* The host gives way (an ellipsis, the whole host as a tooltip); "· status" never
              wraps away from it. */}
          <span className="flex min-w-0 items-center gap-x-[var(--mog-space-2)] text-[var(--mog-text-xs)] text-muted-foreground">
            {row.host !== '' && (
              <>
                <span data-workspace-host className="min-w-0 truncate" title={row.host}>
                  {row.host}
                </span>
                <span aria-hidden="true" className="shrink-0">
                  ·
                </span>
              </>
            )}
            <span
              data-workspace-status={row.signIn}
              className={cn(
                'shrink-0 whitespace-nowrap',
                row.signIn === 'expired' && 'text-foreground',
              )}
            >
              {STATUS_TEXT[row.signIn]}
            </span>
          </span>
          {reason !== undefined && (
            <span data-workspace-reason className="text-[var(--mog-text-xs)] text-muted-foreground">
              {reason}
            </span>
          )}
        </div>
        <div
          data-workspace-controls
          className="ml-auto flex max-w-full flex-wrap items-center justify-end gap-[var(--mog-space-2)]"
        >
          {busy && (
            <span role="status" aria-label={`Working on ${row.name}`} data-busy-spinner>
              <Loader2Icon
                className="size-4 animate-spin text-muted-foreground motion-reduce:animate-none"
                aria-hidden="true"
              />
            </span>
          )}
          {row.signIn === 'expired' && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={busy}
              aria-label={`Sign in to ${row.name}`}
              data-sign-in
              onClick={() => send({ action: 'workspace.sign_in', profile: row.profile })}
            >
              Sign in…
            </Button>
          )}
          {!row.isHome && (
            <Button
              ref={makeHomeBtn}
              type="button"
              variant="outline"
              size="sm"
              disabled={busy || unusable(row) || panel === 'confirm'}
              aria-label={`Make home: ${row.name}`}
              aria-expanded={panel === 'confirm'}
              data-make-home
              onClick={() => setPanel('confirm')}
            >
              Make home
            </Button>
          )}
          <span
            data-availability
            className="flex items-center gap-[var(--mog-space-2)] whitespace-nowrap text-[var(--mog-text-xs)] text-muted-foreground"
          >
            {row.isHome ? (
              <span>Always on</span>
            ) : (
              <>
                <span aria-hidden="true">Available to your MOG</span>
                <Switch
                  checked={row.available}
                  disabled={busy || (!row.available && unusable(row))}
                  aria-label={`Available to your MOG: ${row.name}`}
                  onCheckedChange={(available) =>
                    send({ action: 'workspace.set_available', profile: row.profile, available })
                  }
                />
              </>
            )}
          </span>
          <RowMenu
            name={row.name}
            disabled={busy}
            buttonRef={menuBtn}
            onRename={() => setPanel('rename')}
            onColour={() => setPanel('colour')}
          />
        </div>
      </div>
      {(panel === 'confirm' || panel === 'colour') && (
        <motion.div
          key={panel}
          data-row-panel={panel}
          initial={reduce ? false : { opacity: 0, y: -4 }}
          animate={{ opacity: 1, y: 0 }}
          transition={reduce ? { duration: 0 } : { duration: 0.18, ease: [0.16, 1, 0.3, 1] }}
          className="mt-[var(--mog-space-3)] rounded-[var(--radius-md)] border border-border/60 bg-card p-[var(--mog-space-3)]"
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.stopPropagation();
              closeTo(panel === 'confirm' ? makeHomeBtn : menuBtn);
            }
          }}
        >
          {panel === 'confirm' ? (
            <div role="group" aria-labelledby={questionId}>
              <p id={questionId} className="text-[var(--mog-text-md)]">
                {`Make ${row.name} your home? Plain questions go there first.`}
              </p>
              <div className="mt-[var(--mog-space-3)] flex gap-[var(--mog-space-2)]">
                <Button
                  type="button"
                  size="sm"
                  disabled={busy}
                  data-confirm-home
                  onClick={() => {
                    send({ action: 'workspace.make_home', profile: row.profile, confirmed: true });
                    closeTo(menuBtn);
                  }}
                >
                  Make home
                </Button>
                <Button
                  ref={cancelBtn}
                  type="button"
                  size="sm"
                  variant="outline"
                  data-cancel-home
                  onClick={() => closeTo(makeHomeBtn)}
                >
                  Cancel
                </Button>
              </div>
            </div>
          ) : (
            <div className="flex items-center justify-between gap-[var(--mog-space-3)]">
              <WorkspaceColor
                autoFocus
                name={row.name}
                profile={row.profile}
                colorVariant={row.colorVariant}
                onPick={(colorVariant) => {
                  send({ action: 'workspace.edit', profile: row.profile, colorVariant });
                  closeTo(menuBtn);
                }}
              />
              <Button
                type="button"
                variant="ghost"
                size="sm"
                data-colour-done
                onClick={() => closeTo(menuBtn)}
              >
                Done
              </Button>
            </div>
          )}
        </motion.div>
      )}
    </li>
  );
}

export function WorkspaceSettings({
  list,
  homeDomains = [],
  onChoose,
}: {
  list: HubWorkspaceList | undefined;
  /** MOG states that carry the home domain picker (PG-18, PG-20). */
  homeDomains?: readonly MogViewState[];
  /** Reopens the home pick dialog. */
  onChoose?: () => void;
}) {
  const notice = list?.notice;
  // A changed notice is a new alert. The same notice sent again is not; once the notice clears,
  // the same text is new again.
  const noticeKey = notice ? `${notice.kind}|${notice.text}` : '';
  const [seen, setSeen] = useState(noticeKey);
  const [seq, setSeq] = useState(0);
  if (noticeKey !== seen) {
    setSeen(noticeKey);
    if (noticeKey !== '') setSeq((n) => n + 1);
  }
  const noHome = list !== undefined && list.home === null && list.rows.length > 0;
  return (
    <section
      data-view="workspace-settings"
      aria-labelledby="workspace-settings-title"
      className="hub-section"
    >
      <h2 id="workspace-settings-title" tabIndex={-1}>
        Workspace
      </h2>
      {/* A polite live region, always in the page, so a new info notice is read when it shows. */}
      <p
        role="status"
        aria-live="polite"
        data-workspace-notice={notice?.kind ?? 'none'}
        className="hub-muted"
      >
        {notice?.kind === 'info' ? notice.text : ''}
      </p>
      {notice?.kind === 'error' && (
        <p role="alert" key={seq} className="hub-error">
          {notice.text}
        </p>
      )}
      {noHome && (
        <div
          data-no-home
          className="mt-[var(--mog-space-3)] flex items-center justify-between gap-[var(--mog-space-3)] rounded-[var(--radius-md)] border border-border/60 bg-card px-[var(--mog-space-4)] py-[var(--mog-space-3)]"
        >
          <div className="flex flex-col gap-[var(--mog-space-1)]">
            <span className="text-[var(--mog-text-md)] font-medium">Choose a home workspace</span>
            <span className="text-[var(--mog-text-xs)] text-muted-foreground">
              Plain questions go to your home workspace first.
            </span>
          </div>
          {list.homePick && onChoose && (
            <Button type="button" size="sm" data-choose-home onClick={onChoose}>
              Choose…
            </Button>
          )}
        </div>
      )}
      {list === undefined && (
        <p data-workspaces-loading className="hub-muted mt-[var(--mog-space-3)]">
          Checking your workspaces…
        </p>
      )}
      {list !== undefined && list.rows.length === 0 && (
        <p data-workspaces-empty className="hub-muted mt-[var(--mog-space-3)]">
          No workspaces yet. Connect one below.
        </p>
      )}
      {list && list.rows.length > 0 && (
        <SettingsGroup className="mt-[var(--mog-space-3)]">
          <ul aria-label="Workspaces" className="m-0 list-none divide-y divide-border/60 p-0">
            {list.rows.map((r) => (
              <WorkspaceRow key={r.profile} row={r} busy={list.busyProfile === r.profile} />
            ))}
          </ul>
        </SettingsGroup>
      )}
      {homeDomains.length > 0 && (
        <ul aria-label="Home domain" className="mt-[var(--mog-space-3)] list-none p-0">
          {homeDomains.map(
            (m) =>
              m.homeDomain && (
                <li key={m.mogId} data-mog={m.mogId}>
                  <DomainPicker mogId={m.mogId} picker={m.homeDomain} />
                </li>
              ),
          )}
        </ul>
      )}
      <ConnectAnotherWorkspace />
      <p className="mt-[var(--mog-space-2)] text-[var(--mog-text-xs)] text-muted-foreground">
        New workspaces start off. Turn one on to let your MOG use it.
      </p>
    </section>
  );
}
