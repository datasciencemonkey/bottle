import type { ExchangeRef } from '../shared/interfaces';
import { decideCardEsc, handleCardEsc, type CardEscDeps } from './card-esc';

const MOG = '00000000-0000-4000-8000-000000000001';
const ref: ExchangeRef = {
  backend: 'genie_one_mcp',
  profile: 'p',
  host: 'h',
  conversationId: 'c1',
};

function deps(fail?: 'cancel' | 'hide') {
  const calls: string[] = [];
  const d: CardEscDeps = {
    cancel: async (r) => {
      calls.push(`cancel ${r.conversationId}`);
      if (fail === 'cancel') throw new Error('x');
    },
    hideCard: async (m) => {
      calls.push(`hide ${m}`);
      if (fail === 'hide') throw new Error('x');
    },
  };
  return { d, calls };
}

describe('card Esc rule', () => {
  it('[UX-19b] Esc with a request in flight cancels the request and keeps the card', () => {
    expect(decideCardEsc({ inFlight: ref })).toEqual({ t: 'cancel_request', ref });
  });

  it('[UX-19b] Esc with no request in flight hides the card', () => {
    expect(decideCardEsc({})).toEqual({ t: 'hide_card' });
  });

  it('[UX-19b] handleCardEsc runs exactly one action', async () => {
    const a = deps();
    await expect(handleCardEsc(MOG, { inFlight: ref }, a.d)).resolves.toEqual({
      t: 'cancel_request',
      ok: true,
    });
    expect(a.calls).toEqual(['cancel c1']);
    const b = deps();
    await expect(handleCardEsc(MOG, {}, b.d)).resolves.toEqual({ t: 'hide_card', ok: true });
    expect(b.calls).toEqual([`hide ${MOG}`]);
  });

  it('[UX-19b] a failed cancel does not hide the card, so the card shows the request state', async () => {
    const a = deps('cancel');
    await expect(handleCardEsc(MOG, { inFlight: ref }, a.d)).resolves.toEqual({
      t: 'cancel_request',
      ok: false,
    });
    expect(a.calls).toEqual(['cancel c1']);
  });

  it('[UX-19b] a failed hide reports the failure and tries nothing else', async () => {
    const a = deps('hide');
    await expect(handleCardEsc(MOG, {}, a.d)).resolves.toEqual({ t: 'hide_card', ok: false });
    expect(a.calls).toEqual([`hide ${MOG}`]);
  });
});
