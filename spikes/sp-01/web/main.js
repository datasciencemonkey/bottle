// SP-01 page: load one Rive runtime, render the .riv, animate mood/level when
// present, and log pointer events with timestamps to the host.
(async function () {
  const now = () => performance.timeOrigin + performance.now();
  const log = (rec) => window.sp01.log(Object.assign({ t: now() }, rec));
  for (const type of ["pointermove", "pointerenter", "mouseover"]) {
    document.addEventListener(type, (e) => {
      log({ kind: "pointer", type: type, x: e.clientX, y: e.clientY });
    }, { capture: true, passive: true });
  }
  window.addEventListener("error", (e) => log({ kind: "error", msg: String(e.message) }));

  const cfg = await window.sp01.config();
  log({ kind: "info", msg: "config", cfg: cfg, host: window.sp01.host });
  const runtime = cfg.runtime === "canvas" ? "canvas" : "webgl2";
  await new Promise((ok, bad) => {
    const s = document.createElement("script");
    s.src = "vendor/" + runtime + "/rive.js";
    s.onload = ok;
    s.onerror = () => bad(new Error("rive.js load failed"));
    document.head.appendChild(s);
  });
  const rive = window.rive;
  rive.RuntimeLoader.setWasmBinary(await window.sp01.blob("wasm-" + runtime));
  const buffer = await window.sp01.blob("riv");
  const canvas = document.getElementById("mog");

  const r = new rive.Rive({
    buffer: buffer,
    canvas: canvas,
    autoplay: true,
    autoBind: true,
    enableRiveAssetCDN: false,
    layout: new rive.Layout({ fit: rive.Fit.Contain, alignment: rive.Alignment.Center }),
    onLoad: () => {
      r.resizeDrawingSurfaceToCanvas();
      const sms = r.stateMachineNames || [];
      const inputs = {};
      for (const sm of sms) {
        try { inputs[sm] = (r.stateMachineInputs(sm) || []).map((i) => ({ name: i.name, type: i.type })); }
        catch (_) { inputs[sm] = "error"; }
      }
      if (sms.length) r.play(sms[0]);
      const vmi = r.viewModelInstance;
      const props = vmi ? (vmi.properties || []).map((p) => ({ name: p.name, type: p.type })) : [];
      log({ kind: "info", msg: "loaded", runtime: runtime, artboards: r.activeArtboard,
            stateMachines: sms, animations: r.animationNames, inputs: inputs, viewModelProps: props });
      const moods = ["idle", "listening", "processing", "answering", "success", "error", "sleeping"];
      const mood = vmi && vmi.enum ? vmi.enum("mood") : null;
      const level = vmi && vmi.number ? vmi.number("level") : null;
      if (cfg.mode === "idle") {
        setTimeout(() => { r.pause(); log({ kind: "info", msg: "paused" }); }, 500);
      } else if (!mood && !level) {
        // Stand-in asset without mood/level: loop its timeline animations instead.
        const names = r.animationNames || [];
        r.stop();
        r.play(names);
        log({ kind: "info", msg: "playing animations", names: names });
      } else {
        let i = 0;
        setInterval(() => {
          i++;
          if (mood) mood.value = moods[i % moods.length];
          if (level) level.value = (i * 13) % 100;
          log({ kind: "info", msg: "anim", mood: mood ? mood.value : null, level: level ? level.value : null });
        }, 1500);
      }
      log({ kind: "info", msg: "ready", hasMood: !!mood, hasLevel: !!level });
    },
    onLoadError: (e) => log({ kind: "error", msg: "rive load error " + String(e && e.data) }),
  });
})().catch((e) => window.sp01.log({ kind: "error", msg: String(e && e.stack || e) }));

// Host forwarder for the fallback (native tracking area -> synthetic DOM events).
window.sp01Forward = function (type, x, y) {
  const target = document.elementFromPoint(x, y) || document.body;
  const init = { bubbles: type !== "pointerenter", clientX: x, clientY: y, pointerType: "mouse" };
  target.dispatchEvent(new PointerEvent(type, init));
  if (type === "pointerenter") target.dispatchEvent(new MouseEvent("mouseover", init));
};

// Frame counter: logs rAF frames per 5 s so a run shows whether rendering is live.
(function () {
  let n = 0;
  const tick = () => { n++; requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
  setInterval(() => { window.sp01.log({ kind: "info", msg: "raf", frames_5s: n, t: Date.now() }); n = 0; }, 5000);
})();
