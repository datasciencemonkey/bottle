/**
 * The registry of the hidden TS brain components T1-T15 of 4.3 (ARC-2, MOG-08b).
 * `startBrain()` constructs each component once; all domain state lives in these objects.
 */
import { createAgentBridge } from './agent-bridge';
import { createCapturePolicy } from './capture-policy';
import { createCleanupProvider } from './cleanup-provider';
import { createConsentService } from './consent-service';
import { createConversationManager } from './conversation-manager';
import { createGenieBridge } from './genie-bridge';
import { createMemoryService } from './memory-service';
import { createMogManager } from './mog-manager';
import { createMoodMapper } from './mood-mapper';
import { createOperationTracker } from './operation-tracker';
import { createPageService } from './page-service';
import { createProfileService } from './profile-service';
import { createRecoveryManager } from './recovery-manager';
import { createRouterContextGate } from './router-context-gate';
import { createSttProvider } from './stt-provider';

export interface BrainComponentEntry {
  readonly id: `T${number}`;
  readonly name: string;
  readonly factory: () => unknown;
}

/** T1..T15 in 4.3 table order. */
export const BRAIN_COMPONENTS = [
  { id: 'T1', name: 'MogManager', factory: () => createMogManager() },
  { id: 'T2', name: 'OperationTracker', factory: () => createOperationTracker() },
  { id: 'T3', name: 'CapturePolicy', factory: () => createCapturePolicy() },
  { id: 'T4', name: 'SttProvider', factory: () => createSttProvider() },
  { id: 'T5', name: 'CleanupProvider', factory: () => createCleanupProvider() },
  { id: 'T6', name: 'RouterContextGate', factory: () => createRouterContextGate() },
  { id: 'T7', name: 'ProfileService', factory: () => createProfileService() },
  { id: 'T8', name: 'GenieBridge', factory: () => createGenieBridge() },
  { id: 'T9', name: 'ConversationManager', factory: () => createConversationManager() },
  { id: 'T10', name: 'MemoryService', factory: () => createMemoryService() },
  { id: 'T11', name: 'MoodMapper', factory: () => createMoodMapper() },
  { id: 'T12', name: 'ConsentService', factory: () => createConsentService() },
  { id: 'T13', name: 'RecoveryManager', factory: () => createRecoveryManager() },
  { id: 'T14', name: 'PageService', factory: () => createPageService() },
  { id: 'T15', name: 'AgentBridge', factory: () => createAgentBridge() },
] as const satisfies readonly BrainComponentEntry[];

type Entry = (typeof BRAIN_COMPONENTS)[number];

/** The constructed components, keyed by 4.3 ID. */
export type BrainComponents = { readonly [E in Entry as E['id']]: ReturnType<E['factory']> };

let brain: BrainComponents | undefined;

/** Construct T1..T15 once. Later calls return the same objects. */
export function startBrain(): BrainComponents {
  brain ??= Object.fromEntries(BRAIN_COMPONENTS.map((c) => [c.id, c.factory()])) as BrainComponents;
  return brain;
}
