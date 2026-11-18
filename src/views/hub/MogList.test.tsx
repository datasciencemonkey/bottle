// @vitest-environment jsdom
import { act } from 'react';
import { accessibleText } from '../../test-support/a11y';
import { render, tauri, viewEmits } from '../test-tauri';
import { MogList } from './MogList';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const A = '00000000-0000-4000-8000-00000000000a';
const B = '00000000-0000-4000-8000-00000000000b';
const list = {
  mogs: [
    { mogId: A, name: 'Sales', home: 'sales-ws', persona: '', running: false },
    { mogId: B, name: 'Ops', home: 'ops-ws', persona: 'Short.', running: true },
  ],
  unreadable: [],
  runningCount: 1,
  maxRunning: 8,
};

const byLabel = (el: HTMLElement, label: string) => {
  const hit = [...el.querySelectorAll('button, input, textarea')].find(
    (b) => accessibleText(b) === label || labelText(b) === label,
  );
  if (!hit) throw new Error(`no control "${label}"`);
  return hit as HTMLElement;
};

function labelText(c: Element): string {
  const l = c.id ? c.ownerDocument.querySelector(`label[for="${c.id}"]`) : null;
  return (l?.textContent ?? '').trim();
}

async function click(c: HTMLElement): Promise<void> {
  await act(async () => c.click());
}

async function type(c: HTMLElement, value: string): Promise<void> {
  const proto = c instanceof HTMLTextAreaElement ? HTMLTextAreaElement : HTMLInputElement;
  await act(async () => {
    Object.getOwnPropertyDescriptor(proto.prototype, 'value')?.set?.call(c, value);
    c.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

async function key(c: HTMLElement, k: string): Promise<void> {
  await act(async () => {
    c.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));
  });
}

const actions = () => viewEmits().map((e) => (e as { payload?: unknown }).payload);

async function shown(): Promise<HTMLElement> {
  const { el } = await render(<MogList />);
  await tauri.emitTo('hub', 'hub:mogs', list);
  return el;
}

describe('hub MOG list (MB-01, MB-03b)', () => {
  beforeEach(() => tauri.reset());

  it('[MB-01] [ARC-5] listens on hub:mogs for its own window, then asks with mogs.ready', async () => {
    await render(<MogList />);
    expect(tauri.listeners('hub:mogs')).toBe(1);
    expect(tauri.listenOptions.get('hub:mogs')).toEqual([{ target: 'hub' }]);
    expect(viewEmits()).toEqual([
      { mogId: 'hub', kind: 'click', payload: { action: 'mogs.ready' } },
    ]);
  });

  it('[MB-01] lists each MOG with its state and the running count', async () => {
    const el = await shown();
    const rows = [...el.querySelectorAll('li[data-mog-row]')].map((li) => [
      li.getAttribute('data-mog-row'),
      li.getAttribute('data-running'),
    ]);
    expect(rows).toEqual([
      [A, 'false'],
      [B, 'true'],
    ]);
    expect(el.textContent).toContain('1 of 8 MOGs are running.');
    expect(byLabel(el, 'Start Sales')).toBeTruthy();
    expect(byLabel(el, 'Stop Ops')).toBeTruthy();
  });

  it('[MB-01] start, stop and duplicate send one hub action each', async () => {
    const el = await shown();
    await click(byLabel(el, 'Start Sales'));
    await click(byLabel(el, 'Stop Ops'));
    await click(byLabel(el, 'Duplicate Sales'));
    expect(actions().slice(1)).toEqual([
      { action: 'mog.start', mogId: A },
      { action: 'mog.stop', mogId: B },
      { action: 'mog.duplicate', mogId: A },
    ]);
  });

  it('[MB-01] the create form sends the name and the home profile', async () => {
    const el = await shown();
    await type(byLabel(el, 'Name'), 'Finance');
    await type(byLabel(el, 'Home profile'), 'fin-ws');
    await click(byLabel(el, 'Create MOG'));
    expect(actions().at(-1)).toEqual({ action: 'mog.create', name: 'Finance', home: 'fin-ws' });
  });

  it('[MB-01] edit opens a labelled form with the values, Save sends mog.edit, Esc cancels', async () => {
    const el = await shown();
    await click(byLabel(el, 'Edit Ops'));
    const name = byLabel(el, 'Name of Ops') as HTMLInputElement;
    expect(name.value).toBe('Ops');
    expect((byLabel(el, 'Persona instructions') as HTMLTextAreaElement).value).toBe('Short.');
    await type(name, 'Ops EU');
    await click(byLabel(el, 'Save'));
    expect(actions().at(-1)).toEqual({
      action: 'mog.edit',
      mogId: B,
      name: 'Ops EU',
      home: 'ops-ws',
      persona: 'Short.',
    });
    await click(byLabel(el, 'Edit Sales'));
    const n = viewEmits().length;
    await key(byLabel(el, 'Name of Sales'), 'Escape');
    expect(el.querySelector('form[data-edit]')).toBeNull();
    expect(viewEmits()).toHaveLength(n);
  });

  it('[MB-01] delete asks first; Esc and Cancel send nothing; Delete sends confirmed', async () => {
    const el = await shown();
    await click(byLabel(el, 'Delete Sales'));
    const dialog = el.querySelector('[role="alertdialog"]') as HTMLElement;
    expect(dialog).not.toBeNull();
    expect(accessibleText(dialog)).toBe('Delete Sales?');
    expect(dialog.textContent).toContain(
      'Bottle removes the settings of this MOG. Its memory and history stay until you use Wipe in Settings.',
    );
    expect(document.activeElement?.textContent).toBe('Cancel');
    await key(dialog, 'Escape');
    expect(el.querySelector('[role="alertdialog"]')).toBeNull();
    await click(byLabel(el, 'Delete Sales'));
    await click(byLabel(el, 'Cancel'));
    expect(actions().filter((a) => (a as { action: string }).action === 'mog.delete')).toEqual([]);
    await click(byLabel(el, 'Delete Sales'));
    await click(byLabel(el, 'Delete'));
    expect(actions().at(-1)).toEqual({ action: 'mog.delete', mogId: A, confirmed: true });
  });

  it('[MB-03b] shows the brain notice in an alert region', async () => {
    const el = await shown();
    const text = '8 MOGs are running. Stop one to start Sales.';
    await tauri.emitTo('hub', 'hub:mogs', { ...list, notice: { kind: 'error', text } });
    const alert = el.querySelector('[role="alert"]');
    expect(alert?.textContent).toBe(text);
    await tauri.emitTo('hub', 'hub:mogs', list);
    expect(el.querySelector('[role="alert"]')?.textContent ?? '').toBe('');
  });

  it('[MB-03a] says how many configs cannot be read, and ignores a bad payload', async () => {
    const el = await shown();
    await tauri.emitTo('hub', 'hub:mogs', { ...list, unreadable: ['x', 'y'] });
    expect(el.textContent).toContain('2 MOG configs cannot be read. Bottle keeps the files.');
    await tauri.emitTo('hub', 'hub:mogs', { mogs: 'no' });
    expect(el.querySelectorAll('li[data-mog-row]')).toHaveLength(2);
  });

  it('[MB-01] focus goes back to the Edit button after Esc or Save (WCAG 2.4.3)', async () => {
    const el = await shown();
    await click(byLabel(el, 'Edit Ops'));
    await key(byLabel(el, 'Name of Ops'), 'Escape');
    expect(document.activeElement).toBe(byLabel(el, 'Edit Ops'));
    await click(byLabel(el, 'Edit Ops'));
    await click(byLabel(el, 'Save'));
    expect(document.activeElement).toBe(byLabel(el, 'Edit Ops'));
  });

  it('[MB-01] focus goes back to the Delete button after Esc or Cancel, and to the heading after a delete', async () => {
    const el = await shown();
    await click(byLabel(el, 'Delete Sales'));
    await key(el.querySelector('[role="alertdialog"]') as HTMLElement, 'Escape');
    expect(document.activeElement).toBe(byLabel(el, 'Delete Sales'));
    await click(byLabel(el, 'Delete Sales'));
    await click(byLabel(el, 'Cancel'));
    expect(document.activeElement).toBe(byLabel(el, 'Delete Sales'));
    await click(byLabel(el, 'Delete Sales'));
    await click(byLabel(el, 'Delete'));
    expect(document.activeElement).toBe(el.querySelector('h2'));
  });

  it('[MB-01] Tab and Shift+Tab stay inside the delete dialog', async () => {
    const el = await shown();
    await click(byLabel(el, 'Delete Sales'));
    const cancel = document.activeElement as HTMLElement;
    expect(cancel.textContent).toBe('Cancel');
    await key(cancel, 'Tab');
    expect(document.activeElement?.textContent).toBe('Delete');
    await act(async () => {
      document.activeElement?.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true }),
      );
    });
    expect(document.activeElement).toBe(cancel);
  });

  it('[MB-01] each row shows Start or Stop and Edit; Duplicate, Forget and Delete sit behind one More entry point (#561)', async () => {
    const el = await shown();
    const row = el.querySelector(`li[data-mog-row="${A}"]`) as HTMLElement;
    const more = row.querySelector('details') as HTMLDetailsElement;
    expect(more.querySelector('summary')?.getAttribute('aria-label')).toBe(
      'More actions for Sales',
    );
    expect(more.open).toBe(false);
    const inMore = [...more.querySelectorAll('button')].map((b) => b.getAttribute('aria-label'));
    expect(inMore).toEqual(['Duplicate Sales', 'Forget the memory of Sales', 'Delete Sales']);
    // The frequent actions stay visible in the row, outside the menu.
    for (const label of ['Start Sales', 'Edit Sales'])
      expect(more.contains(byLabel(row, label))).toBe(false);
  });

  it('[MB-03b] the same error twice gives a new alert, so a screen reader reads it again', async () => {
    const el = await shown();
    const text = '8 MOGs are running. Stop one to start Sales.';
    await tauri.emitTo('hub', 'hub:mogs', { ...list, notice: { kind: 'error', text } });
    const first = el.querySelector('[role="alert"]');
    await tauri.emitTo('hub', 'hub:mogs', { ...list, notice: { kind: 'error', text } });
    const second = el.querySelector('[role="alert"]');
    expect(second?.textContent).toBe(text);
    expect(second).not.toBe(first);
  });
});
