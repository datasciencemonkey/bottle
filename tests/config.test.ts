import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const pkg = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8')) as {
  name: string;
  dependencies?: Record<string, string>;
  devDependencies?: Record<string, string>;
};
const deps = Object.keys({ ...pkg.dependencies, ...pkg.devDependencies });

describe('front-end package rules', () => {
  it('[ARC-4] no window can load the http, shell, fs or network plugins', () => {
    const forbidden = deps.filter((d) =>
      /^@tauri-apps\/plugin-(http|shell|fs|websocket|upload|network)/.test(d),
    );
    expect(forbidden).toEqual([]);
  });

  it('[MB-10a] [MB-10c] the npm package is named bottle with no MOG in it', () => {
    expect(pkg.name).toBe('bottle');
    expect(pkg.name.toLowerCase()).not.toContain('mog');
  });

  it('[ARC-2] each window kind has an HTML entry that loads its own module', () => {
    const entries: Record<string, string> = {
      brain: '/src/brain/main.ts',
      hub: '/src/views/hub/main.tsx',
      mog: '/src/views/mog/main.tsx',
      card: '/src/views/card/main.tsx',
    };
    for (const [kind, entry] of Object.entries(entries)) {
      const html = readFileSync(resolve(root, `${kind}.html`), 'utf8');
      expect(html).toContain(`src="${entry}"`);
      expect(html).not.toMatch(/<script(?![^>]*\bsrc=)[^>]*>/);
    }
  });
});
