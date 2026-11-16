// @vitest-environment jsdom
import {
  bannedNameHits,
  htmlTitle,
  markdownProse,
  mogNounHits,
  rustWindowTitles,
  visibleTexts,
} from './naming-rules';

// Built from parts, so a repository scan never finds the banned names in this file.
const OLD = ['Mother', 'Bricks'];

describe('naming-rules helpers (#71)', () => {
  it('[MB-10a] finds each spelling of the former product name', () => {
    for (const s of [OLD.join(' '), OLD.join(''), OLD.join('').toLowerCase(), OLD.join('-')]) {
      expect(bannedNameHits(`About ${s} here`)).toEqual([s]);
    }
    expect(bannedNameHits('Bottle (Databricks Labs / experimental)')).toEqual([]);
    expect(bannedNameHits('Mother Databricks')).toEqual([]);
  });

  it('[MB-10c] flags the MOG noun in any case other than MOG or MOGs', () => {
    expect(mogNounHits('Bottle mog')).toEqual(['mog']);
    expect(mogNounHits('Add a Mog. Two mogs.')).toEqual(['Mog', 'mogs']);
    expect(mogNounHits('Bottle MOG card; MOGs; MOG.')).toEqual([]);
  });

  it('[MB-10c] reads each text node and attribute of a view on its own', () => {
    const el = document.createElement('div');
    el.innerHTML =
      '<p>Add a mog</p><ul><li>MOG 1</li></ul><button aria-label="New Mog"></button>' +
      '<pre data-notices>crates/mog-core mog</pre>';
    expect(visibleTexts(el).flatMap(mogNounHits)).toEqual(['mog', 'Mog']);
  });

  it('[MB-10a] reads a former name split by an HTML space entity', () => {
    expect(bannedNameHits(OLD.join('&nbsp;'))).toEqual([OLD.join('&nbsp;')]);
  });

  it('[MB-10c] does not flag identifiers, labels and paths', () => {
    const ids = 'crates/mog-core mog-core-ffi mog-<id> mog.html mog:view_state mog_drag <mog>';
    expect(mogNounHits(ids)).toEqual([]);
    expect(mogNounHits('MogView asMogViewState')).toEqual([]);
  });

  it('[MB-10c] reads Markdown prose without code', () => {
    const md = 'A MOG.\n\n```\nmog x\n```\n\nSee `mog` and a mog.\n';
    expect(mogNounHits(markdownProse(md))).toEqual(['mog']);
  });

  it('[MB-10b] reads the HTML title and the Rust window titles', () => {
    expect(htmlTitle('<head><title> Bottle MOG </title></head>')).toBe('Bottle MOG');
    expect(htmlTitle('<p>none</p>')).toBeUndefined();
    expect(rustWindowTitles('a { title: "Bottle", x: 1 }\nb { title: "MOG card" }')).toEqual([
      'Bottle',
      'MOG card',
    ]);
  });
});
