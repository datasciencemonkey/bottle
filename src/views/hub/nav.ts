/**
 * The hub's navigation areas (ui-1, #97, #103). `HubView` names every area the sidebar can ever
 * show; `NAV_ITEMS` is the single source of which ones render right now. 'recordings'
 * (rec-lead's area, ADR 0103, #595) joined once its own PR landed (merge of origin/main into
 * ui-1, review of #602); 'workspaces' (the CLI path/version move, a later PR) stays out of
 * `NAV_ITEMS` until its own PR lands, so the type is ready but the sidebar does not show an
 * empty area. 'memory' is the local memory area (local memory plan 9.1, ADR 0112).
 */
import {
  BrainIcon,
  DiscAlbumIcon,
  HouseIcon,
  MessageSquareIcon,
  SettingsIcon,
  SparklesIcon,
  type LucideIcon,
} from 'lucide-react';

export type HubView =
  'home' | 'conversations' | 'memory' | 'mogs' | 'workspaces' | 'recordings' | 'settings';

export interface NavItem {
  readonly id: HubView;
  readonly label: string;
  readonly icon: LucideIcon;
}

export const NAV_ITEMS: readonly NavItem[] = [
  { id: 'home', label: 'Home', icon: HouseIcon },
  { id: 'mogs', label: 'MOGs', icon: SparklesIcon },
  { id: 'conversations', label: 'Conversations', icon: MessageSquareIcon },
  { id: 'recordings', label: 'Recordings', icon: DiscAlbumIcon },
  { id: 'memory', label: 'Memory', icon: BrainIcon },
  { id: 'settings', label: 'Settings', icon: SettingsIcon },
];

/**
 * The area the hub opens on: Home, the calm launch screen. The setup steps sit above every area,
 * so first-run users are never blocked (#103). An open for a purpose (Open in Bottle, Change
 * workspace, the talk key with no MOG, Choose where to save) switches to its own area.
 */
export const DEFAULT_HUB_VIEW: HubView = 'home';
