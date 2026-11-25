// @vitest-environment jsdom
/** ADR 0122: Settings → Workspace → "MOG agent". All values are fake. */
import { act } from 'react';
import {
  HUB_SCOPE,
  type HubWorkspaceList,
  type HubWorkspaceRow,
} from '../../shared/hub-workspaces';
import type { HubOrchestratorView } from '../../shared/hub-orchestrator';
import { inputEmits, render, tauri } from '../test-tauri';
import {
  AgentSettings,
  BUDGET_HELP,
  CLEAN_DICTATION_LABEL,
  COPY_GENIE_ANSWER_LABEL,
  DICTATION_ENGINE_LABEL,
  DICTATION_ENGINE_LOCAL,
  ORCHESTRATOR_LABEL,
} from './AgentSettings';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const row = (o: Partial<HubWorkspaceRow> & { profile: string }): HubWorkspaceRow => ({
  name: o.profile,
  host: `${o.profile}.fake.test`,
  colorVariant: 'coral',
  signIn: 'signed_in',
  available: true,
  isHome: false,
  ...o,
});
const ORCH: HubOrchestratorView = {
  models: [
    { id: 'system.ai.claude-haiku-5-5', label: 'Claude Haiku 5.5' },
    { id: 'system.ai.gpt-oss-20b', label: 'GPT OSS 20B' },
  ],
  selected: 'system.ai.claude-haiku-5-5',
  cleanDictation: true,
  cloudDictation: false,
  state: 'ready',
  homeName: 'Sales',
};
const list = (): HubWorkspaceList => ({
  rows: [
    row({ profile: 'sales', name: 'Sales', isHome: true }),
    row({ profile: 'fin', name: 'Finance' }),
    row({ profile: 'pat', name: 'Token only', signIn: 'unsupported' }),
  ],
  home: 'sales',
  agent: { genieCallBudget: 10, copyGenieAnswer: true },
});
const clicks = () => inputEmits().map((e) => (e as { payload: unknown }).payload);
const act2 = (fn: () => void) => act(async () => fn());
const click = (el: Element | null | undefined) => act2(() => (el as HTMLElement).click());
beforeEach(() => tauri.reset());
afterEach(() => {
  document.body.innerHTML = '';
});

describe('AgentSettings', () => {
  const model = (el: Element) => el.querySelector<HTMLSelectElement>('[data-orchestrator-model]');
  const clean = (el: Element) => el.querySelector<HTMLElement>('[data-orchestrator-clean]');
  const copyAnswer = (el: Element) => el.querySelector<HTMLElement>('[data-copy-genie-answer]');

  it('has no row for the old session host and no agent workspace menu', async () => {
    const { el } = await render(<AgentSettings list={list()} orchestrator={ORCH} />);
    expect(el.textContent ?? '').not.toMatch(/genie\s*code/i);
    expect(el.querySelector('[data-agent-profile]')).toBeNull();
    expect(el.querySelector('[data-agent-install], [data-agent-recheck]')).toBeNull();
  });

  it('[ADR 0122] the Orchestrator model is a labelled select that lists each model by its plain label', async () => {
    const { el } = await render(<AgentSettings list={list()} orchestrator={ORCH} />);
    const sel = model(el);
    expect(sel?.tagName).toBe('SELECT');
    expect(document.getElementById(sel?.getAttribute('aria-labelledby') ?? '')?.textContent).toBe(
      ORCHESTRATOR_LABEL,
    );
    expect([...(sel?.options ?? [])].map((o) => [o.value, o.textContent])).toEqual([
      ['system.ai.claude-haiku-5-5', 'Claude Haiku 5.5'],
      ['system.ai.gpt-oss-20b', 'GPT OSS 20B'],
    ]);
    expect(sel?.value).toBe('system.ai.claude-haiku-5-5');
    // The help names the home workspace and says what the model does.
    const help = document.getElementById(
      sel?.getAttribute('aria-describedby')?.split(' ')[0] ?? '',
    );
    expect(help?.textContent).toBe(
      "Runs on Sales's AI Gateway. Cleans up dictation and answers ⌥G by asking your workspaces' Genies.",
    );
  });

  it('shows no test label anywhere: no "tested" and no "not tested" text', async () => {
    const { el } = await render(<AgentSettings list={list()} orchestrator={ORCH} />);
    expect(el.textContent ?? '').not.toMatch(/tested/i);
    expect(el.innerHTML).not.toMatch(/tested/i);
  });

  it('a selection sends the model id', async () => {
    const { el } = await render(<AgentSettings list={list()} orchestrator={ORCH} />);
    await act2(() => {
      const sel = model(el);
      if (!sel) return;
      // React listens to the change event; set the value the way the browser does.
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value')?.set?.call(
        sel,
        'system.ai.gpt-oss-20b',
      );
      sel.dispatchEvent(new Event('change', { bubbles: true }));
    });
    expect(clicks()).toEqual([{ action: 'orchestrator.model', model: 'system.ai.gpt-oss-20b' }]);
  });

  it('"Clean up dictation" is a labelled switch, on by default, that sends the new value', async () => {
    const { el } = await render(<AgentSettings list={list()} orchestrator={ORCH} />);
    const sw = clean(el);
    expect(sw?.getAttribute('role')).toBe('switch');
    expect(sw?.getAttribute('aria-checked')).toBe('true');
    expect(document.getElementById(sw?.getAttribute('aria-labelledby') ?? '')?.textContent).toBe(
      CLEAN_DICTATION_LABEL,
    );
    expect(document.getElementById(sw?.getAttribute('aria-describedby') ?? '')).not.toBeNull();
    await click(sw);
    expect(clicks()).toEqual([{ action: 'orchestrator.clean_dictation', on: false }]);
    tauri.reset();
    const off = await render(
      <AgentSettings list={list()} orchestrator={{ ...ORCH, cleanDictation: false }} />,
    );
    expect(clean(off.el)?.getAttribute('aria-checked')).toBe('false');
    await click(clean(off.el));
    expect(clicks()).toEqual([{ action: 'orchestrator.clean_dictation', on: true }]);
  });

  it('"Copy Genie answers to the clipboard" is a labelled switch, on by default, that sends the new value', async () => {
    const { el } = await render(<AgentSettings list={list()} orchestrator={ORCH} />);
    const sw = copyAnswer(el);
    expect(sw?.getAttribute('role')).toBe('switch');
    expect(sw?.getAttribute('aria-checked')).toBe('true');
    expect(document.getElementById(sw?.getAttribute('aria-labelledby') ?? '')?.textContent).toBe(
      COPY_GENIE_ANSWER_LABEL,
    );
    await click(sw);
    expect(clicks()).toEqual([{ action: 'workspace.copy_genie_answer', on: false }]);
  });

  it('"Copy Genie answers to the clipboard" still shows with no orchestrator model (the direct Genie fallback)', async () => {
    const { el } = await render(<AgentSettings list={list()} />);
    expect(copyAnswer(el)?.getAttribute('aria-checked')).toBe('true');
    // No orchestrator view: the model row and "Clean up dictation" do not render, but this does.
    expect(model(el)).toBeNull();
    expect(clean(el)).toBeNull();
  });

  it('[ADR 0123] "Dictation engine" is a two-way choice, On this Mac by default, that sends the new value', async () => {
    const eng = (el: Element, v: string) =>
      el.querySelector<HTMLElement>(`[data-dictation-engine] [data-engine="${v}"]`);
    const { el } = await render(<AgentSettings list={list()} orchestrator={ORCH} />);
    const group = el.querySelector('[data-dictation-engine]');
    expect(group?.getAttribute('role')).toBe('radiogroup');
    expect(document.getElementById(group?.getAttribute('aria-labelledby') ?? '')?.textContent).toBe(
      DICTATION_ENGINE_LABEL,
    );
    expect(eng(el, 'local')?.getAttribute('aria-checked')).toBe('true');
    expect(eng(el, 'databricks')?.getAttribute('aria-checked')).toBe('false');
    expect(el.textContent).toContain('On this Mac');
    expect(el.querySelector('[data-dictation-engine-help]')?.textContent).toBe(
      DICTATION_ENGINE_LOCAL,
    );
    await click(eng(el, 'databricks'));
    expect(clicks()).toEqual([{ action: 'orchestrator.cloud_dictation', on: true }]);
    tauri.reset();
    const cloud = await render(
      <AgentSettings list={list()} orchestrator={{ ...ORCH, cloudDictation: true }} />,
    );
    expect(eng(cloud.el, 'databricks')?.getAttribute('aria-checked')).toBe('true');
    expect(cloud.el.querySelector('[data-dictation-engine-help]')?.textContent).toBe(
      "Databricks: your voice is sent to your home workspace's AI Gateway to transcribe it.",
    );
    await click(eng(cloud.el, 'local'));
    expect(clicks()).toEqual([{ action: 'orchestrator.cloud_dictation', on: false }]);
  });

  it('shows quiet status text with the reason when the orchestrator is not ready', async () => {
    const ready = await render(<AgentSettings list={list()} orchestrator={ORCH} />);
    expect(ready.el.querySelector('[data-orchestrator-status]')).toBeNull();
    const texts: [NonNullable<HubOrchestratorView['state']>, string][] = [
      ['no_home', 'Choose a home workspace first.'],
      ['signed_out', 'Your sign-in to Sales expired.'],
      ['no_model', 'This model is not available in Sales. Pick another.'],
      ['unreachable', 'The orchestrator model is not reachable.'],
    ];
    for (const [state, text] of texts) {
      const { el } = await render(
        <AgentSettings list={list()} orchestrator={{ ...ORCH, state }} />,
      );
      expect(el.querySelector('[data-orchestrator-status]')?.textContent).toContain(text);
      expect(model(el)?.getAttribute('aria-describedby')).toContain(
        el.querySelector('[role="status"]')?.id ?? 'none',
      );
    }
  });

  it('Check again sends one recheck when the model is unreachable', async () => {
    const { el } = await render(
      <AgentSettings list={list()} orchestrator={{ ...ORCH, state: 'unreachable' }} />,
    );
    await click(el.querySelector('[data-orchestrator-recheck]'));
    expect(clicks()).toEqual([{ action: 'orchestrator.recheck' }]);
  });

  it('draws no orchestrator row before the brain sends the view', async () => {
    const { el } = await render(<AgentSettings list={list()} />);
    expect(model(el)).toBeNull();
    expect(el.querySelector('[data-agent-budget]')).not.toBeNull();
  });

  it('the disclosure line names the orchestrator model and the home workspace', async () => {
    const { el } = await render(<AgentSettings list={list()} orchestrator={ORCH} />);
    expect(el.querySelector('[data-agent-disclosure]')?.textContent).toContain(
      'Bottle sends your question to the orchestrator model in Sales.',
    );
  });

  it('has no switch for D1 "Combine across workspaces" (removed, ADR 0122 amendment)', async () => {
    const { el } = await render(<AgentSettings list={list()} />);
    expect(el.querySelector('[data-agent-combine]')).toBeNull();
    expect(el.textContent ?? '').not.toMatch(/combine across workspaces/i);
  });

  it('the disclosure line says the model may combine answers from more than one workspace', async () => {
    const { el } = await render(<AgentSettings list={list()} />);
    expect(el.querySelector('[data-agent-disclosure]')?.textContent).toContain(
      'It may combine answers from more than one of your workspaces into one answer.',
    );
  });

  it('[D2] the budget is a labelled radio group; a click sends the value', async () => {
    const { el } = await render(<AgentSettings list={list()} />);
    const group = el.querySelector('[data-agent-budget]');
    expect(group?.getAttribute('role')).toBe('radiogroup');
    expect(group?.getAttribute('aria-labelledby')).toBeTruthy();
    expect(el.textContent).toContain(BUDGET_HELP);
    expect(el.querySelector('[data-budget="10"]')?.getAttribute('aria-checked')).toBe('true');
    await click(el.querySelector('[data-budget="20"]'));
    expect(clicks()).toEqual([{ action: 'workspace.budget', genieCallBudget: 20 }]);
  });

  it('every click uses the hub scope (ARC-4)', async () => {
    const { el } = await render(<AgentSettings list={list()} />);
    await click(el.querySelector('[data-budget="5"]'));
    expect(inputEmits()[0]).toMatchObject({ mogId: HUB_SCOPE, kind: 'click' });
  });
});
