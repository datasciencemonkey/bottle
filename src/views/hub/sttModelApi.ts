/**
 * The real `SttModelApi` of `SttModelStep` (#413, #45, #529, ADR 0104): the hub's own
 * `stt_model_*` commands. The gateway lets only the hub call them (like `view_notice_subscribe`,
 * ADR 0091), so this file is meant for a hub view only.
 */
import { Channel, call } from '../../shared/ipc';
import type { SttModelApi, SttProgressView, SttSource } from './SttModelStep';

export const sttModelApi: SttModelApi = {
  status: () => call('stt_model_status', {}),
  setConsent: (consent) => call('stt_model_consent', { consent }),
  install: (source: SttSource, onProgress: (p: SttProgressView) => void) => {
    const channel = new Channel<SttProgressView>();
    channel.onmessage = onProgress;
    return call('stt_model_install', { source, onProgress: channel });
  },
  verify: () => call('stt_model_verify', {}),
  cancel: () => call('stt_model_cancel', {}).then((r) => r.cancelled),
};
