/**
 * Entry point of the hidden brain window (ARC-2). All domain logic lives here.
 * Shared queues run on the one brain actor (`getBrainActor`). Core-to-brain items arrive on
 * the private `brain_subscribe` Channel (ADR 0003).
 */
import { startBrain } from './components';

startBrain();
