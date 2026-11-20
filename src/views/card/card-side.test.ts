import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const css = (f: string) => readFileSync(resolve(import.meta.dirname, f), 'utf8');
const rule = (src: string, sel: string) =>
  new RegExp(`${sel.replace(/[[\]().*+?^$|]/g, '\\$&')}\\s*\\{([^}]*)\\}`).exec(src)?.[1] ?? '';

describe('lamp-alive: the tail is on the MOG side', () => {
  it.each([
    ['card.css', '.popup::before'],
    ['genie-card.css', '.genie::before'],
  ])('%s: a card on the right keeps the tail on its left edge', (file, sel) => {
    expect(rule(css(file), sel)).toMatch(/left:\s*-5px/);
  });

  it.each([
    ['card.css', '.popup::before'],
    ['genie-card.css', '.genie::before'],
  ])('%s: a card on the left puts the tail on its right edge, pointing at the MOG', (file, sel) => {
    const left = rule(css(file), `html[data-side='left'] ${sel}`);
    expect(left).toMatch(/right:\s*-5px/);
    expect(left).toMatch(/left:\s*auto/);
    expect(left).toMatch(/border-right:\s*1px solid/);
    expect(left).toMatch(/border-top:\s*1px solid/);
    // The rotated square keeps the same top, so the tail stays at head height.
    expect(left).not.toMatch(/(?<![-\w])top:/);
  });

  it('the fade drifts toward the MOG on both sides', () => {
    expect(rule(css('card.css'), ".popup[data-state='fading']")).toMatch(/translateX\(-4px\)/);
    expect(rule(css('card.css'), "html[data-side='left'] .popup[data-state='fading']")).toMatch(
      /translateX\(4px\)/,
    );
  });
});

describe('lamp-alive: the pill is next to the MOG and the window does not cover the lamp', () => {
  it('right-aligns the pill and the card on the left side', () => {
    expect(rule(css('card.css'), "html[data-side='left'] .card-popup .popup")).toMatch(
      /margin-left:\s*auto/,
    );
    expect(rule(css('genie-card.css'), "html[data-side='left'] .card-genie .genie")).toMatch(
      /margin-left:\s*auto/,
    );
  });

  it('has 10 px (the gap to the art) of window on the side that faces the MOG, and 12 on the far side', () => {
    expect(rule(css('card.css'), '.card-popup')).toMatch(/padding:\s*10px 12px 14px 10px/);
    expect(rule(css('card.css'), "html[data-side='left'] .card-popup")).toMatch(
      /padding:\s*10px 10px 14px 12px/,
    );
    expect(rule(css('genie-card.css'), '.card-genie')).toMatch(/padding:\s*10px 12px 14px 10px/);
    expect(rule(css('genie-card.css'), "html[data-side='left'] .card-genie")).toMatch(
      /padding:\s*10px 10px 14px 12px/,
    );
  });
});
