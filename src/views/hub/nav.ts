/**
 * The hub's navigation areas (ui-1, #97, #103). `HubView` names every area the sidebar can ever
 * show; `NAV_ITEMS` is the single source of which ones render right now. 'recordings'
 * (rec-lead's area, ADR 0103, #595) joined once its own PR landed (merge of origin/main into
 * ui-1, review of #602); 'workspaces' (the CLI path/version move, a later PR) stays out of
 * `NAV_ITEMS` until its own PR lands, so the type is ready but the sidebar does not show an
 * empty area.
 */
import {
  DiscAlbumIcon,
  MessageSquareIcon,
  SettingsIcon,
  SparklesIcon,
  type LucideIcon,
} from 'lucide-react';

export type HubView = 'conversations' | 'mogs' | 'workspaces' | 'recordings' | 'settings';

export interface NavItem {
  readonly id: HubView;
  readonly label: string;
  readonly icon: LucideIcon;
}

export const NAV_ITEMS: readonly NavItem[] = [
  { id: 'conversations', label: 'Conversations', icon: MessageSquareIcon },
  { id: 'mogs', label: 'MOGs', icon: SparklesIcon },
  { id: 'recordings', label: 'Recordings', icon: DiscAlbumIcon },
  { id: 'settings', label: 'Settings', icon: SettingsIcon },
];

/** The area the hub opens on: the first item, where the first-run setup steps live (#103). */
export const DEFAULT_HUB_VIEW: HubView = NAV_ITEMS[0]?.id ?? 'conversations';
