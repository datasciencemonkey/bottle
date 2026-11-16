/** V2 MOG view entry. The MOG ID comes from the `mog-<id>` window label. */
import './mog.css';
import { mount } from '../mount';
import { ownMogId, viewMicLevels } from '../view-ipc';
import { FrameProbe } from './frame-probe';
import { MogView } from './MogView';

const mogId = ownMogId('mog');
// #13 (ADR 0031): the latency run reads `__bottleLatency.toJSON()` in Web Inspector (#355).
const probe = import.meta.env.VITE_BOTTLE_LATENCY_TRACE === '1' ? new FrameProbe() : undefined;
if (probe) (globalThis as { __bottleLatency?: FrameProbe }).__bottleLatency = probe;
mount(
  mogId ? (
    <MogView mogId={mogId} probe={probe} levels={viewMicLevels()} />
  ) : (
    <p role="alert">Unknown MOG window</p>
  ),
);
