/**
 * The one workspace of a call recording (ADR 0103): Bottle uses a home profile only when every
 * MOG with a home has the same one. It never picks between workspaces.
 */
import { recordingProfile } from './recording-profile';

const deps = (homes: Record<string, string | undefined>) => ({
  mogs: async () => Object.keys(homes),
  home: async (id: string) => homes[id],
});

describe('recordingProfile (ADR 0103)', () => {
  it('[REC-H15] one distinct home among the MOGs is the recording profile', async () => {
    await expect(recordingProfile(deps({ a: 'work', b: 'work', c: undefined }))).resolves.toBe(
      'work',
    );
  });

  it('[REC-H15] no MOG with a home is no_profile', async () => {
    await expect(recordingProfile(deps({ a: undefined }))).rejects.toEqual({ error: 'no_profile' });
    await expect(recordingProfile(deps({}))).rejects.toEqual({ error: 'no_profile' });
  });

  it('[REC-H15] two different homes is profiles_differ: Bottle never picks one', async () => {
    await expect(recordingProfile(deps({ a: 'work', b: 'lab' }))).rejects.toEqual({
      error: 'profiles_differ',
    });
  });

  it('[REC-H15] a failed MOG list or home read is no_profile', async () => {
    await expect(
      recordingProfile({
        mogs: async () => {
          throw new Error('x');
        },
        home: async () => 'work',
      }),
    ).rejects.toEqual({ error: 'no_profile' });
  });
});
