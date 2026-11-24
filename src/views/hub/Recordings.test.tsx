// @vitest-environment jsdom
/**
 * The hub Recordings area (ADR 0103, refs #65): the list, newest first, and the detail with the
 * notes (safe markdown, no raw HTML), the speaker turns and "Open in Databricks". Fake values.
 */
import { act } from 'react';
import {
  HUB_RECORDING_DETAIL_EVENT,
  HUB_RECORDINGS_EVENT,
  RECORDING_ERRORS,
  RECORDING_TEXT,
  RECORDING_VIEW_ID,
  type HubRecordingDetail,
  type HubRecordings,
} from '../../shared/recording-view';
import { inputEmits, render, tauri } from '../test-tauri';
import { Recordings } from './Recordings';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const NOW = new Date(2026, 10, 19, 15, 0).getTime();
const list: HubRecordings = {
  rows: [
    {
      id: 'r3',
      name: '2026-11-19_1400',
      startedAt: new Date(2026, 10, 19, 14, 0).getTime(),
      durationS: 600,
      status: 'writing_notes',
      canOpen: true,
    },
    {
      id: 'r2',
      name: '2026-11-19_1000',
      startedAt: new Date(2026, 10, 19, 10, 0).getTime(),
      durationS: 3900,
      status: 'failed',
      error: RECORDING_ERRORS.transcribe_endpoint_missing,
      canOpen: true,
    },
    {
      id: 'r1',
      name: '2026-11-18_0900',
      startedAt: new Date(2026, 10, 18, 9, 0).getTime(),
      durationS: 1500,
      status: 'done',
      canOpen: true,
    },
  ],
};
const detail: HubRecordingDetail = {
  id: 'r1',
  status: 'done',
  notesMd: '## Summary\nWe agreed on **Friday**.\n\n<script>alert(1)</script> <b>raw</b>',
  turns: [
    { speaker: 'Speaker 1', startS: 0, text: 'Hello. Shall we start?' },
    { speaker: 'Speaker 2', startS: 83, text: 'Yes.' },
  ],
};

const emits = () =>
  inputEmits()
    .filter((e) => (e as { mogId: string }).mogId === RECORDING_VIEW_ID)
    .map((e) => (e as { payload: unknown }).payload);
const click = async (el: Element | null | undefined) =>
  act(async () => (el as HTMLElement).click());

describe('Recordings area (hub)', () => {
  beforeEach(() => tauri.reset());

  it('[REC-V12] asks for the list once it listens and shows a loading state', async () => {
    const { el } = await render(<Recordings active now={() => NOW} />);
    expect(emits()).toContainEqual({ action: 'recordings.ready' });
    expect(el.querySelector('[aria-busy="true"]')).not.toBeNull();
  });

  it('[REC-V12] an unreadable list shows the plain reason from the brain when it sends one', async () => {
    const { el } = await render(<Recordings active now={() => NOW} />);
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, {
      rows: [],
      unavailable: true,
      error: RECORDING_ERRORS.no_profile,
    });
    expect(el.textContent).toContain(RECORDING_ERRORS.no_profile.text);
  });

  it('[REC-V12] an empty list says how to record', async () => {
    const { el } = await render(<Recordings active now={() => NOW} />);
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, { rows: [] });
    expect(el.textContent).toContain('No recordings yet.');
  });

  it('[REC-V13] lists each recording newest first with date, length and status; no control inside an option', async () => {
    const { el } = await render(<Recordings active now={() => NOW} />);
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, list);
    const rows = [...el.querySelectorAll('[data-recording-row]')];
    expect(rows.map((r) => r.getAttribute('data-recording-row'))).toEqual(['r3', 'r2', 'r1']);
    expect(rows[0]?.textContent).toContain('10 min');
    expect(rows[0]?.textContent).toContain('Writing notes');
    expect(rows[1]?.textContent).toContain('1 h 5 min');
    expect(rows[1]?.textContent).toContain('Failed');
    expect(rows[2]?.textContent).toContain('Done');
    expect(el.textContent).toContain('Today');
    expect(el.textContent).toContain('Yesterday');
    for (const row of rows) expect(row.querySelector('button, a, input')).toBeNull();
  });

  it('[REC-V13] with nothing selected, the first row is the active option of the list', async () => {
    const { el } = await render(<Recordings active now={() => NOW} />);
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, list);
    const lb = el.querySelector('[role="listbox"]');
    const first = el.querySelector('[data-recording-row="r3"]');
    expect(lb?.getAttribute('aria-activedescendant')).toBe(first?.id);
    expect(first?.hasAttribute('data-active')).toBe(true);
    expect(first?.getAttribute('aria-selected')).toBe('false');
  });

  it('[REC-V16] Open in Databricks shows only once the audio is in the volume', async () => {
    const { el } = await render(<Recordings active now={() => NOW} />);
    const row = (status: string, canOpen: boolean) => ({
      id: 'x',
      name: 'x',
      startedAt: NOW - 1000,
      durationS: 60,
      status,
      canOpen,
    });
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, { rows: [row('uploading', false)] });
    await click(el.querySelector('[data-recording-row="x"]'));
    expect(el.querySelector('[data-recording-open]')).toBeNull();
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, { rows: [row('transcribing', true)] });
    expect(el.querySelector('[data-recording-open]')).not.toBeNull();
  });

  it('[REC-V14] a click opens the detail: safe notes, speaker turns with time stamps, Open in Databricks', async () => {
    const { el } = await render(<Recordings active now={() => NOW} />);
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, list);
    await click(el.querySelector('[data-recording-row="r1"]'));
    expect(emits().at(-1)).toEqual({ action: 'recordings.detail', id: 'r1' });
    await tauri.emitTo('hub', HUB_RECORDING_DETAIL_EVENT, detail);
    const d = el.querySelector('[data-recording-detail]');
    expect(d?.querySelector('h2, h3')).not.toBeNull();
    const strong = [...(d?.querySelectorAll('strong') ?? [])].map((s) => s.textContent);
    expect(strong).toContain('Friday');
    expect(d?.querySelector('script')).toBeNull();
    expect(d?.querySelector('b')).toBeNull();
    expect(d?.textContent).toContain('<b>raw</b>');
    const turns = [...(d?.querySelectorAll('[data-turn]') ?? [])];
    expect(turns.map((t) => t.textContent)).toEqual([
      expect.stringContaining('Speaker 1'),
      expect.stringContaining('01:23'),
    ]);
    expect(d?.querySelector('a[href]')).toBeNull();
    await click(d?.querySelector('[data-recording-link="folder"]'));
    expect(emits().at(-1)).toEqual({ action: 'recordings.open', id: 'r1', target: 'folder' });
  });

  it('[REC-V14] a failed recording shows its error text and one action in the detail', async () => {
    const onSettings = vi.fn();
    const { el } = await render(<Recordings active now={() => NOW} onOpenSettings={onSettings} />);
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, list);
    await click(el.querySelector('[data-recording-row="r2"]'));
    await tauri.emitTo('hub', HUB_RECORDING_DETAIL_EVENT, {
      id: 'r2',
      status: 'failed',
      turns: [],
      error: RECORDING_ERRORS.transcribe_endpoint_missing,
    });
    const alert = el.querySelector('[data-recording-detail] [role="alert"]');
    expect(alert?.textContent).toContain(RECORDING_ERRORS.transcribe_endpoint_missing.text);
    await click(alert?.querySelector('button'));
    expect(onSettings).toHaveBeenCalledTimes(1);
    await tauri.emitTo('hub', HUB_RECORDING_DETAIL_EVENT, {
      id: 'r2',
      status: 'failed',
      turns: [],
      error: RECORDING_ERRORS.upload_failed,
    });
    await click(el.querySelector('[data-recording-detail] [role="alert"] button'));
    expect(emits().at(-1)).toEqual({ action: 'recordings.retry', id: 'r2' });
  });

  it('[REC-V14] a detail for another recording than the selected one is not shown', async () => {
    const { el } = await render(<Recordings active now={() => NOW} />);
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, list);
    await click(el.querySelector('[data-recording-row="r3"]'));
    await tauri.emitTo('hub', HUB_RECORDING_DETAIL_EVENT, detail);
    expect(el.querySelector('[data-recording-detail]')?.textContent).not.toContain('Friday');
  });

  describe('every status and action (upload in parts)', () => {
    const base = { name: 'x', startedAt: NOW - 1000, durationS: 600, canOpen: false };
    const rowsOf = (...r: object[]) =>
      r.map((x, i) => ({ ...base, id: `s${i}`, startedAt: NOW - i * 1000, ...x }));
    const label = (el: Element, id: string) =>
      el.querySelector(`[data-recording-row="${id}"] [data-recording-status]`)?.textContent;

    it('[REC-P1] shows recording, uploading N/M, waiting for network, transcribing, writing notes, done, failed with a reason, and not uploaded', async () => {
      const { el } = await render(<Recordings active now={() => NOW} />);
      await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, {
        rows: rowsOf(
          { status: 'recording', partsUploaded: 1, partsTotal: 2 },
          { status: 'uploading', partsUploaded: 2, partsTotal: 5 },
          { status: 'recording', waitingNetwork: true, partsUploaded: 0, partsTotal: 3 },
          { status: 'transcribing', canOpen: true },
          { status: 'writing_notes', canOpen: true },
          { status: 'done', canOpen: true },
          {
            status: 'failed',
            error: RECORDING_ERRORS.volume_permission,
            canOpen: true,
          },
          { status: 'failed', onDeviceOnly: true },
        ),
      });
      expect([0, 1, 2, 3, 4, 5, 6, 7].map((i) => label(el, `s${i}`))).toEqual([
        'Recording',
        'Uploading 2/5',
        'Waiting for network',
        'Transcribing',
        'Writing notes',
        'Done',
        'Failed',
        'Not uploaded',
      ]);
      // The failed row gives its reason in the list; a device-only row has none of its own
      // (the brain adds the storage reason when there is one).
      expect(el.querySelector('[data-recording-row="s6"]')?.textContent).toContain(
        RECORDING_ERRORS.volume_permission.text,
      );
      expect(el.querySelector('[data-recording-row="s7"] [data-recording-reason]')).toBeNull();
    });

    it('[REC-P2] a device-only recording has Upload, not Open in volume, and asks for no detail', async () => {
      const { el } = await render(<Recordings active now={() => NOW} />);
      await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, {
        rows: rowsOf({
          status: 'failed',
          onDeviceOnly: true,
          error: RECORDING_ERRORS.upload_failed,
        }),
      });
      const before = emits().length;
      await click(el.querySelector('[data-recording-row="s0"]'));
      expect(emits().length).toBe(before);
      expect(el.querySelector('[data-recording-open]')).toBeNull();
      expect(el.querySelector('[data-recording-device]')?.textContent).toContain(
        'only on this device',
      );
      await click(el.querySelector('[data-recording-upload]'));
      expect(emits().at(-1)).toEqual({ action: 'recordings.retry', id: 's0' });
    });

    it('[REC-P4] Delete shows only on a row that never uploaded, asks first, and then sends the action', async () => {
      const { el } = await render(<Recordings active now={() => NOW} />);
      await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, {
        rows: rowsOf(
          { status: 'failed', onDeviceOnly: true, error: RECORDING_ERRORS.upload_failed },
          { status: 'failed', partsUploaded: 0, error: RECORDING_ERRORS.volume_permission },
          { status: 'failed', partsUploaded: 2, error: RECORDING_ERRORS.volume_permission },
          { status: 'done', canOpen: true },
          { status: 'uploading', partsUploaded: 0, partsTotal: 2 },
          { status: 'recording' },
        ),
      });
      const has = async (id: string) => {
        await click(el.querySelector(`[data-recording-row="${id}"]`));
        return el.querySelector('[data-recording-delete]') !== null;
      };
      expect(await has('s0')).toBe(true);
      expect(await has('s1')).toBe(true);
      expect(await has('s2')).toBe(false);
      expect(await has('s3')).toBe(false);
      expect(await has('s4')).toBe(false);
      expect(await has('s5')).toBe(false);
      // The question comes first; Cancel sends nothing.
      await click(el.querySelector('[data-recording-row="s1"]'));
      const before = emits().length;
      await click(el.querySelector('[data-recording-delete]'));
      expect(el.querySelector('[data-recording-confirm]')?.textContent).toContain(
        'Delete this recording? It is only on this Mac and cannot be recovered.',
      );
      expect(emits().length).toBe(before);
      await click(el.querySelector('[data-recording-delete-no]'));
      expect(el.querySelector('[data-recording-confirm]')).toBeNull();
      expect(emits().length).toBe(before);
      // Delete, then confirm: one action with the id.
      await click(el.querySelector('[data-recording-delete]'));
      await click(el.querySelector('[data-recording-delete-yes]'));
      expect(emits().at(-1)).toEqual({ action: 'recordings.delete', id: 's1' });
      expect(el.querySelector('[data-recording-confirm]')).toBeNull();
    });

    it('[REC-P3] Retry shows for a failed recording and sends the retry once', async () => {
      const { el } = await render(<Recordings active now={() => NOW} />);
      await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, {
        rows: rowsOf({
          status: 'failed',
          error: RECORDING_ERRORS.volume_permission,
          canOpen: true,
        }),
      });
      await click(el.querySelector('[data-recording-row="s0"]'));
      await click(el.querySelector('[data-recording-retry]'));
      expect(emits().at(-1)).toEqual({ action: 'recordings.retry', id: 's0' });
      // An error whose own action is Retry shows that action, not a second button.
      await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, {
        rows: rowsOf({ status: 'failed', error: RECORDING_ERRORS.network, canOpen: true }),
      });
      expect(el.querySelector('[data-recording-retry]')).toBeNull();
      expect(el.querySelectorAll('[role="alert"] button')).toHaveLength(1);
    });

    it('[REC-P4] Open in volume shows once a part is in the volume, while the call goes on', async () => {
      const { el } = await render(<Recordings active now={() => NOW} />);
      await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, {
        rows: rowsOf({ status: 'recording', partsUploaded: 1, partsTotal: 2, canOpen: true }),
      });
      await click(el.querySelector('[data-recording-row="s0"]'));
      await click(el.querySelector('[data-recording-open]'));
      expect(emits().at(-1)).toEqual({ action: 'recordings.open', id: 's0' });
    });

    it('[REC-P5] waiting for network explains that the audio stays on the device', async () => {
      const { el } = await render(<Recordings active now={() => NOW} />);
      await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, {
        rows: rowsOf({
          status: 'uploading',
          waitingNetwork: true,
          partsUploaded: 1,
          partsTotal: 3,
        }),
      });
      await click(el.querySelector('[data-recording-row="s0"]'));
      expect(el.querySelector('[data-recording-waiting]')?.textContent).toContain('network');
      expect(el.querySelector('[data-recording-retry], [data-recording-upload]')).toBeNull();
    });

    it('[REC-P6] a list update replaces the rows with no ask (the brain pushes on each job event)', async () => {
      const { el } = await render(<Recordings active now={() => NOW} />);
      await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, {
        rows: rowsOf({ status: 'uploading', partsUploaded: 1, partsTotal: 5 }),
      });
      expect(label(el, 's0')).toBe('Uploading 1/5');
      const asks = emits().length;
      await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, {
        rows: rowsOf({ status: 'uploading', partsUploaded: 4, partsTotal: 5 }),
      });
      expect(label(el, 's0')).toBe('Uploading 4/5');
      expect(emits().length).toBe(asks);
    });
  });
});

describe('storage banner, Upload all and file links (hub)', () => {
  beforeEach(() => tauri.reset());
  const base = new Date(2026, 10, 19, 9, 0).getTime();
  const waiting = (id: string, n: number, extra: Record<string, unknown> = {}) => ({
    id,
    name: `2026-11-19_09${n}0`,
    startedAt: base + n * 1000,
    durationS: 60,
    status: 'failed' as const,
    canOpen: false,
    onDeviceOnly: true as const,
    needsUpload: true as const,
    ...extra,
  });

  it('[REC-L1] shows the banner with the reason and a button to Settings', async () => {
    const onSettings = vi.fn();
    const { el } = await render(<Recordings active now={() => NOW} onOpenSettings={onSettings} />);
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, {
      rows: [waiting('a', 1)],
      storageProblem: { reason: RECORDING_TEXT.noMyFiles, canUpload: false },
      uploadAll: { running: false, total: 1, done: 0, failed: 0 },
    });
    const b = el.querySelector('[data-recording-banner]');
    expect(b?.textContent).toContain("Recordings can't upload: choose where to save them");
    expect(b?.textContent).toContain('No My Files on this workspace. Choose a volume in Settings.');
    await click(b?.querySelector('button'));
    expect(onSettings).toHaveBeenCalledTimes(1);
    // The row says only "Not uploaded"; the banner explains.
    expect(el.querySelector('[data-recording-row="a"]')?.textContent).toContain('Not uploaded');
    expect(el.querySelector('[data-recording-row="a"]')?.textContent).not.toContain('My Files');
    expect(
      el.querySelector<HTMLButtonElement>('[data-recording-upload-all] button')?.disabled,
    ).toBe(true);
  });

  it('[REC-L1] shows no banner when storage works', async () => {
    const { el } = await render(<Recordings active now={() => NOW} />);
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, list);
    expect(el.querySelector('[data-recording-banner]')).toBeNull();
    expect(el.querySelector('[data-recording-upload-all]')).toBeNull();
  });

  it('[REC-L2] Upload all sends one action and shows progress and the state of each row', async () => {
    const { el } = await render(<Recordings active now={() => NOW} />);
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, {
      rows: [waiting('a', 1), waiting('b', 2)],
      uploadAll: { running: false, total: 2, done: 0, failed: 0 },
    });
    const btn = el.querySelector<HTMLButtonElement>('[data-recording-upload-all] button');
    expect(btn?.textContent).toBe('Upload all (2)');
    await click(btn);
    expect(emits().at(-1)).toEqual({ action: 'recordings.upload_all' });
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, {
      rows: [waiting('a', 1, { batch: 'done' }), waiting('b', 2, { batch: 'running' })],
      uploadAll: { running: true, total: 2, done: 1, failed: 0 },
    });
    expect(
      el.querySelector<HTMLButtonElement>('[data-recording-upload-all] button')?.disabled,
    ).toBe(true);
    expect(el.querySelector('[data-recording-upload-progress]')?.textContent).toBe('2 of 2');
    const chips = [...el.querySelectorAll('[data-recording-batch]')].map((c) => c.textContent);
    expect(chips).toEqual(['Uploaded', 'Uploading…']);
  });

  it('[REC-L3] only the detail pane links to the transcript, the notes and the folder', async () => {
    const { el } = await render(<Recordings active now={() => NOW} />);
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, list);
    // The list stays clean: no control inside the listbox at all.
    expect(el.querySelector('.rec-row-links')).toBeNull();
    expect(el.querySelector('[role="listbox"] button')).toBeNull();
    await click(el.querySelector('[data-recording-row="r1"]'));
    await tauri.emitTo('hub', HUB_RECORDING_DETAIL_EVENT, detail);
    const d = el.querySelector('[data-recording-detail]');
    expect(d?.querySelector('[data-recording-link="transcript"]')?.textContent).toBe(
      'Open transcript in Databricks',
    );
    expect(d?.querySelector('[data-recording-link="notes"]')?.textContent).toBe(
      'Open notes in Databricks',
    );
    expect(d?.querySelector('[data-recording-file="transcript"]')?.textContent).toBe(
      '2026-11-18_0900.transcript.md',
    );
    expect(d?.querySelector('[data-recording-file="notes"]')?.textContent).toBe(
      '2026-11-18_0900.notes.md',
    );
    await click(d?.querySelector('[data-recording-link="transcript"]'));
    expect(emits().at(-1)).toEqual({ action: 'recordings.open', id: 'r1', target: 'transcript' });
    await click(d?.querySelector('[data-recording-link="notes"]'));
    expect(emits().at(-1)).toEqual({ action: 'recordings.open', id: 'r1', target: 'notes' });
    await click(d?.querySelector('[data-recording-link="folder"]'));
    expect(emits().at(-1)).toEqual({ action: 'recordings.open', id: 'r1', target: 'folder' });
  });

  it('[REC-L4] the transcript shows with speaker labels and times, or as text when turns are missing', async () => {
    const { el } = await render(<Recordings active now={() => NOW} />);
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, list);
    await click(el.querySelector('[data-recording-row="r1"]'));
    await tauri.emitTo('hub', HUB_RECORDING_DETAIL_EVENT, {
      id: 'r1',
      status: 'done',
      turns: [],
      transcriptMd: '**Speaker 1** [00:00:00]\nHi.',
    });
    expect(el.querySelector('[data-transcript-raw]')?.textContent).toContain('Speaker 1');
    await tauri.emitTo('hub', HUB_RECORDING_DETAIL_EVENT, detail);
    expect(el.querySelector('[data-transcript-raw]')).toBeNull();
    expect(el.querySelectorAll('[data-turn]')).toHaveLength(2);
  });
});
