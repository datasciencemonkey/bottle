/**
 * Entry point of the hidden brain window (ARC-2). All domain logic lives here.
 * Shared queues run on the one brain actor (`getBrainActor`).
 */
import { startBrain } from './components';

startBrain();
