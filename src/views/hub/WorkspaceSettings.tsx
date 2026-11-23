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
import {
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type RefObject,
} from 'react';
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
import { Input } from '../ui/input';
import { Switch } from '../ui/switch';
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

const STATUS_TEXT: Record<HubSignIn, string> = {
  signed_in: 'Signed in',
  expired: 'Expired',
  unknown: 'Not checked yet',
  unsupported: 'Not supported',
  missing: 'Not in your CLI config',
};

/** A row the brain cannot use: no switch to turn on, no home to make. */
const unusable = (r: HubWorkspaceRow) => r.signIn === 'unsupported' || r.signIn === 'missing';

/** The "…" menu of a row: Rename and Colour. Arrow keys move, Escape closes. */
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
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const items = useRef<(HTMLButtonElement | null)[]>([]);
  const menuId = useId();

  useEffect(() => {
    if (!open) return;
    items.current[0]?.focus();
    const away = (e: PointerEvent) => {
      if (wrap.current && !wrap.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('pointerdown', away);
    return () => document.removeEventListener('pointerdown', away);
  }, [open]);

  const onKey = (e: ReactKeyboardEvent) => {
    if (!open) return;
    const els = items.current.filter((x): x is HTMLButtonElement => x !== null);
    const i = els.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      setOpen(false);
      buttonRef.current?.focus();
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const n = els.length;
      els[(i + (e.key === 'ArrowDown' ? 1 : n - 1) + n) % n]?.focus();
    } else if (e.key === 'Tab') {
      setOpen(false);
    }
  };
  const choose = (fn: () => void) => () => {
    setOpen(false);
    fn();
  };
  return (
    <div ref={wrap} className="relative" onKeyDown={onKey}>
      <Button
        ref={buttonRef as never}
        type="button"
        variant="ghost"
        size="icon"
        className="size-8"
        aria-label={`Options for ${name}`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        disabled={disabled}
        data-row-menu
        onClick={() => setOpen((o) => !o)}
      >
        <EllipsisIcon className="size-4" />
      </Button>
      {open && (
        <div
          id={menuId}
          role="menu"
          aria-label={`Options for ${name}`}
          className="absolute right-0 top-[calc(100%+4px)] z-10 flex min-w-32 flex-col rounded-[var(--radius-md)] border border-border bg-popover p-1 shadow-md"
        >
          {(
            [
              ['Rename', onRename],
              ['Colour', onColour],
            ] as const
          ).map(([label, fn], i) => (
            <button
              key={label}
              ref={(el) => {
                items.current[i] = el;
              }}
              type="button"
              role="menuitem"
              data-slot="menu-item"
              className="rounded-sm px-3 py-1.5 text-left text-[var(--mog-text-md)] outline-none hover:bg-accent focus-visible:bg-accent"
              onClick={choose(fn)}
            >
              {label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function RenameForm({ row, onDone }: { row: HubWorkspaceRow; onDone: () => void }) {
  const [name, setName] = useState(row.name);
  const id = useId();
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
        autoFocus
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
  const menuBtn = useRef<HTMLButtonElement>(null);
  const makeHomeBtn = useRef<HTMLButtonElement>(null);
  const cancelBtn = useRef<HTMLButtonElement>(null);

  // Focus the safe choice when the confirm opens.
  useEffect(() => {
    if (panel === 'confirm') cancelBtn.current?.focus();
  }, [panel]);
  // The row became home (or stopped being home): drop an open confirm.
  const [wasHome, setWasHome] = useState(row.isHome);
  if (wasHome !== row.isHome) {
    setWasHome(row.isHome);
    setPanel(undefined);
  }

  // WCAG 2.4.3: a closing panel gives focus back to the control that opened it.
  const closeTo = (focus: RefObject<HTMLButtonElement | null>) => {
    setPanel(undefined);
    requestAnimationFrame(() => focus.current?.focus());
  };

  return (
    <li
      data-workspace-row={row.profile}
      data-home={String(row.isHome)}
      data-busy={String(busy)}
      aria-busy={busy}
      className="px-[var(--mog-space-4)] py-[var(--mog-space-3)]"
    >
      <div className="flex items-center gap-[var(--mog-space-3)]">
        <ColorDot colorVariant={row.colorVariant} profile={row.profile} />
        <div className="flex min-w-0 flex-1 flex-col gap-[var(--mog-space-1)]">
          {panel === 'rename' ? (
            <RenameForm row={row} onDone={() => closeTo(menuBtn)} />
          ) : (
            <span className="flex min-w-0 items-center gap-[var(--mog-space-2)]">
              <span data-workspace-name className="truncate text-[var(--mog-text-md)] font-medium">
                {row.name}
              </span>
              {row.isHome && (
                <span
                  data-home-badge
                  className="shrink-0 rounded-full bg-primary px-2 py-px text-[var(--mog-text-xs)] font-medium text-primary-foreground"
                >
                  Home
                </span>
              )}
            </span>
          )}
          <span className="flex min-w-0 flex-wrap items-center gap-x-[var(--mog-space-2)] text-[var(--mog-text-xs)] text-muted-foreground">
            {row.host !== '' && (
              <span data-workspace-host className="truncate">
                {row.host}
              </span>
            )}
            <span
              data-workspace-status={row.signIn}
              className={row.signIn === 'expired' ? 'text-foreground' : undefined}
            >
              {row.host !== '' && <span aria-hidden="true">· </span>}
              {STATUS_TEXT[row.signIn]}
            </span>
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-[var(--mog-space-2)]">
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
          {!row.isHome && panel !== 'confirm' && (
            <Button
              ref={makeHomeBtn}
              type="button"
              variant="outline"
              size="sm"
              disabled={busy || unusable(row)}
              aria-label={`Make ${row.name} home`}
              data-make-home
              onClick={() => setPanel('confirm')}
            >
              Make home
            </Button>
          )}
          {!row.isHome && (
            <Switch
              checked={row.available}
              disabled={busy || (!row.available && unusable(row))}
              aria-label={`Available to your MOG: ${row.name}`}
              onCheckedChange={(available) =>
                send({ action: 'workspace.set_available', profile: row.profile, available })
              }
            />
          )}
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
          className="mt-[var(--mog-space-3)] rounded-[var(--radius-md)] bg-muted/50 p-[var(--mog-space-3)]"
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.stopPropagation();
              closeTo(panel === 'confirm' ? makeHomeBtn : menuBtn);
            }
          }}
        >
          {panel === 'confirm' ? (
            <>
              <p className="text-[var(--mog-text-md)]">
                {`Make ${row.name} your home? Plain questions go there first.`}
              </p>
              <div className="mt-[var(--mog-space-3)] flex gap-[var(--mog-space-2)]">
                <Button
                  type="button"
                  size="sm"
                  data-confirm-home
                  onClick={() => {
                    send({
                      action: 'workspace.make_home',
                      profile: row.profile,
                      confirmed: true,
                    });
                    setPanel(undefined);
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
            </>
          ) : (
            <WorkspaceColor
              name={row.name}
              profile={row.profile}
              colorVariant={row.colorVariant}
              onPick={(colorVariant) => {
                send({ action: 'workspace.edit', profile: row.profile, colorVariant });
                closeTo(menuBtn);
              }}
            />
          )}
        </motion.div>
      )}
    </li>
  );
}

export function WorkspaceSettings({
  list,
  homeDomains = [],
}: {
  list: HubWorkspaceList | undefined;
  /** MOG states that carry the home domain picker (PG-18, PG-20). */
  homeDomains?: readonly MogViewState[];
}) {
  const notice = list?.notice;
  return (
    <section
      data-view="workspace-settings"
      aria-labelledby="workspace-settings-title"
      className="hub-section"
    >
      <h2 id="workspace-settings-title">Workspace</h2>
      {/* A polite live region: always in the page, so a new notice is read when it appears. */}
      <p
        role="status"
        aria-live="polite"
        data-workspace-notice={notice?.kind ?? 'none'}
        className={notice?.kind === 'error' ? 'hub-error' : 'hub-muted'}
      >
        {notice?.text ?? ''}
      </p>
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
