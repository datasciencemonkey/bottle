/**
 * T7 ProfileService (4.3, hidden TS brain window). Implements the 4.7 `ProfileService`.
 *
 * Responsibilities: Profile list, status, login and target resolution
 * (select-databricks-profile rules). It never sees a token.
 *
 * `cliInfo()` reads `cli_status` (#16): the CLI that the R5 launch check verified, and the W-10a
 * plaintext-storage flag of the T7 host check. TODO(#570): the host routes `cli_status`; until
 * then the call is refused with `not_implemented`.
 */
import type { MogId, Profile, ProfileName, ProfileService } from '../../shared/interfaces';
import { call } from '../../shared/ipc';
import { NotImplementedError } from '../errors';

type Status = Awaited<ReturnType<ProfileService['status']>>;
type Target = Awaited<ReturnType<ProfileService['resolveTarget']>>;

const C = 'ProfileService';

export class ProfileServiceStub implements ProfileService {
  async list(): Promise<Profile[]> {
    throw new NotImplementedError(C, 'list', 17); // TODO(#17)
  }
  async status(_profile: ProfileName): Promise<Status> {
    throw new NotImplementedError(C, 'status', 17); // TODO(#17)
  }
  async login(
    _host: string,
    _name: ProfileName,
    _opts?: { clientId?: string },
  ): Promise<{ loginId: string }> {
    throw new NotImplementedError(C, 'login', 162); // TODO(#162)
  }
  async cliInfo(): Promise<{ path: string; version: string; plaintextStorage: boolean }> {
    const { path, version, plaintextStorage } = await call('cli_status', {});
    return { path, version, plaintextStorage };
  }
  home(_mogId: MogId): ProfileName {
    throw new NotImplementedError(C, 'home', 163); // TODO(#163)
  }
  async resolveTarget(_hints: string[], _mogId: MogId): Promise<Target> {
    throw new NotImplementedError(C, 'resolveTarget', 164); // TODO(#164)
  }
}

export function createProfileService(): ProfileService {
  return new ProfileServiceStub();
}
