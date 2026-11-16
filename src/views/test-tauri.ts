/**
 * Test double for `@tauri-apps/api` in view tests. View tests mock the Tauri modules with
 * `vi.mock` and use these handles to push brain events and read `invoke` calls.
 */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import type { ReactNode } from 'react';

type Handler = (e: { payload: unknown }) => void;

export const tauri = {
  handlers: new Map<string, Set<Handler>>(),
  /** The `options` of each `listen` call, by event name. */
  listenOptions: new Map<string, unknown[]>(),
  invoke: vi.fn(async (_cmd: string, _args?: unknown) => ({})),
  label: 'hub',
  reset(): void {
    this.handlers.clear();
    this.listenOptions.clear();
    this.invoke.mockClear();
  },
  /** Deliver a brain-to-view event to every listener of `name`. */
  async emit(name: string, payload: unknown): Promise<void> {
    await act(async () => {
      for (const h of this.handlers.get(name) ?? []) h({ payload });
    });
  },
  listeners(name: string): number {
    return this.handlers.get(name)?.size ?? 0;
  },
};

export const eventModule = {
  listen: async (name: string, h: Handler, options?: unknown) => {
    tauri.listenOptions.set(name, [...(tauri.listenOptions.get(name) ?? []), options]);
    const set = tauri.handlers.get(name) ?? new Set<Handler>();
    set.add(h);
    tauri.handlers.set(name, set);
    return () => set.delete(h);
  },
};

export const coreModule = {
  invoke: (cmd: string, args?: unknown) => tauri.invoke(cmd, args),
  Channel: class {
    readonly id = 0;
  },
};

export const windowModule = {
  getCurrentWindow: () => ({ label: tauri.label }),
};

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** Render `node` into a fresh container and flush effects. */
export async function render(node: ReactNode): Promise<{ el: HTMLElement; root: Root }> {
  const el = document.createElement('div');
  document.body.append(el);
  const root = createRoot(el);
  await act(async () => root.render(node));
  return { el, root };
}

/** The `view_emit` calls sent so far, as their `args`. */
export function viewEmits(): unknown[] {
  return tauri.invoke.mock.calls
    .filter(([cmd]) => cmd === 'view_emit')
    .map(([, a]) => (a as { args: unknown }).args);
}
