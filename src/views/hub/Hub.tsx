/** V1 hub view (thin): lists the MOGs the brain reports and sends selections with view_emit. */
import { PanelLeftIcon } from 'lucide-react';
import { useCallback, useState, type CSSProperties } from 'react';
import type { MogId } from '../../shared/interfaces';
import type { HubRecording } from '../../shared/recording-view';
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
import { HubSidebar, sidebarWidth } from './HubSidebar';
import { LegalNotice } from './LegalNotice';
import { MicPreference } from './MicPreference';
import { MogList } from './MogList';
import { DEFAULT_HUB_VIEW, NAV_ITEMS, type HubView } from './nav';
import { PermissionNotices } from './PermissionNotices';
import { RecIndicator, RecordControl } from './RecordControl';
import { Recordings } from './Recordings';
import { RecordingSettings } from './RecordingSettings';
import { useHubRecording } from './recording-ipc';
import { SttSetupStep } from './SttSetupStep';
import { StoreNotice } from './StoreNotice';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '../ui/tooltip';
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

/**
 * The 'mogs' area: the live mood chips, the MOG list and its dialogs (#26, #103). Stays mounted
 * with `hidden` (review of #602), like `Conversations`, rather than unmounting on `view !==
 * 'mogs'`: an unmount would have dropped any open MogList form on a sidebar nav click.
 */
function MogsArea({ hidden, mogs }: { hidden: boolean; mogs: ReadonlyMap<MogId, MogViewState> }) {
  return (
    <section data-hub-view="mogs" hidden={hidden}>
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

/**
 * The 'settings' area (#97, #103): one grouped, consistently spaced settings pane. Stays
 * mounted with `hidden` (review of #602), like `Conversations`: unmounting on `view !==
 * 'settings'` dropped the Wipe scope choice and closed an open Wipe dialog mid-confirm as soon
 * as the user switched areas. RecordingSettings (ADR 0103, merged from main) moved in here from
 * the old combined "home" area.
 */
function SettingsArea({
  hidden,
  recording,
}: {
  hidden: boolean;
  recording: HubRecording | undefined;
}) {
  return (
    <section data-hub-view="settings" aria-label="Settings" hidden={hidden}>
      <MicPreference />
      <RecordingSettings model={recording} />
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
  // review of #602 ("also fix"): picking an area closes an unpinned overlay sidebar, the way a
  // native overlay sidebar closes once you have told it where to go; a pinned sidebar is a
  // permanent rail, so it stays.
  const pickView = useCallback(
    (v: HubView) => {
      setView(v);
      if (!sidebar.pinned) sidebar.requestClose();
    },
    [sidebar],
  );
  const recording = useHubRecording();
  const openRecordingSettings = useCallback(() => {
    pickView('settings');
    // After the view shows: move to the section, and focus its heading for screen readers.
    requestAnimationFrame(() => {
      const h = document.getElementById('recording-settings-title');
      h?.scrollIntoView?.({ block: 'start', behavior: 'smooth' });
      h?.focus({ preventScroll: true });
    });
  }, [pickView]);
  const control = recording?.control;
  const live = control?.phase === 'recording' || control?.phase === 'stopping';

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
  // Pinned open, HubSidebar's panel pushes `.hub-content` over by its own width instead of
  // overlaying it (review of #602, finding 3): `--hub-sidebar-width` is read by `.hub-content`
  // in hub.css, which in turn is what `.conv` and `.rec-area` cancel back out with their own
  // fixed `-32px` margin (review of #602 round 3, blocker 1: `.conv` used to read this variable
  // itself too, which double-subtracted the sidebar's width and left it under the panel instead
  // of beside it). 0 (unset) when unpinned or hidden, so an overlay still costs no reflow.
  const hubBodyStyle = { '--hub-sidebar-width': `${sidebarWidth(sidebar)}px` } as CSSProperties;

  // TODO(#43): the onboarding wizard.
  // Conversations and Recordings are both a full-height list+detail layout (`hub--wide`);
  // MOGs and Settings keep the comfortable reading width (hub.css).
  const wide = view === 'conversations' || view === 'recordings';
  return (
    <main className={wide ? 'hub hub--wide' : 'hub'}>
      <HubHeader />
      {/* ADR 0105: why macOS asks for the password, or that history is off. Stays a direct
          sibling of the header, outside the sidebar and every area, so it shows regardless of
          which area or sidebar state is active. */}
      <StoreNotice />
      <div className="hub-body" style={hubBodyStyle}>
        <HubSidebar state={sidebar} view={view} onView={pickView} />
        <div className="hub-content">
          <div className="hub-area-header">
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    // relative z-30 (ui-1, review of #602 round 3, blocker 2): above the
                    // sidebar panel's own z-20 (HubSidebar.tsx), so the toggle stays reachable
                    // even while an unpinned overlay covers this corner. `.hub-area-header`
                    // itself (hub.css) no longer carries that elevation: only the toggle needs
                    // it, not the area title next to it (which must not paint over the panel).
                    className="relative z-30 size-7 shrink-0"
                    data-sidebar-toggle
                    aria-pressed={sidebar.visible}
                    onClick={sidebar.toggleVisible}
                  >
                    <PanelLeftIcon className="size-4" />
                    <span className="sr-only">{'Toggle the sidebar (⌘\\ or ⌘B)'}</span>
                  </Button>
                </TooltipTrigger>
                <TooltipContent side="bottom">{'Toggle the sidebar (⌘\\ or ⌘B)'}</TooltipContent>
              </Tooltip>
            </TooltipProvider>
            <h2 className="hub-area-title">{activeLabel}</h2>
          </div>
          <SetupRegion />
          {/* ADR 0103: a running recording shows in every view, with the time left and Stop. */}
          {live && view !== 'recordings' && (
            <RecIndicator
              endsAtMs={control?.endsAtMs}
              stopping={control?.phase === 'stopping'}
              compact
            />
          )}
          <Conversations active={view === 'conversations'} onOpen={showConversations} />
          <Recordings
            active={view === 'recordings'}
            onOpenSettings={openRecordingSettings}
            header={<RecordControl model={recording} onOpenSettings={openRecordingSettings} />}
          />
          <MogsArea hidden={view !== 'mogs'} mogs={mogs} />
          <SettingsArea hidden={view !== 'settings'} recording={recording} />
        </div>
      </div>
    </main>
  );
}
