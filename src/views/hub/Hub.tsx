/** V1 hub view (thin): lists the MOGs the brain reports and sends selections with view_emit. */
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
import { CliInstallStep } from './CliInstallStep';
import { CliSettings } from './CliSettings';
import { ConnectWorkspace } from './ConnectWorkspace';
import { Conversations } from './Conversations';
import { DataPanel } from './DataDisclosures';
import { DictationNotices } from './DictationNotices';
import { LegalNotice } from './LegalNotice';
import { MicPreference } from './MicPreference';
import { ForgetDialogHost } from './ForgetDialogHost';
import { MogList } from './MogList';
import { PermissionNotices } from './PermissionNotices';
import { SttSetupStep } from './SttSetupStep';
import { WipeSettings } from './WipeSettings';
import { Workspaces } from './Workspaces';

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

type HubView = 'conversations' | 'home' | 'workspaces';

/**
 * The hub navigation (#103, #560): Conversations first, then the MOGs and settings, then
 * Workspaces. TODO(#560): once branch `issue/ui-1-tokens-spacing` pushes its `NAV_ITEMS` sidebar
 * (`src/views/hub/nav.ts`, `'workspaces'` id reserved there), rebase this onto it instead of this
 * inline nav.
 */
function HubNav({ view, onView }: { view: HubView; onView: (v: HubView) => void }) {
  const item = (v: HubView, label: string) => (
    <button
      type="button"
      className="hub-nav-item"
      aria-current={view === v ? 'page' : undefined}
      onClick={() => onView(v)}
    >
      {label}
    </button>
  );
  return (
    <nav aria-label="Hub" className="hub-nav">
      {item('conversations', 'Conversations')}
      {item('home', 'MOGs and settings')}
      {item('workspaces', 'Workspaces')}
    </nav>
  );
}

export function Hub() {
  // The hub opens on the MOGs and settings, where the first-run steps are. A double-click on the
  // MOG or the card opens Conversations (#103).
  const [view, setView] = useState<HubView>('home');
  const showConversations = useCallback(() => setView('conversations'), []);
  const [mogs, setMogs] = useState<ReadonlyMap<MogId, MogViewState>>(new Map());

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
        <LegalNotice
          key={notice.version}
          version={notice.version}
          storeError={notice.storeError === true}
          onContinue={(v) => void acceptNotice(v).catch(() => undefined)}
        />
      </main>
    );
  }

  // TODO(#43, #77): the wizard and settings.
  return (
    <main className={view === 'conversations' ? 'hub hub--wide' : 'hub'}>
      <HubHeader />
      <HubNav view={view} onView={setView} />
      <Conversations active={view === 'conversations'} onOpen={showConversations} />
      <div data-hub-view="home" hidden={view !== 'home'}>
        {/* ADR 0101 (CAP-36, CAP-37): a missing OS permission shows first; without Input
            Monitoring the talk key does nothing. */}
        <PermissionNotices />
        {/* W-17c (#161): the CLI install step comes before the workspace setup. */}
        <CliInstallStep />
        <DictationNotices />
        {/* #162 (W-07f): the brain opens it at first run when no usable profile exists. */}
        <ConnectWorkspace />
        {/* The live moods the MOG windows report: one quiet chip per MOG (empty until one runs). */}
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
        <MicPreference />
        <SttSetupStep />
        <WipeSettings />
        <CliSettings />
        <DataPanel />
        <About />
      </div>
      <div data-hub-view="workspaces" hidden={view !== 'workspaces'}>
        <Workspaces />
      </div>
    </main>
  );
}
