// @vitest-environment jsdom
/**
 * #124: the hub Settings → Wipe panel and its dialog (thin; MEM-33a, SEC-15c, MB-15, G-45b, U9,
 * ARC-2, ARC-5; ADR 0087). The brain builds every model. Every value here is fake.
 */
import { act } from 'react';
import type { WipeDialogModel, WipeSettingsModel } from '../../shared/wipe-view';
import { render, tauri, viewEmits } from '../test-tauri';
import { WipeDialog } from '../WipeDialog';
import { Hub } from './Hub';
import { WipeSettings } from './WipeSettings';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const MOG = '00000000-0000-4000-8000-000000000001';
const CID = '01f0aaaa-test-not-real-0001';

const confirm: WipeDialogModel = {
  scope: 'older_than_90d',
  alsoGenie: false,
  phase: 'confirm',
  title: 'Wipe data older than 90 days',
  text: 'Delete the data older than 90 days on this computer? You cannot undo this.',
};

const done: WipeDialogModel = {
  ...confirm,
  alsoGenie: true,
  phase: 'done',
  result: {
    local: 12,
    deletedConversations: 1,
    notices: [
      {
        mogId: MOG,
        label: 'Delete in Genie One',
        text: 'Bottle cannot delete Genie One conversations yet.',
        links: [
          {
            conversationId: CID,
            url: `https://acme.cloud.databricks.com/one/chat/threads/${CID}?o=1001`,
          },
        ],
        noLink: 0,
      },
    ],
    otherText: 'Bottle could not delete 1 Genie conversation. Delete it in Genie.',
  },
};

const settings: WipeSettingsModel = {
  rows: [
    { id: 'transcripts', label: 'Transcripts', value: 'Kept until you wipe', locked: false },
    { id: 'audio_journal', label: 'Session audio journal', value: '7 days', locked: true },
  ],
};

function must<T>(v: T | null | undefined): T {
  if (v === null || v === undefined) throw new Error('missing element');
  return v;
}

const radios = (el: HTMLElement) => [
  ...el.querySelectorAll<HTMLInputElement>('input[type="radio"][name="wipeScope"]'),
];

beforeEach(() => tauri.reset());

describe('Settings → Wipe (#124)', () => {
  it('[MEM-33a] offers older than 30 d, older than 90 d and everything, with none chosen', async () => {
    const { el, root } = await render(<WipeSettings />);
    const r = radios(el);
    expect(r.map((x) => x.value)).toEqual(['older_than_30d', 'older_than_90d', 'everything']);
    expect(r.map((x) => x.labels?.[0]?.textContent)).toEqual([
      'Older than 30 days',
      'Older than 90 days',
      'Everything',
    ]);
    expect(r.some((x) => x.checked)).toBe(false);
    const genie = must(
      el.querySelector<HTMLInputElement>('input[type="checkbox"][name="alsoGenie"]'),
    );
    expect(genie.checked).toBe(false);
    expect(genie.labels?.[0]?.textContent).toBe('Also delete Genie conversations');
    // U9: the Wipe button is off until the user picks an option.
    expect(must(el.querySelector<HTMLButtonElement>('button[data-wipe-open]')).disabled).toBe(true);
    act(() => root.unmount());
  });

  it('[MEM-33a] [U9] the Wipe button sends the chosen option and asks the brain for a confirm; it deletes nothing', async () => {
    const { el, root } = await render(<WipeSettings />);
    await act(async () => {
      must(radios(el)[2]).click();
      must(el.querySelector<HTMLInputElement>('input[name="alsoGenie"]')).click();
    });
    await act(async () => {
      must(el.querySelector<HTMLButtonElement>('button[data-wipe-open]')).click();
    });
    expect(viewEmits()).toContainEqual({
      mogId: 'wipe',
      kind: 'click',
      payload: { target: 'wipe_open', scope: 'everything', alsoGenie: true },
    });
    // The panel calls only view_emit (ARC-2): no wipe command, no confirm without the dialog.
    const cmds = tauri.invoke.mock.calls.map((c) => c[0]);
    expect(new Set(cmds)).toEqual(new Set(['view_emit']));
    expect(JSON.stringify(viewEmits())).not.toContain('wipe_confirm');
    act(() => root.unmount());
  });

  it('[SEC-15c] [MB-15] [ARC-5] the retention rows show the admin lock with "Set by your admin"', async () => {
    const { el, root } = await render(<WipeSettings />);
    // The panel sends nothing on mount; the user asks for the rows.
    expect(viewEmits()).toEqual([]);
    await act(async () => {
      must(el.querySelector<HTMLButtonElement>('button[data-wipe-settings]')).click();
    });
    expect(viewEmits()).toEqual([
      { mogId: 'wipe', kind: 'click', payload: { target: 'wipe_settings' } },
    ]);
    expect(tauri.listenOptions.get('hub:wipe_settings')).toEqual([{ target: 'hub' }]);
    await tauri.emitTo('hub', 'hub:wipe_settings', settings);
    const rows = [...el.querySelectorAll('[data-retention-row]')];
    expect(rows.map((r) => r.getAttribute('data-retention-row'))).toEqual([
      'transcripts',
      'audio_journal',
    ]);
    expect(rows[0]?.textContent).toContain('Kept until you wipe');
    expect(rows[0]?.querySelector('[data-admin-lock]')).toBeNull();
    const lock = must(rows[1]?.querySelector('[data-admin-lock]'));
    expect(lock.getAttribute('role')).toBe('img');
    expect(lock.getAttribute('aria-label')).toBe('Locked');
    expect(rows[1]?.textContent).toContain('7 days');
    expect(rows[1]?.textContent).toContain('Set by your admin');
    await tauri.emitTo('hub', 'hub:wipe_settings', { rows: 'bad' });
    expect(el.querySelectorAll('[data-retention-row]')).toHaveLength(2);
    act(() => root.unmount());
  });

  it('[MEM-33a] [ARC-5] hosts the dialog sent to the hub only, and closes on null', async () => {
    const { el, root } = await render(<WipeSettings />);
    expect(tauri.listenOptions.get('hub:wipe_dialog')).toEqual([{ target: 'hub' }]);
    await tauri.emitTo(`mog-${MOG}`, 'hub:wipe_dialog', confirm);
    expect(el.querySelector('[data-wipe-dialog]')).toBeNull();
    await tauri.emitTo('hub', 'hub:wipe_dialog', confirm);
    expect(el.querySelector('[data-wipe-dialog]')?.textContent).toContain(confirm.title);
    await tauri.emitTo('hub', 'hub:wipe_dialog', null);
    expect(el.querySelector('[data-wipe-dialog]')).toBeNull();
    act(() => root.unmount());
  });

  it('[MEM-33a] the confirm step sends wipe_confirm with the option only', async () => {
    const { el, root } = await render(<WipeDialog model={confirm} />);
    const box = must(el.querySelector('[data-wipe-dialog]'));
    expect(box.getAttribute('role')).toBe('dialog');
    expect(box.textContent).toContain(confirm.text);
    await act(async () => {
      must(el.querySelector<HTMLButtonElement>('button[data-wipe-confirm]')).click();
    });
    expect(viewEmits()).toContainEqual({
      mogId: 'wipe',
      kind: 'click',
      payload: { target: 'wipe_confirm', scope: 'older_than_90d' },
    });
    const cancel = must(el.querySelector<HTMLButtonElement>('button[data-wipe-cancel]'));
    expect(cancel.textContent).toBe('Cancel');
    await act(async () => {
      cancel.click();
    });
    expect(viewEmits()).toContainEqual({
      mogId: 'wipe',
      kind: 'click',
      payload: { target: 'wipe_cancel' },
    });
    act(() => root.unmount());
  });

  it('[G-45b] [AT-16] the done step shows the counts and the Delete in Genie One notice', async () => {
    const { el, root } = await render(<WipeDialog model={done} />);
    expect(el.querySelector('button[data-wipe-confirm]')).toBeNull();
    expect(must(el.querySelector('[data-wipe-result]')).textContent).toBe(
      'Bottle deleted 12 rows on this computer and 1 Genie conversation.',
    );
    expect(
      must(el.querySelector<HTMLButtonElement>('button[data-genie-one-delete-link]')).textContent,
    ).toBe('Delete in Genie One');
    expect(must(el.querySelector('[data-wipe-other]')).textContent).toBe(done.result?.otherText);
    expect(must(el.querySelector('button[data-wipe-cancel]')).textContent).toBe('Close');
    act(() => root.unmount());
  });

  it('[MEM-33a] the hub shows the Wipe panel after the notice is accepted', async () => {
    const { el, root } = await render(<Hub />);
    expect(el.querySelector('[data-view="wipe-settings"]')).not.toBeNull();
    act(() => root.unmount());
  });
});
