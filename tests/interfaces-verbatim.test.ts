import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');

describe('src/shared/interfaces.ts', () => {
  it('keeps the 4.7 block a verbatim copy of the requirements (amendments go after it)', () => {
    const req = readFileSync(resolve(root, 'research/final-requirements.md'), 'utf8');
    const section = req.slice(req.indexOf('### 4.7 Final TypeScript interfaces'));
    const spec = /```ts\n([\s\S]*?)\n```/.exec(section)?.[1];
    const src = readFileSync(resolve(root, 'src/shared/interfaces.ts'), 'utf8');
    const block = /\/\/ BEGIN 4\.7\n([\s\S]*?)\n\/\/ END 4\.7/.exec(src)?.[1];
    expect(spec).toBeDefined();
    expect(block).toBe(spec);
  });
});
