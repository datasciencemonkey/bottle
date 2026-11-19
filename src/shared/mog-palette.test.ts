import {
  CARD_LABEL_TEXT_HEX,
  CARD_SURFACE_HEX,
  contrastRatio,
  DEFAULT_VARIANT,
  MOG_PALETTE,
  nextVariant,
  paletteColorOf,
  PALETTE_TEXT,
} from './mog-palette';

describe('MOG identity palette (#ui-4)', () => {
  it('has 6 to 8 colours, each with a unique id', () => {
    expect(MOG_PALETTE.length).toBeGreaterThanOrEqual(6);
    expect(MOG_PALETTE.length).toBeLessThanOrEqual(8);
    expect(new Set(MOG_PALETTE.map((c) => c.id)).size).toBe(MOG_PALETTE.length);
  });

  it('[a11y] white pill text is at or above 4.5:1 on every palette colour', () => {
    for (const c of MOG_PALETTE) {
      const ratio = contrastRatio(c.hex, PALETTE_TEXT);
      expect(ratio).toBeDefined();
      expect(ratio as number).toBeGreaterThanOrEqual(4.5);
    }
  });

  it('[a11y] [#603] the Genie card label text is at or above 4.5:1 on the card surface (the real use, not the pill)', () => {
    // #603 review, finding 5: an accent colour (right for the pill's white-on-accent fill) read
    // 2.1:1 to 3:1 as text on the card's dark surface. `.genie-identity` now uses this neutral
    // pair instead and carries the colour with a dot, so this checks the pair the card actually
    // renders, with the real WCAG formula.
    const ratio = contrastRatio(CARD_LABEL_TEXT_HEX, CARD_SURFACE_HEX);
    expect(ratio).toBeDefined();
    expect(ratio as number).toBeGreaterThanOrEqual(4.5);
  });

  it('contrastRatio is symmetric and 1 for a colour against itself', () => {
    expect(contrastRatio('#000000', '#ffffff')).toBeCloseTo(21, 0);
    expect(contrastRatio('#ffffff', '#000000')).toBeCloseTo(21, 0);
    expect(contrastRatio('#b8322a', '#b8322a')).toBeCloseTo(1, 5);
  });

  it('contrastRatio returns undefined for a value that is not #rrggbb', () => {
    expect(contrastRatio('not-a-colour', '#ffffff')).toBeUndefined();
    expect(contrastRatio('#fff', '#ffffff')).toBeUndefined();
  });

  describe('nextVariant (deterministic assignment at create time)', () => {
    it('picks the first unused palette colour', () => {
      const used = [MOG_PALETTE[0]?.id ?? '', MOG_PALETTE[1]?.id ?? ''];
      expect(nextVariant(used)).toBe(MOG_PALETTE[2]?.id);
    });

    it('the first MOG (no MOGs yet) gets the default variant', () => {
      expect(nextVariant([])).toBe(DEFAULT_VARIANT);
    });

    it('wraps by index once every colour is used, so it stays deterministic', () => {
      const all = MOG_PALETTE.map((c) => c.id);
      expect(nextVariant(all, all.length)).toBe(MOG_PALETTE[0]?.id);
      expect(nextVariant(all, all.length + 1)).toBe(MOG_PALETTE[1]?.id);
    });

    it('ignores how many times a colour repeats; only "used at all" matters', () => {
      const used = [MOG_PALETTE[0]?.id ?? '', MOG_PALETTE[0]?.id ?? '', MOG_PALETTE[0]?.id ?? ''];
      expect(nextVariant(used)).toBe(MOG_PALETTE[1]?.id);
    });
  });

  describe('paletteColorOf (lamp, pill, card and Conversations tint)', () => {
    it('a known variant id wins', () => {
      const c = MOG_PALETTE[3];
      if (!c) throw new Error('palette too small');
      expect(paletteColorOf(c.id, 'any-mog')).toEqual(c);
    });

    it('"auto" (the Appendix F default) falls back to a stable hash of the MOG ID', () => {
      const a = paletteColorOf('auto', 'mog-aaaa');
      const b = paletteColorOf('auto', 'mog-aaaa');
      expect(a).toEqual(b);
      expect(MOG_PALETTE).toContainEqual(a);
    });

    it('an unknown or legacy variant value also falls back by MOG ID, not by the value', () => {
      const a = paletteColorOf('some-legacy-value', 'mog-bbbb');
      const b = paletteColorOf(undefined, 'mog-bbbb');
      expect(a).toEqual(b);
    });

    it('two different MOG IDs usually resolve to different fallback colours', () => {
      const ids = Array.from({ length: 20 }, (_, i) => `mog-${i}`);
      const colours = new Set(ids.map((id) => paletteColorOf('auto', id).id));
      expect(colours.size).toBeGreaterThan(1);
    });
  });
});
