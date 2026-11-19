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
    },
    {
      id: 'r2',
      name: '2026-11-19_1000',
      startedAt: new Date(2026, 10, 19, 10, 0).getTime(),
      durationS: 3900,
      status: 'failed',
      error: RECORDING_ERRORS.transcribe_endpoint_missing,
    },
    {
      id: 'r1',
      name: '2026-11-18_0900',
      startedAt: new Date(2026, 10, 18, 9, 0).getTime(),
      durationS: 1500,
      status: 'done',
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

  it('[REC-V12] an empty list says how to record', async () => {
    const { el } = await render(<Recordings active now={() => NOW} />);
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, { rows: [] });
    expect(el.textContent).toContain('No recordings yet.');
  });

  it('[REC-V13] lists each recording newest first with date, length and status; failed has Retry', async () => {
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
    await click(rows[1]?.querySelector('[data-recording-retry]'));
    expect(emits().at(-1)).toEqual({ action: 'recordings.retry', id: 'r2' });
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
    await click(d?.querySelector('[data-recording-open]'));
    expect(emits().at(-1)).toEqual({ action: 'recordings.open', id: 'r1' });
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
});
