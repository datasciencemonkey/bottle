/** V1 hub view (thin): shows the hub areas, Settings → Workspace included, and sends input with view_emit. */
import { PanelLeftIcon } from 'lucide-react';
import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { HUB_WORKSPACES_EVENT, asHubWorkspaceList } from '../../shared/hub-workspaces';
import type { HubRecording } from '../../shared/recording-view';
import {
  acceptNotice,
  asMogViewState,
  useHubNotice,
  useViewEvent,
  type MogViewState,
} from '../view-ipc';
import type { MogId } from '../../shared/interfaces';
import { About } from './About';
import { Button } from '../ui/button';
import { CliInstallStep } from './CliInstallStep';
import { CliSettings } from './CliSettings';
import { DictationWords } from './DictationWords';
import { ConnectWorkspace } from './ConnectWorkspace';
import { Conversations } from './Conversations';
import { MemoryArea } from './Memory';
import { DataPanel } from './DataDisclosures';
import { DictationNotices } from './DictationNotices';
import { ForgetDialogHost } from './ForgetDialogHost';
import { HomePickDialog } from './HomePickDialog';
import { HubSidebar, sidebarWidth } from './HubSidebar';
import { Home } from './Home';
import { LegalNotice } from './LegalNotice';
import { MicPreference } from './MicPreference';
import { DEFAULT_HUB_VIEW, NAV_ITEMS, type HubView } from './nav';
import { PermissionNotices } from './PermissionNotices';
import { RecIndicator, RecordControl } from './RecordControl';
import { Recordings } from './Recordings';
import { RecordingSettings } from './RecordingSettings';
import { useHubRecording } from './recording-ipc';
import { SttModelStep } from './SttModelStep';
import { sttModelApi } from './sttModelApi';
import { StoreNotice } from './StoreNotice';
import { ThreadComposer } from './ThreadComposer';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '../ui/tooltip';
import type { HubOrchestratorView } from '../../shared/hub-orchestrator';
import { useHubAgent } from './use-hub-agent';
import { useHubOrchestrator } from './use-hub-orchestrator';
import { useHubSidebar } from './use-hub-sidebar';
import { WipeSettings } from './WipeSettings';
import { useHubWorkspaces, WorkspaceSettings } from './WorkspaceSettings';
import type { HubWorkspaceList } from '../../shared/hub-workspaces';

/** Smooth scrolling, unless the user asked for reduced motion. */
function scrollBehavior(): ScrollBehavior {
  return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth';
}

/** #145: the reply composer of a thread's detail, the genie-followup hub→brain seam. */
const threadComposer = (thread: Parameters<typeof ThreadComposer>[0]['thread']) => (
  <ThreadComposer thread={thread} />
);

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
      {/* #162 (W-07f): the brain opens it at first run when no usable profile exists, or from
          a later `connect_open` (Settings, "Connect another workspace…"). Renders nothing with
          no model, or the brain's explicit `closed` step, so a working profile shows no panel
          here (review of #602). */}
      <ConnectWorkspace />
      {/* #413, #45, #529 (ADR 0104): the Local speech step, over the real `stt_model_*`
          commands. It shows only while the model is pending (not downloaded, partly
          downloaded, or declined) and renders nothing once the model is installed and checked. */}
      <SttModelStep api={sttModelApi} />
    </div>
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
  workspaces,
  homeDomains,
  onChoose,
  agentBusy,
  orchestrator,
}: {
  hidden: boolean;
  agentBusy: boolean;
  orchestrator: HubOrchestratorView | undefined;
  recording: HubRecording | undefined;
  workspaces: HubWorkspaceList | undefined;
  homeDomains: readonly MogViewState[];
  onChoose: () => void;
}) {
  return (
    <section data-hub-view="settings" aria-label="Settings" hidden={hidden}>
      {/* ADR 0117: one row for each workspace, with the switch "Available to your MOG". It ends
          with the "Connect another workspace…" row (#162 follow-up, W-07e), the one deliberate
          way to open "Connect a workspace" again once a usable profile exists. */}
      <WorkspaceSettings
        list={workspaces}
        homeDomains={homeDomains}
        onChoose={onChoose}
        agentBusy={agentBusy}
        orchestrator={orchestrator}
      />
      <MicPreference />
      <DictationWords />
      <RecordingSettings model={recording} />
      <WipeSettings />
      <CliSettings />
      <DataPanel />
      <About />
    </section>
  );
}

export function Hub() {
  // The hub opens on Home (the launch screen); the Setup region (above) shows first-run steps on
  // every area, so a pending step is never hidden. A purpose open switches area (below).
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
  const lastNotice = useRef('');
  // A workspace list that carries a notice (a result of a switch, a sign-in or a pick) shows
  // Settings, where the Workspace section and its notice live.
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void listen<unknown>(
      HUB_WORKSPACES_EVENT,
      (e) => {
        const n = asHubWorkspaceList(e.payload)?.notice;
        if (n === undefined) {
          lastNotice.current = '';
          return;
        }
        // Only a new notice moves the hub (a republish of the same one does not).
        const key = `${n.kind}|${n.text}`;
        if (key === lastNotice.current) return;
        lastNotice.current = key;
        setView('settings');
        requestAnimationFrame(() => {
          const h = document.getElementById('workspace-settings-title');
          h?.scrollIntoView?.({ block: 'start', behavior: scrollBehavior() });
          h?.focus({ preventScroll: true });
        });
      },
      { target: getCurrentWindow().label },
    ).then((u) => {
      if (gone) u();
      else off = u;
    });
    return () => {
      gone = true;
      off?.();
    };
  }, []);
  const recording = useHubRecording();
  const openRecordingSettings = useCallback(() => {
    pickView('settings');
    // After the view shows: move to the section, and focus its heading for screen readers.
    requestAnimationFrame(() => {
      const h = document.getElementById('recording-settings-title');
      h?.scrollIntoView?.({ block: 'start', behavior: scrollBehavior() });
      h?.focus({ preventScroll: true });
    });
  }, [pickView]);
  const onState = useCallback((p: unknown) => {
    const s = asMogViewState(p);
    if (s) setMogs((m) => new Map(m).set(s.mogId, s));
  }, []);
  const control = recording?.control;
  const live = control?.phase === 'recording' || control?.phase === 'stopping';

  useViewEvent('mog.view_state', onState);
  const notice = useHubNotice();
  const workspaces = useHubWorkspaces();
  const agent = useHubAgent();
  const orchestrator = useHubOrchestrator();
  const homeDomains = [...mogs.values()].filter((m) => m.homeDomain !== undefined);
  const hp = workspaces?.homePick;
  const pickKey = hp ? `${hp.suggested}|${hp.candidates.join(',')}` : '';
  // The key of the pick the user hid. A different pick shows again; "Choose…" clears it.
  const [pickDismissed, setPickDismissed] = useState('');
  const onChoose = useCallback(() => setPickDismissed(''), []);
  const pickNames = new Map((workspaces?.rows ?? []).map((r) => [r.profile, r.name] as const));

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
  // Set on `<main>`, not `.hub-body` (ui-1, review of #602 round 4, blocker 1): a pinned rail
  // pushes `.hub-content` over by its own width instead of overlaying it (review of #602,
  // finding 3), and `.hub`'s own header row (`.hub-title`/`.hub-labs`, below) needs the same
  // push — they are direct children of `main`, outside `.hub-body` entirely (AT-37), so a
  // variable set only on `.hub-body` never reached them. A custom property set here still
  // inherits down into `.hub-body`/`.hub-content`/`.conv`/`.rec-area` exactly as before, just
  // from one level higher. 0 (unset) when unpinned or hidden, so an overlay still costs no
  // reflow.
  const hubBodyStyle = { '--hub-sidebar-width': `${sidebarWidth(sidebar)}px` } as CSSProperties;

  // TODO(#43): the onboarding wizard.
  // Conversations, Memory and Recordings are full-height list+detail layouts (`hub--wide`);
  // Home and Settings keep the comfortable reading width (hub.css).
  const wide = view === 'conversations' || view === 'memory' || view === 'recordings';
  return (
    <main className={wide ? 'hub hub--wide' : 'hub'} style={hubBodyStyle}>
      <HubHeader />
      {/* ADR 0105: why macOS asks for the password, or that history is off. Stays a direct
          sibling of the header, outside the sidebar and every area, so it shows regardless of
          which area or sidebar state is active. */}
      <StoreNotice />
      <div className="hub-body">
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
                    // No elevated z-index (ui-1, review of #602 round 4, blocker 2): the
                    // earlier `relative z-30` kept the toggle clickable above an open overlay,
                    // but the toggle sits where the sidebar's own first nav item does,
                    // so a click there closed the sidebar instead of opening that item. The panel
                    // (z-20 in HubSidebar.tsx) now simply covers the toggle while it is open, the
                    // way a native macOS sidebar covers whatever is under it — Esc, an outside
                    // click and the panel's own collapse/pin buttons still close it.
                    className="size-7 shrink-0"
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
          <Home hidden={view !== 'home'} />
          <Conversations
            active={view === 'conversations'}
            onOpen={showConversations}
            composer={threadComposer}
            agent={agent}
            agentMogId={[...mogs.keys()][0]}
          />
          <MemoryArea active={view === 'memory'} forgetMogId={[...mogs.keys()][0]} />
          <Recordings
            active={view === 'recordings'}
            onOpenSettings={openRecordingSettings}
            header={<RecordControl model={recording} onOpenSettings={openRecordingSettings} />}
          />
          <SettingsArea
            hidden={view !== 'settings'}
            recording={recording}
            workspaces={workspaces}
            homeDomains={homeDomains}
            onChoose={onChoose}
            agentBusy={agent?.live !== undefined || agent?.session.state === 'busy'}
            orchestrator={orchestrator}
          />
          <ForgetDialogHost />
          {/* ADR 0117 (FLAG 5): the one-time home pick shows over every area while it is set. */}
          <HomePickDialog
            homePick={workspaces?.homePick}
            names={pickNames}
            open={pickKey !== pickDismissed}
            onDismiss={() => setPickDismissed(pickKey)}
          />
        </div>
      </div>
    </main>
  );
}
