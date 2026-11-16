/**
 * T1 MogManager (4.3, hidden TS brain window).
 *
 * Responsibilities: MOG definitions, start and stop, running cap, persona, per-MOG config
 * (no secrets), import and export.
 */
import type { MogBinding, MogId } from '../../shared/interfaces';
import { NotImplementedError } from '../errors';
import type { MogPagesConfig } from '../pages/home-domain';

/**
 * One MOG definition. It holds no secret. The saved config is `MogConfig` (`./mog-config`, #25);
 * TODO(#26): MogManager loads it with `config_list` and `config_read`.
 */
export interface MogDefinition {
  mogId: MogId;
  name: string;
  binding: MogBinding;
  /** `mog.pages`: the home domain for each workspace (PG-18, `PageDomains.config`). */
  pages?: MogPagesConfig;
}

export interface MogManager {
  list(): Promise<MogDefinition[]>;
  start(mogId: MogId): Promise<void>;
  stop(mogId: MogId): Promise<void>;
}

const C = 'MogManager';

export class MogManagerStub implements MogManager {
  async list(): Promise<MogDefinition[]> {
    throw new NotImplementedError(C, 'list', 26); // TODO(#26)
  }
  async start(_mogId: MogId): Promise<void> {
    throw new NotImplementedError(C, 'start', 26); // TODO(#26)
  }
  async stop(_mogId: MogId): Promise<void> {
    throw new NotImplementedError(C, 'stop', 26); // TODO(#26)
  }
}

export function createMogManager(): MogManager {
  return new MogManagerStub();
}
