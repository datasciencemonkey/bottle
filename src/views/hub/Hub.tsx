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
import { LegalNotice } from './LegalNotice';
import { MogList } from './MogList';

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
      <main>
        <h1>Bottle</h1>
        <p>Databricks Labs / experimental</p>
        <LegalNotice
          key={notice.version}
          version={notice.version}
          onContinue={(v) => void acceptNotice(v).catch(() => undefined)}
        />
      </main>
    );
  }

  // TODO(#43, #77, #103): the wizard, settings and history.
  return (
    <main>
      <h1>Bottle</h1>
      <p>Databricks Labs / experimental</p>
      <ul aria-label="MOGs">
        {[...mogs].map(([id, s]) => (
          <li key={id} data-mog={id} data-mood={s.mood}>
            <button type="button" onClick={() => void emitView(id, 'click')}>
              MOG {id.slice(0, 8)} ({s.mood})
            </button>
            {s.homeDomain && <DomainPicker mogId={id} picker={s.homeDomain} />}
          </li>
        ))}
      </ul>
      <MogList />
      <DataPanel />
      <About />
    </main>
  );
}
