/**
 * The one workspace of a call recording (ADR 0103, ADR 0117): the home profile. Bottle never
 * picks a workspace.
 */
import { recordingProfile } from './recording-profile';

describe('recordingProfile (ADR 0103, ADR 0117)', () => {
  it('[AC-1.8] the home profile is the recording profile', async () => {
    await expect(recordingProfile({ home: async () => 'work' })).resolves.toBe('work');
  });

  it('[AC-1.8] no home is no_profile: Bottle never picks one', async () => {
    await expect(recordingProfile({ home: async () => undefined })).rejects.toEqual({
      error: 'no_profile',
    });
    await expect(recordingProfile({ home: async () => '' })).rejects.toEqual({
      error: 'no_profile',
    });
  });

  it('[AC-1.8] a failed home read is no_profile', async () => {
    await expect(
      recordingProfile({
        home: async () => {
          throw new Error('x');
        },
      }),
    ).rejects.toEqual({ error: 'no_profile' });
  });

  it('[AC-1.8] the code profiles_differ no longer exists in the brain', async () => {
    const src = (await import('node:fs')).readFileSync(
      new URL('./recording-profile.ts', import.meta.url),
      'utf8',
    );
    expect(src.replace(/\/\*[\s\S]*?\*\//g, '')).not.toContain('profiles_differ');
  });
});
