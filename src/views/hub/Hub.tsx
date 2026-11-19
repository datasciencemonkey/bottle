/** V1 hub view (thin): lists the MOGs the brain reports and sends selections with view_emit. */
import { PanelLeftIcon } from 'lucide-react';
import { useCallback, useState } from 'react';
import type { MogId } from '../../shared/interfaces';
import { DomainPicker } from '../DomainPicker';
import {
  acceptNotice,
  asMogViewState,
  emitView,
  useHubNotice,
  useViewEvent,
  type MogViewState,
} from '../view-ipc';
import { About } from './About';
import { Button } from '../ui/button';
import { CliInstallStep } from './CliInstallStep';
import { CliSettings } from './CliSettings';
import { ConnectWorkspace } from './ConnectWorkspace';
import { Conversations } from './Conversations';
import { DataPanel } from './DataDisclosures';
import { DictationNotices } from './DictationNotices';
import { ForgetDialogHost } from './ForgetDialogHost';
import { HubSidebar } from './HubSidebar';
import { LegalNotice } from './LegalNotice';
import { MicPreference } from './MicPreference';
import { MogList } from './MogList';
import { DEFAULT_HUB_VIEW, NAV_ITEMS, type HubView } from './nav';
import { PermissionNotices } from './PermissionNotices';
import { SttSetupStep } from './SttSetupStep';
import { StoreNotice } from './StoreNotice';
import { useHubSidebar } from './use-hub-sidebar';
import { WipeSettings } from './WipeSettings';

/**
 * The product name and the Labs label (MB-10a, MB-10b); the title bar shows the label too. Both
 * stay direct children of `main` (AT-37 reads `main > p`); the CSS sets them on one line.
 */
function HubHeader() {
  return (
    <>
      <h1 className="hub-title">Bottle</h1>
      <p className="hub-labs">Databricks Labs / experimental</p>
    </>
  );
}

/** The setup steps (ui-1, #97, #103): shown above every area while a step is pending. Each of
    these already renders nothing (or a quiet status line) once its step clears. */
function SetupRegion() {
  return (
    <div data-setup-region>
      {/* ADR 0101 (CAP-36, CAP-37): a missing OS permission shows first; without Input
          Monitoring the talk key does nothing. */}
      <PermissionNotices />
      {/* W-17c (#161): the CLI install step comes before the workspace setup. */}
      <CliInstallStep />
      <DictationNotices />
      {/* #162 (W-07f): the brain opens it at first run when no usable profile exists. */}
      <ConnectWorkspace />
    </div>
  );
}

/** The 'mogs' area: the live mood chips, the MOG list and its dialogs (#26, #103). */
function MogsArea({ mogs }: { mogs: ReadonlyMap<MogId, MogViewState> }) {
  return (
    <section data-hub-view="mogs">
      <ul aria-label="MOGs" className="hub-live">
        {[...mogs].map(([id, s]) => (
          <li key={id} data-mog={id} data-mood={s.mood}>
            <button type="button" className="hub-chip" onClick={() => void emitView(id, 'click')}>
              <span className="hub-mood-dot" aria-hidden="true" />
              MOG {id.slice(0, 8)} <span className="hub-muted">({s.mood})</span>
            </button>
            {s.homeDomain && <DomainPicker mogId={id} picker={s.homeDomain} />}
          </li>
        ))}
      </ul>
      <MogList />
      <ForgetDialogHost />
    </section>
  );
}

/** The 'settings' area (#97, #103): one grouped, consistently spaced settings pane. */
function SettingsArea() {
  return (
    <section data-hub-view="settings" aria-label="Settings">
      <MicPreference />
      <SttSetupStep />
      <WipeSettings />
      <CliSettings />
      <DataPanel />
      <About />
    </section>
  );
}

export function Hub() {
  // ui-1 (#97, #103): Conversations is the area the hub opens on; the Setup region (above) shows
  // first-run steps on every area, so the old "home" area's special-casing is gone.
  const [view, setView] = useState<HubView>(DEFAULT_HUB_VIEW);
  const showConversations = useCallback(() => setView('conversations'), []);
  const [mogs, setMogs] = useState<ReadonlyMap<MogId, MogViewState>>(new Map());
  const sidebar = useHubSidebar();

  const onState = useCallback((p: unknown) => {
    const s = asMogViewState(p);
    if (s) setMogs((m) => new Map(m).set(s.mogId, s));
  }, []);
  useViewEvent('mog.view_state', onState);
  const notice = useHubNotice();

  // MB-08a, MB-08b: until the brain reports the current notice as accepted, the hub shows only
  // the notice. A new version gets a fresh, unchecked notice (`key`).
  if (notice && !notice.accepted) {
    return (
      <main className="hub hub--notice">
        <HubHeader />
        {/* ADR 0105: why macOS asks for the password, or that history is off. */}
        <StoreNotice />
        <LegalNotice
          key={notice.version}
          version={notice.version}
          storeError={notice.storeError === true}
          onContinue={(v) => void acceptNotice(v).catch(() => undefined)}
        />
      </main>
    );
  }

  const activeLabel = NAV_ITEMS.find((i) => i.id === view)?.label ?? '';

  // TODO(#43): the onboarding wizard.
  return (
    <main className={view === 'conversations' ? 'hub hub--wide' : 'hub'}>
      <HubHeader />
      {/* ADR 0105: why macOS asks for the password, or that history is off. Stays a direct
          sibling of the header, outside the sidebar and every area, so it shows regardless of
          which area or sidebar state is active. */}
      <StoreNotice />
      <div className="hub-body">
        <HubSidebar state={sidebar} view={view} onView={setView} />
        <div className="hub-content">
          <div className="hub-area-header">
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="size-7 shrink-0"
              data-sidebar-toggle
              aria-pressed={sidebar.visible}
              onClick={sidebar.toggleVisible}
            >
              <PanelLeftIcon className="size-4" />
              <span className="sr-only">Toggle the sidebar (⌘\\ or ⌘B)</span>
            </Button>
            <h2 className="hub-area-title">{activeLabel}</h2>
          </div>
          <SetupRegion />
          <Conversations active={view === 'conversations'} onOpen={showConversations} />
          {view === 'mogs' && <MogsArea mogs={mogs} />}
          {view === 'settings' && <SettingsArea />}
        </div>
      </div>
    </main>
  );
}
