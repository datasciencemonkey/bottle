/**
 * T1 MogManager (4.3, hidden TS brain window).
 *
 * Responsibilities: MOG definitions, start and stop, running cap, persona, per-MOG config
 * (no secrets), import and export.
 *
 * #26 (MB-01, MB-03a, MB-03b): create, edit, duplicate, start, stop and delete. The configs are
 * the local JSON files of #25 (ADR 0035), through `config_*`. Start and stop open and close the
 * `mog-<id>` window through `window_mog_open` / `window_mog_close` (R11, ADR 0008). The brain
 * keeps the running cap (`app.maxRunningMogs`, default 8); the Rust `WindowLedger` keeps the same
 * cap of 8, and its refusal gives the same text.
 *
 * Bottle deletes a config only when the user confirms a delete in the hub. Memory and history of
 * the MOG stay until Settings > Wipe (no automatic deletion of user content).
 *
 * Import and export are MB-13 (v1).
 */
import type { JsonObject } from '../../shared/ipc';
import { call } from '../../shared/ipc';
import type { MogBinding, MogId, ProfileName } from '../../shared/interfaces';
import { BrainActor } from '../actor';
import type { MogPagesConfig } from '../pages/home-domain';
import { MOG_CONFIG_DEFAULTS, type MogConfig } from './mog-config';
import { MOG_PALETTE, nextVariant } from '../../shared/mog-palette';

/** Appendix F `app.maxRunningMogs` default (D44, MB-03b). */
export const MAX_RUNNING_MOGS_DEFAULT = 8;
/** The cap of the Rust host (`windows::MAX_ACTIVE_MOGS`, ADR 0117: one MOG). The brain cap never passes it. */
export const HOST_MAX_RUNNING_MOGS = 1;

const NAME_MAX = 64;
const PERSONA_MAX = 2000;
const PROFILE_MAX = 256;
const COPY_SUFFIX = ' copy';

/**
 * The run state of one MOG. #98 (MoodMapper lifecycle) and #50 (badges, ADR 0029) read it with
 * `runState` and `onChange`. A MOG is `running` while its `mog-<id>` window is open.
 */
export type MogRunState = 'stopped' | 'running';

/** One MOG definition. It holds no secret. The saved config is `MogConfig` (#25). */
export interface MogDefinition {
  mogId: MogId;
  name: string;
  binding: MogBinding;
  /** `mog.pages`: the home domain for each workspace (PG-18, `PageDomains.config`). */
  pages?: MogPagesConfig;
}

/** One MOG as the hub lists it. */
export interface MogSummary extends MogDefinition {
  persona: string;
  runState: MogRunState;
  /** #ui-4: `mog.ui.variant` (a palette id, or `auto` for a legacy config). */
  colorVariant: string;
}

export interface MogListing {
  /** Sorted by name, then ID. */
  mogs: MogSummary[];
  /** Config IDs that the core cannot read. Bottle keeps these files. */
  unreadable: string[];
}

/** The content-free reasons of a refused action. The message is the hub text. */
export type MogErrorCode =
  | 'invalid_name'
  | 'invalid_home'
  | 'invalid_persona'
  | 'invalid_color'
  | 'not_found'
  | 'not_confirmed'
  | 'cap_reached'
  | 'secret_rejected'
  | 'limit_reached'
  | 'window_closing'
  | 'list_failed'
  | 'save_failed'
  | 'start_failed'
  | 'stop_failed'
  | 'delete_failed';

/** A refused MOG action. `message` is fixed text for the hub; it never holds a config value. */
export class MogManagerError extends Error {
  override readonly name = 'MogManagerError';
  readonly code: MogErrorCode;

  constructor(code: MogErrorCode, message: string) {
    super(message);
    this.code = code;
  }
}

/** The text of MB-03b. Requirement wording for 8: "8 MOGs are running. Stop one to start <name>." */
export function capText(cap: number, name: string): string {
  return cap === 1
    ? `1 MOG is running. Stop it to start ${name}.`
    : `${cap} MOGs are running. Stop one to start ${name}.`;
}

/** The `config_*` commands (#25). The host routes them to `MogConfigComponent` (#379). */
export interface MogConfigClient {
  list(): Promise<{ items: { id: string; name: string }[]; unreadable: string[] }>;
  read(id: string): Promise<MogConfig>;
  /** Returns the saved config with the defaults filled in. */
  write(id: string, config: MogConfig): Promise<MogConfig>;
  /** False if no such config was saved. */
  remove(id: string): Promise<boolean>;
}

/** The `window_mog_*` commands. The host routes them to R11 `WindowHost` (#285, ADR 0040). */
export interface MogWindowClient {
  openMog(mogId: MogId): Promise<void>;
  closeMog(mogId: MogId): Promise<void>;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const isStr = (v: unknown): v is string => typeof v === 'string';
const badReply = () => Promise.reject({ error: 'bad_reply' });

function configReply(r: unknown): Promise<MogConfig> {
  return isObj(r) && isObj(r['json'])
    ? Promise.resolve(r['json'] as unknown as MogConfig)
    : badReply();
}

/** `MogConfigClient` over `call` (ADR 0035 decision 9 reply shapes). */
export function ipcMogConfigClient(): MogConfigClient {
  return {
    async list() {
      const r: unknown = await call('config_list', {});
      if (
        !isObj(r) ||
        !Array.isArray(r['items']) ||
        !r['items'].every((i) => isObj(i) && isStr(i['id']) && isStr(i['name'])) ||
        !Array.isArray(r['unreadable']) ||
        !r['unreadable'].every(isStr)
      )
        return badReply();
      return r as { items: { id: string; name: string }[]; unreadable: string[] };
    },
    async read(id) {
      return configReply(await call('config_read', { id }));
    },
    async write(id, config) {
      return configReply(await call('config_write', { id, json: config as unknown as JsonObject }));
    },
    async remove(id) {
      const r: unknown = await call('config_delete', { id });
      return isObj(r) && typeof r['deleted'] === 'boolean' ? r['deleted'] : badReply();
    },
  };
}

/** `MogWindowClient` over `call`. */
export function ipcMogWindowClient(): MogWindowClient {
  return {
    async openMog(mogId) {
      await call('window_mog_open', { mogId });
    },
    async closeMog(mogId) {
      await call('window_mog_close', { mogId });
    },
  };
}

/**
 * The code of a gateway or component refusal (`{ error }` or `{ reason }`), else ''.
 * Since #379 every config code (`config_not_found`, `secret_named_key`, `token_like_value`,
 * `config_limit_reached` and the G-03 backend codes) crosses the gateway as `{ error: <code> }`.
 */
function codeOf(e: unknown): string {
  if (!isObj(e)) return '';
  if (isStr(e['error'])) return e['error'];
  if (isStr(e['reason'])) return e['reason'];
  return '';
}

const chars = (s: string) => [...s].length;
// eslint-disable-next-line no-control-regex
const hasControl = (s: string) => /[\u0000-\u001f\u007f-\u009f]/.test(s);

export interface MogManager {
  /** Every defined MOG (MB-03a). */
  list(): Promise<MogListing>;
  /** A new MOG with the Appendix F defaults. No name gives the lowest free "MOG <n>". */
  create(input: { name?: string; home: ProfileName }): Promise<MogSummary>;
  edit(
    mogId: MogId,
    patch: { name?: string; home?: ProfileName; persona?: string; colorVariant?: string },
  ): Promise<MogSummary>;
  /** A copy under a new ID, named "<name> copy", without the per-MOG chord (MB-14). */
  duplicate(mogId: MogId): Promise<MogSummary>;
  /** Opens `mog-<id>`. Refused over the cap with the MB-03b text. */
  start(mogId: MogId): Promise<void>;
  stop(mogId: MogId): Promise<void>;
  /** Deletes the config after the user confirms. Stops a running MOG first. */
  remove(mogId: MogId, opts: { confirmed: true }): Promise<void>;
  runState(mogId: MogId): MogRunState;
  runningCount(): number;
  readonly maxRunning: number;
  /** Called after each change of a config or a run state. Returns the unsubscribe. */
  onChange(listener: () => void): () => void;
}

export interface MogManagerOptions {
  configs?: MogConfigClient;
  windows?: MogWindowClient;
  /** `app.maxRunningMogs`, 1 to 8. Any other value gives 8. TODO(#77): read it from settings. */
  maxRunningMogs?: number;
  newId?: () => MogId;
}

const QUEUE = 'mogs';

class Manager implements MogManager {
  readonly maxRunning: number;
  readonly #configs: MogConfigClient;
  readonly #windows: MogWindowClient;
  readonly #newId: () => MogId;
  /** Brain memory only. TODO(#419): sync with the R11 window events. */
  readonly #running = new Set<MogId>();
  readonly #listeners = new Set<() => void>();
  readonly #actor = new BrainActor();

  constructor(o: MogManagerOptions) {
    this.#configs = o.configs ?? ipcMogConfigClient();
    this.#windows = o.windows ?? ipcMogWindowClient();
    this.#newId = o.newId ?? (() => crypto.randomUUID());
    const cap = o.maxRunningMogs;
    this.maxRunning =
      cap !== undefined && Number.isInteger(cap) && cap >= 1 && cap <= HOST_MAX_RUNNING_MOGS
        ? cap
        : MAX_RUNNING_MOGS_DEFAULT;
  }

  /** One action at a time, so two starts never pass the cap together. */
  #run<T>(task: () => Promise<T>): Promise<T> {
    return this.#actor.enqueue(QUEUE, task);
  }

  #changed(): void {
    for (const l of [...this.#listeners]) l();
  }

  onChange(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  runState(mogId: MogId): MogRunState {
    return this.#running.has(mogId) ? 'running' : 'stopped';
  }

  runningCount(): number {
    return this.#running.size;
  }

  #summary(mogId: MogId, c: MogConfig): MogSummary {
    return {
      mogId,
      name: c.name,
      binding: {
        home: c.binding.home,
        backend: c.binding.backend,
        genieCompanion: c.binding.genieCompanion,
      },
      persona: c.persona,
      runState: this.runState(mogId),
      // Defensive: a config a test double or an old file did not fill in (the real core always
      // does) still lists, with the "auto" fallback `paletteColorOf` resolves by MOG ID.
      colorVariant: c.ui?.variant ?? MOG_CONFIG_DEFAULTS.ui.variant,
    };
  }

  async #read(mogId: MogId): Promise<MogConfig> {
    try {
      return await this.#configs.read(mogId);
    } catch (e) {
      throw codeOf(e) === 'config_not_found'
        ? new MogManagerError('not_found', 'This MOG is not in the list any more.')
        : new MogManagerError('save_failed', 'Bottle cannot read this MOG. Try again.');
    }
  }

  async #write(mogId: MogId, c: MogConfig): Promise<MogSummary> {
    let saved: MogConfig;
    try {
      saved = await this.#configs.write(mogId, c);
    } catch (e) {
      const code = codeOf(e);
      if (code === 'secret_named_key' || code === 'token_like_value')
        throw new MogManagerError(
          'secret_rejected',
          'A MOG config cannot hold a token, password, key or secret.',
        );
      if (code === 'config_limit_reached')
        throw new MogManagerError(
          'limit_reached',
          'Bottle cannot keep more MOGs. Delete one to add a new MOG.',
        );
      throw new MogManagerError('save_failed', 'Bottle could not save the MOG. Try again.');
    }
    this.#changed();
    return this.#summary(mogId, saved);
  }

  async #listNow(): Promise<MogListing> {
    let raw: Awaited<ReturnType<MogConfigClient['list']>>;
    try {
      raw = await this.#configs.list();
    } catch {
      throw new MogManagerError('list_failed', 'Bottle cannot read the MOG list.');
    }
    const mogs: MogSummary[] = [];
    const unreadable = [...raw.unreadable];
    for (const { id } of raw.items) {
      try {
        mogs.push(this.#summary(id, await this.#configs.read(id)));
      } catch {
        unreadable.push(id);
      }
    }
    mogs.sort((a, b) => a.name.localeCompare(b.name) || (a.mogId < b.mogId ? -1 : 1));
    return { mogs, unreadable };
  }

  list(): Promise<MogListing> {
    return this.#run(() => this.#listNow());
  }

  create(input: { name?: string; home: ProfileName }): Promise<MogSummary> {
    return this.#run(async () => {
      const home = checkHome(input.home);
      const existing = (await this.#listNow()).mogs;
      let name = (input.name ?? '').trim();
      if (!name) {
        const used = new Set(existing.map((m) => m.name));
        let n = 1;
        while (used.has(`MOG ${n}`)) n++;
        name = `MOG ${n}`;
      }
      // #ui-4 (D-identity): the first unused palette colour, or an even spread past it.
      const variant = nextVariant(
        existing.map((m) => m.colorVariant),
        existing.length,
      );
      const config: MogConfig = {
        ...structuredClone(MOG_CONFIG_DEFAULTS),
        name: checkName(name),
        ui: { ...MOG_CONFIG_DEFAULTS.ui, variant },
        binding: {
          home,
          backend: { kind: 'genie_one_mcp', profile: home },
          genieCompanion: true,
          profileAliases: {},
        },
      };
      return this.#write(this.#newId(), config);
    });
  }

  edit(
    mogId: MogId,
    patch: { name?: string; home?: ProfileName; persona?: string; colorVariant?: string },
  ): Promise<MogSummary> {
    return this.#run(async () => {
      const name = patch.name === undefined ? undefined : checkName(patch.name.trim());
      const home = patch.home === undefined ? undefined : checkHome(patch.home);
      const persona = patch.persona === undefined ? undefined : checkPersona(patch.persona);
      const colorVariant =
        patch.colorVariant === undefined ? undefined : checkColorVariant(patch.colorVariant);
      const c = await this.#read(mogId);
      if (name !== undefined) c.name = name;
      if (persona !== undefined) c.persona = persona;
      if (colorVariant !== undefined) c.ui = { ...c.ui, variant: colorVariant };
      if (home !== undefined && home !== c.binding.home) {
        // The backend follows the home profile when it used the old home (G-01).
        if (c.binding.backend.profile === c.binding.home)
          c.binding.backend = { ...c.binding.backend, profile: home };
        c.binding.home = home;
      }
      return this.#write(mogId, c);
    });
  }

  duplicate(mogId: MogId): Promise<MogSummary> {
    return this.#run(async () => {
      const c = await this.#read(mogId);
      const base = [...c.name].slice(0, NAME_MAX - COPY_SUFFIX.length).join('');
      return this.#write(this.#newId(), {
        ...c,
        name: `${base}${COPY_SUFFIX}`,
        hotkeys: {},
      });
    });
  }

  start(mogId: MogId): Promise<void> {
    return this.#run(async () => {
      if (this.#running.has(mogId)) return;
      const { name } = await this.#read(mogId);
      if (this.#running.size >= this.maxRunning)
        throw new MogManagerError('cap_reached', capText(this.maxRunning, name));
      try {
        await this.#windows.openMog(mogId);
      } catch (e) {
        const code = codeOf(e);
        if (code === 'mog_cap_reached')
          throw new MogManagerError('cap_reached', capText(HOST_MAX_RUNNING_MOGS, name));
        if (code === 'window_closing')
          throw new MogManagerError(
            'window_closing',
            `${name} is still closing. Try again in a moment.`,
          );
        throw new MogManagerError('start_failed', `Bottle could not start ${name}.`);
      }
      this.#running.add(mogId);
      this.#changed();
    });
  }

  #stopNow = async (mogId: MogId, name: () => Promise<string>): Promise<void> => {
    if (!this.#running.has(mogId)) return;
    try {
      await this.#windows.closeMog(mogId);
    } catch (e) {
      // `unknown_mog`: the window is gone already, so the MOG is stopped.
      if (codeOf(e) !== 'unknown_mog')
        throw new MogManagerError('stop_failed', `Bottle could not stop ${await name()}.`);
    }
    this.#running.delete(mogId);
    this.#changed();
  };

  stop(mogId: MogId): Promise<void> {
    return this.#run(() =>
      this.#stopNow(
        mogId,
        async () => (await this.#read(mogId).catch(() => null))?.name ?? 'the MOG',
      ),
    );
  }

  remove(mogId: MogId, opts: { confirmed: true }): Promise<void> {
    return this.#run(async () => {
      if (opts.confirmed !== true)
        throw new MogManagerError('not_confirmed', 'Confirm the delete first.');
      const { name } = await this.#read(mogId);
      await this.#stopNow(mogId, async () => name);
      try {
        await this.#configs.remove(mogId);
      } catch {
        throw new MogManagerError('delete_failed', `Bottle could not delete ${name}.`);
      }
      this.#changed();
    });
  }
}

function checkName(name: string): string {
  const n = chars(name);
  if (n < 1 || n > NAME_MAX || hasControl(name))
    throw new MogManagerError('invalid_name', 'A MOG name must have 1 to 64 characters.');
  return name;
}

function checkHome(home: string): ProfileName {
  const h = home.trim();
  if (chars(h) < 1 || chars(h) > PROFILE_MAX || hasControl(h))
    throw new MogManagerError('invalid_home', 'Choose a home profile for the MOG.');
  return h;
}

function checkPersona(persona: string): string {
  if (chars(persona) > PERSONA_MAX)
    throw new MogManagerError(
      'invalid_persona',
      'Persona instructions can have at most 2,000 characters.',
    );
  return persona;
}

/** #ui-4: the row colour picker in the hub offers only the fixed palette, never a raw hex. */
const PALETTE_IDS = new Set(MOG_PALETTE.map((c) => c.id));
function checkColorVariant(colorVariant: string): string {
  if (!PALETTE_IDS.has(colorVariant))
    throw new MogManagerError('invalid_color', 'Choose a colour from the MOG palette.');
  return colorVariant;
}

export function createMogManager(opts: MogManagerOptions = {}): MogManager {
  return new Manager(opts);
}
