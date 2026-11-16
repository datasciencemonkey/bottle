/** Hub MOG list shapes (#26). Red stub. */
export const HUB_MOGS_EVENT = '';
export const HUB_SCOPE = '';
export interface HubMogRow { mogId: string; name: string; home: string; persona: string; running: boolean }
export interface HubMogList { mogs: HubMogRow[]; unreadable: string[]; runningCount: number; maxRunning: number; notice?: { kind: 'error' | 'info'; text: string } }
export function asHubMogList(_p: unknown): HubMogList | undefined { throw new Error('todo #26'); }
export function asMogAction(_p: unknown): unknown { throw new Error('todo #26'); }
