// @vitest-environment jsdom
/**
 * Inspection checklist for Table SEC-DISC (SEC-17, AT-24). The test reads the table from
 * research/final-requirements.md, so a new row in the table fails here until it has copy.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { render, tauri } from '../test-tauri';
import { DataPanel, OnboardingDisclosures } from './DataDisclosures';
import { SEC_DISC, type Disclosure } from './disclosures';
import { Hub } from './Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const root = resolve(import.meta.dirname, '../../..');
const requirements = readFileSync(resolve(root, 'research/final-requirements.md'), 'utf8');

/** The items of Table SEC-DISC, verbatim, in table order. Throws if the table is missing. */
function secDiscItems(md: string): string[] {
  const line = md.split('\n').find((l) => l.startsWith('**Table SEC-DISC'));
  if (!line) throw new Error('Table SEC-DISC not found in the requirements');
  const body = line.replace(/^\*\*Table SEC-DISC[^*]*\*\*\s*/, '').replace(/\.\s*$/, '');
  return body.split(' · ').map((s) => s.trim());
}

const squash = (s: string) => s.replace(/\s+/g, ' ').trim();
const byId = (id: string): Disclosure => {
  const d = SEC_DISC.find((x) => x.id === id);
  if (!d) throw new Error(`no disclosure ${id}`);
  return d;
};
const text = (id: string) => byId(id).lines.join(' ');
const allCopy = () => SEC_DISC.flatMap((d) => [d.title, ...d.lines]);

function expectRendersEvery(el: HTMLElement): void {
  for (const d of SEC_DISC) {
    const node = el.querySelector(`[data-disclosure="${d.id}"]`);
    expect(node, d.item).not.toBeNull();
    expect(node?.querySelector('h3')?.textContent).toContain(d.title);
    const shown = squash(node?.textContent ?? '');
    for (const line of d.lines) expect(shown).toContain(squash(line));
    expect(node?.hasAttribute('data-v1')).toBe(d.v1);
  }
}

describe('Table SEC-DISC disclosures', () => {
  beforeEach(() => tauri.reset());

  it('[SEC-17] [AT-24] reads the SEC-DISC items from the requirements', () => {
    const items = secDiscItems(requirements);
    expect(items.length).toBeGreaterThanOrEqual(8);
    expect(items).toContain('Home-profile processing');
    expect(items).toContain('"Where your data goes"');
    expect(() => secDiscItems('no table here')).toThrow(/SEC-DISC not found/);
  });

  it('[SEC-17] [AT-24] has exactly one disclosure for each SEC-DISC item, and no extra', () => {
    const items = secDiscItems(requirements);
    const keys = SEC_DISC.map((d) => d.item);
    expect(new Set(keys).size).toBe(keys.length);
    expect(new Set(SEC_DISC.map((d) => d.id)).size).toBe(SEC_DISC.length);
    expect([...keys].sort()).toEqual([...items].sort());
    for (const d of SEC_DISC) expect(d.v1).toBe(d.item.startsWith('(v1)'));
  });

  it('[SEC-17] [AT-24] the hub data panel shows every SEC-DISC disclosure', async () => {
    const { el } = await render(<Hub />);
    const panel = el.querySelector<HTMLElement>('section[data-view="data-panel"]');
    expect(panel).not.toBeNull();
    expect(panel?.querySelector('h2')?.textContent).toBe('Where your data goes');
    expectRendersEvery(panel as HTMLElement);
  });

  it('[SEC-17] [AT-24] the onboarding disclosures show every SEC-DISC disclosure', async () => {
    const { el } = await render(<OnboardingDisclosures />);
    const section = el.querySelector<HTMLElement>('section[data-view="onboarding-disclosures"]');
    expect(section).not.toBeNull();
    expectRendersEvery(section as HTMLElement);
  });

  it('[SEC-17] the panel and onboarding render the same copy from one source', async () => {
    const a = await render(<DataPanel />);
    const b = await render(<OnboardingDisclosures />);
    for (const d of SEC_DISC) {
      const pa = a.el.querySelector(`[data-disclosure="${d.id}"]`)?.textContent;
      const pb = b.el.querySelector(`[data-disclosure="${d.id}"]`)?.textContent;
      expect(pa).toBe(pb);
    }
  });

  it('[SEC-17] STT: local Parakeet is the default and Databricks ASR is opt-in', () => {
    const t = text('stt-location');
    expect(t).toMatch(/on this computer by default/);
    expect(t).toMatch(/Parakeet/);
    expect(t).toMatch(/Databricks speech-to-text only if you or your admin turn it on/);
    // CAP-12c (#21): PTT and hands-free audio never go to a file. The OS can page memory, and
    // the Windows page file is not encrypted by default (#500), so the text must not promise
    // "never on disk".
    expect(t).toMatch(
      /Push-to-talk and hands-free audio stay in memory, and Bottle never writes them to a file/,
    );
    expect(t).toMatch(/On Windows the page file is not encrypted by default/);
    expect(t).not.toMatch(/never written to disk/);
  });

  it('[SEC-17] home-profile processing names the model calls and the hop sentence', () => {
    const t = text('home-profile');
    for (const w of ['cleanup', 'router step', 'memory writer', 'embeddings', 'main agent model']) {
      expect(t).toContain(w);
    }
    expect(t).toContain('Questions about another workspace are processed in your home workspace.');
  });

  it('[SEC-17] the main agent harness and model location, and tokens stay in the core', () => {
    const t = text('agent-harness');
    expect(t).toMatch(/Claude or Codex harness/);
    expect(t).toMatch(/on this computer/);
    expect(t).toMatch(/Databricks AI Gateway/);
    expect(t).toMatch(/local model/);
    expect(t).toMatch(/never gets your sign-in tokens/);
    expect(text('where-data-goes')).toMatch(/tokens stay in the Bottle core/);
  });

  it('[SEC-17] the summary names cleanup egress and the Appendix B side effects', () => {
    const t = text('where-data-goes');
    expect(t).toMatch(/Dictation text goes to your home workspace for cleanup/);
    expect(t).not.toMatch(/stay in an encrypted file/);
    for (const w of [
      'system.ai_gateway.usage',
      'abuse-check store',
      'inference tables',
      'Gemini global endpoint',
    ]) {
      expect(t).toContain(w);
    }
  });

  it('[SEC-17] Genie history and Genie One memory are disclosed', () => {
    expect(text('genie-history')).toMatch(/Genie history/);
    expect(text('genie-history')).toMatch(/system\.access\.audit/);
    expect(text('genie-history')).toMatch(/system\.query\.history/);
    expect(text('genie-history')).not.toMatch(/after 90 days/);
    expect(text('genie-one-memory')).toMatch(/server/);
    expect(text('genie-one-memory')).toMatch(/Forget/);
  });

  it('[SEC-17] DRAFT Pages are visible to Genie One for the owner; no claim about spaces or agents', () => {
    const t = text('draft-pages');
    expect(t).toMatch(/DRAFT/);
    expect(t).toMatch(/Genie One can read your DRAFT Pages in your own conversations/);
    expect(allCopy().join(' ')).not.toMatch(
      /Genie spaces? (can )?reads?|agents? (can )?reads? (your )?Pages/i,
    );
  });

  it('[SEC-17] the v1 Lakebase PITR window is disclosed with its 2-day default', () => {
    const d = byId('lakebase-pitr');
    expect(d.v1).toBe(true);
    expect(d.lines.join(' ')).toMatch(/2 days/);
    expect(d.lines.join(' ')).toMatch(/not in this version/);
  });

  it('[SEC-17] the copy is plain text and short (no markup, 25 words or less a sentence)', async () => {
    for (const s of allCopy()) {
      expect(s).not.toMatch(/[<>*`#[\]]/);
      for (const sentence of s.split(/(?<=[.!?])\s+/)) {
        expect(sentence.split(/\s+/).length, sentence).toBeLessThanOrEqual(25);
      }
    }
    const { el } = await render(<DataPanel />);
    expect(el.querySelectorAll('a, script, img, iframe').length).toBe(0);
  });

  it('[SEC-17] the user doc lists every disclosure title and sentence', () => {
    const userDoc = readFileSync(resolve(root, 'docs/user/where-your-data-goes.md'), 'utf8');
    for (const d of SEC_DISC) {
      expect(userDoc).toContain(`## ${d.title}`);
      for (const line of d.lines) expect(userDoc).toContain(`- ${line}`);
    }
  });
});
