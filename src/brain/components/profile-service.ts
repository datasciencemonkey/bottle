/**
 * T7 ProfileService (4.3, hidden TS brain window). Implements the 4.7 `ProfileService`.
 *
 * Responsibilities: Profile list, status, login and target resolution
 * (select-databricks-profile rules). It never sees a token.
 */
import type { MogId, Profile, ProfileName, ProfileService } from '../../shared/interfaces';
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
    throw new NotImplementedError(C, 'cliInfo', 16); // TODO(#16)
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
