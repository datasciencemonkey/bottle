// @vitest-environment jsdom
import { accessibleText, contrastRatio } from './a11y';

describe('a11y test helpers', () => {
  it('[AT-20] accessibleText prefers aria-labelledby, then aria-label, then the text', () => {
    document.body.innerHTML =
      '<span id="n">Sales</span><span id="s">listening</span>' +
      '<button id="a" aria-label=" Stop  capture ">x</button>' +
      '<button id="b" aria-labelledby="n s" aria-label="ignored">y</button>' +
      '<button id="e" aria-labelledby="missing" aria-label="Fallback">z</button>' +
      '<span id="c"> REC </span><span id="d"></span>';
    const get = (id: string) => document.getElementById(id) as Element;
    expect(accessibleText(get('a'))).toBe('Stop capture');
    expect(accessibleText(get('b'))).toBe('Sales listening');
    expect(accessibleText(get('e'))).toBe('Fallback');
    expect(accessibleText(get('c'))).toBe('REC');
    expect(accessibleText(get('d'))).toBe('');
  });

  it('[AT-20] contrastRatio gives the WCAG ratio and rejects other colour forms', () => {
    expect(contrastRatio('#000000', '#ffffff')).toBeCloseTo(21, 5);
    expect(contrastRatio('#ffffff', '#000000')).toBeCloseTo(21, 5);
    expect(contrastRatio('#767676', '#ffffff')).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio('#777777', '#ffffff')).toBeLessThan(4.5);
    expect(() => contrastRatio('red', '#ffffff')).toThrow();
  });
});
