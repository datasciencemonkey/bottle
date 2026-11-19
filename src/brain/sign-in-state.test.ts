import { createSignInTracker } from './sign-in-state';
import type { TerminalAskEvent } from './components/ask-events';

const end = (t: TerminalAskEvent): { profile: string; end: TerminalAskEvent } => ({
  profile: 'DEFAULT',
  end: t,
});

describe('sign-in state (#560, #486)', () => {
  it('is unknown before any recorded outcome', () => {
    expect(createSignInTracker().signInState('DEFAULT')).toBe('unknown');
  });

  it('is signed_in after an answer, and after a clarify question (needs_input)', () => {
    const t = createSignInTracker();
    t.record(
      end({
        t: 'answer',
        md: 'hi',
        incomplete: false,
        keyNumbers: [],
        deepLink: '',
        queryItems: [],
      }),
    );
    expect(t.signInState('DEFAULT')).toBe('signed_in');
    const t2 = createSignInTracker();
    t2.record(end({ t: 'needs_input', kind: 'clarify', prompt: 'Which table?', options: [] }));
    expect(t2.signInState('DEFAULT')).toBe('signed_in');
  });

  it('is needs_sign_in after a failure with AUTH_EXPIRED or UNSUPPORTED_AUTH', () => {
    const t = createSignInTracker();
    t.record(end({ t: 'failed', code: 'AUTH_EXPIRED', userText: 'x', retry: 'relogin' }));
    expect(t.signInState('DEFAULT')).toBe('needs_sign_in');
    const t2 = createSignInTracker();
    t2.record(end({ t: 'failed', code: 'UNSUPPORTED_AUTH', userText: 'x', retry: 'relogin' }));
    expect(t2.signInState('DEFAULT')).toBe('needs_sign_in');
  });

  it('leaves the state unchanged on a failure that is not an auth code', () => {
    const t = createSignInTracker();
    t.record(end({ t: 'failed', code: 'RATE_LIMITED', userText: 'x', retry: 'none' }));
    expect(t.signInState('DEFAULT')).toBe('unknown');
    t.record(
      end({
        t: 'answer',
        md: 'hi',
        incomplete: false,
        keyNumbers: [],
        deepLink: '',
        queryItems: [],
      }),
    );
    t.record(end({ t: 'failed', code: 'OFFLINE', userText: 'x', retry: 'none' }));
    expect(t.signInState('DEFAULT')).toBe('signed_in');
  });

  it('is per profile', () => {
    const t = createSignInTracker();
    t.record({
      profile: 'A',
      end: { t: 'failed', code: 'AUTH_EXPIRED', userText: 'x', retry: 'relogin' },
    });
    t.record({
      profile: 'B',
      end: {
        t: 'answer',
        md: 'hi',
        incomplete: false,
        keyNumbers: [],
        deepLink: '',
        queryItems: [],
      },
    });
    expect(t.signInState('A')).toBe('needs_sign_in');
    expect(t.signInState('B')).toBe('signed_in');
    expect(t.signInState('C')).toBe('unknown');
  });

  it('tracks only the latest outcome per profile', () => {
    const t = createSignInTracker();
    t.record(end({ t: 'failed', code: 'AUTH_EXPIRED', userText: 'x', retry: 'relogin' }));
    t.record(
      end({
        t: 'answer',
        md: 'hi',
        incomplete: false,
        keyNumbers: [],
        deepLink: '',
        queryItems: [],
      }),
    );
    expect(t.signInState('DEFAULT')).toBe('signed_in');
  });
});
