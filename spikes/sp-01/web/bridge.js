// SP-01 host bridge. The page runs in Tauri (window.__TAURI__) or in the
// WKWebView proxy harness (window.webkit.messageHandlers). Bytes cross the
// bridge as base64 so that connect-src stays "ipc: http://ipc.localhost" (ARC-4).
(function () {
  function b64ToBuf(b64) {
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out.buffer;
  }
  const tauri = window.__TAURI__ && window.__TAURI__.core;
  const wk = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.sp01;
  async function call(cmd, arg) {
    if (tauri) return tauri.invoke(cmd, arg || {});
    if (wk) return wk.postMessage({ cmd: cmd, arg: arg || {} });
    throw new Error("no host bridge");
  }
  window.sp01 = {
    host: tauri ? "tauri" : wk ? "wkwebview" : "none",
    config: () => call("sp01_config"),
    blob: async (name) => b64ToBuf(await call("sp01_blob", { name: name })),
    log: (rec) => {
      try { call("sp01_log", { line: JSON.stringify(rec) }); } catch (_) { /* ignore */ }
    },
  };
})();
