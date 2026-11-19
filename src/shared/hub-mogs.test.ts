import { asHubMogList, asMogAction, HUB_MOGS_EVENT, HUB_SCOPE } from './hub-mogs';

const row = {
  mogId: 'a',
  name: 'Sales',
  home: 'p',
  persona: '',
  running: false,
  colorVariant: 'coral',
};
const list = { mogs: [row], unreadable: [], runningCount: 0, maxRunning: 8 };

describe('hub MOG list shapes (#26, TODO(#407))', () => {
  it('[MB-01] the view-local names are hub:mogs and the hub scope', () => {
    expect(HUB_MOGS_EVENT).toBe('hub:mogs');
    expect(HUB_SCOPE).toBe('hub');
  });

  it('[MB-01] asMogAction accepts the eight hub actions', () => {
    const ok = [
      { action: 'mogs.ready' },
      { action: 'mog.create', name: 'A', home: 'p' },
      { action: 'mog.edit', mogId: 'a', name: 'A', home: 'p', persona: '' },
      { action: 'mog.duplicate', mogId: 'a' },
      { action: 'mog.start', mogId: 'a' },
      { action: 'mog.stop', mogId: 'a' },
      { action: 'mog.delete', mogId: 'a', confirmed: true },
      { action: 'mog.set_color', mogId: 'a', colorVariant: 'teal' },
    ];
    for (const a of ok) expect(asMogAction(a)).toEqual(a);
  });

  it('[MB-01] asMogAction drops a bad action and keeps only known fields', () => {
    for (const bad of [
      null,
      'mog.start',
      { action: 'mog.start' },
      { action: 'mog.start', mogId: 3 },
      { action: 'mog.delete', mogId: 'a' },
      { action: 'mog.delete', mogId: 'a', confirmed: 'yes' },
      { action: 'mog.create', name: 'A' },
      { action: 'mog.edit', mogId: 'a', name: 'A', home: 'p' },
      { action: 'mog.set_color', mogId: 'a' },
      { action: 'mog.set_color', colorVariant: 'teal' },
      { action: 'notice.accept', noticeVersion: 1 },
    ])
      expect(asMogAction(bad)).toBeUndefined();
    expect(asMogAction({ action: 'mog.stop', mogId: 'a', extra: 1 })).toEqual({
      action: 'mog.stop',
      mogId: 'a',
    });
  });

  it('[MB-01] asHubMogList checks the list the brain sends', () => {
    expect(asHubMogList(list)).toEqual(list);
    const withNotice = { ...list, notice: { kind: 'error', text: 'x' } };
    expect(asHubMogList(withNotice)).toEqual(withNotice);
    for (const bad of [
      null,
      { ...list, mogs: [{ ...row, running: 'no' }] },
      { ...list, mogs: [{ ...row, colorVariant: 7 }] },
      {
        ...list,
        mogs: [{ mogId: 'a', name: 'Sales', home: 'p', persona: '', running: false }],
      },
      { ...list, unreadable: [1] },
      { ...list, maxRunning: '8' },
      { ...list, notice: { kind: 'shout', text: 'x' } },
    ])
      expect(asHubMogList(bad)).toBeUndefined();
  });

  it('[ui-4] [#603] asHubMogList accepts and checks openEdit ("Change home workspace…")', () => {
    const withEdit = { ...list, openEdit: { mogId: 'a', token: 1 } };
    expect(asHubMogList(withEdit)).toEqual(withEdit);
    for (const bad of [
      { ...list, openEdit: { mogId: 'a' } },
      { ...list, openEdit: { token: 1 } },
      { ...list, openEdit: { mogId: 7, token: 1 } },
      { ...list, openEdit: { mogId: 'a', token: '1' } },
      { ...list, openEdit: 'a' },
    ])
      expect(asHubMogList(bad)).toBeUndefined();
  });
});
