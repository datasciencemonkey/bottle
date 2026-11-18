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
import { DataPanel } from './DataDisclosures';
import { DictationNotices } from './DictationNotices';
import { LegalNotice } from './LegalNotice';
import { MicPreference } from './MicPreference';
import { ForgetDialogHost } from './ForgetDialogHost';
import { MogList } from './MogList';
import { SttSetupStep } from './SttSetupStep';
import { WipeSettings } from './WipeSettings';

/** The product name and the Labs label (MB-10a, MB-10b); the title bar shows the label too. */
function HubHeader() {
  return (
    <header className="hub-head">
      <h1>Bottle</h1>
      <p className="hub-labs">Databricks Labs / experimental</p>
    </header>
  );
}

export function Hub() {
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

  // TODO(#43, #77, #103): the wizard, settings and history.
  return (
    <main className="hub">
      <HubHeader />
      <DictationNotices />
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
      <DataPanel />
      <About />
    </main>
  );
}
