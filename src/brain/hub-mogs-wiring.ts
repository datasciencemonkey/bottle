/** Red stub (#26). */
import type { HubMogList } from '../shared/hub-mogs';
import type { BrainBus } from './bus';
import type { MogManager } from './components/mog-manager';
export function sendHubMogs(_l: HubMogList): Promise<void> { throw new Error('todo #26'); }
export function wireHubMogs(_b: BrainBus, _m: MogManager, _s?: (l: HubMogList) => Promise<unknown>): () => void { throw new Error('todo #26'); }
