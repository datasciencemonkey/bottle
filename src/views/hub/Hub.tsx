/** V1 hub view (thin): lists the MOGs the brain reports and sends selections with view_emit. */
import { useCallback, useState } from 'react';
import type { Mood, MogId } from '../../shared/interfaces';
import { asMogViewState, emitView, useViewEvent } from '../view-ipc';
import { About } from './About';
import { DataPanel } from './DataDisclosures';

export function Hub() {
  const [moods, setMoods] = useState<ReadonlyMap<MogId, Mood>>(new Map());

  const onState = useCallback((p: unknown) => {
    const s = asMogViewState(p);
    if (s) setMoods((m) => new Map(m).set(s.mogId, s.mood));
  }, []);
  useViewEvent('mog.view_state', onState);

  // TODO(#43, #77, #103): the wizard, settings and history.
  return (
    <main>
      <h1>Bottle</h1>
      <p>Databricks Labs / experimental</p>
      <ul aria-label="MOGs">
        {[...moods].map(([id, mood]) => (
          <li key={id} data-mog={id} data-mood={mood}>
            <button type="button" onClick={() => void emitView(id, 'click')}>
              MOG {id.slice(0, 8)} ({mood})
            </button>
          </li>
        ))}
      </ul>
      <DataPanel />
      <About />
    </main>
  );
}
