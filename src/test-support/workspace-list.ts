/** A `workspace_list` reply for brain tests (ADR 0117). */
import type { WorkspaceList, WorkspaceRow } from '../shared/ipc';

export interface FakeRow extends Partial<WorkspaceRow> {
  profile: string;
}

export function workspaceRow(r: FakeRow): WorkspaceRow {
  return {
    host: `https://${r.profile}.example.test`,
    mogIds: [],
    available: false,
    isHome: false,
    signIn: 'signed_in',
    ...r,
  };
}

export function workspaceList(
  home: string | null,
  rows: FakeRow[] = home === null ? [] : [{ profile: home }],
  over: Partial<WorkspaceList> = {},
): WorkspaceList {
  return {
    home,
    lampMogId: null,
    workspaces: rows.map((r) =>
      workspaceRow({ ...r, ...(r.profile === home ? { isHome: true, available: true } : {}) }),
    ),
    agent: {
      genieCallBudget: 10,
      endpoint: 'system.ai.claude-haiku-5-5',
      cleanDictation: true,
      cloudDictation: false,
    },
    ...over,
  };
}
