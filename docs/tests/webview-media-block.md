# WebView media block: device check (#387, CAP-02b, AT-17)

Unit and config tests cover the deny handler and the config (ADR 0037). These steps check the real
WebView on a device. A person runs them. Time: about 20 minutes for each platform.

The steps call `getUserMedia` only to check that it fails. They never record or keep audio.
If a step shows a microphone prompt, or the microphone indicator turns on, stop and report a fail.

## Before you start

- Use a debug build, so you can open the Web Inspector or DevTools of a WebView.
- Build with the bundled pages (this build sends the `Permissions-Policy` header):
  `pnpm tauri build --debug`. Then open the built app from `target/debug/bundle/`.
- In each WebView console, you will paste this line (the "probe"):

  ```js
  navigator.mediaDevices?.getUserMedia({ audio: true, video: false }).then((s) => { s.getTracks().forEach((t) => t.stop()); console.log('GRANTED'); }, (e) => console.log('DENIED', e.name)) ?? console.log('DENIED no mediaDevices')
  ```

## macOS (needs:mac)

| # | Do this | Expected result | Requirement |
|---|---|---|---|
| M1 | Open System Settings > Privacy & Security > Microphone. Note if Bottle is in the list and if it is on. | You know the start state. | CAP-02b |
| M2 | Start the app with `BOTTLE_DEMO_WINDOWS=1`. Right-click the demo MOG window and select Inspect Element. In the console, paste the probe. | The console prints `DENIED NotAllowedError` (or `DENIED no mediaDevices`). No macOS prompt shows. The orange microphone dot in the menu bar does not show. | CAP-02b, AT-17 |
| M3 | Do step M2 in the hub window and in the demo card window. | The same result as M2. | CAP-02b |
| M4 | Open the brain window inspector (Safari > Develop > Bottle > brain). Paste the probe. | The same result as M2. | CAP-02b |
| M5 | If Bottle has the microphone grant from M1 (or from a core capture test), do M2 again. | Still `DENIED`. The grant of the app does not reach the WebView. | CAP-02b, AT-17 |
| M6 | Run the probe with `{ video: true }` in place of `{ audio: true, video: false }`. | `DENIED`. No camera prompt shows and the camera light stays off. | CAP-02b |
| M7 | Run `pnpm tauri dev` with `BOTTLE_DEMO_WINDOWS=1` and do M2. | `DENIED` (the deny handler works in dev too). | CAP-02b |

## Windows (needs:windows)

| # | Do this | Expected result | Requirement |
|---|---|---|---|
| W1 | Start the app with `BOTTLE_DEMO_WINDOWS=1`. In the demo MOG window, press F12 (or right-click > Inspect). In the console, paste the probe. | The console prints `DENIED NotAllowedError`. No WebView2 permission prompt and no Windows prompt shows. The microphone icon does not show in the taskbar. | CAP-02b, AT-17 |
| W2 | In the same console, run `document.featurePolicy.allowsFeature('microphone')`. | `false` (the `Permissions-Policy` header is in force). | CAP-02b |
| W3 | Do W1 in the hub window, the demo card window and the brain window. | The same result as W1. | CAP-02b |
| W4 | Run the probe with `{ video: true }`. | `DENIED`. No camera prompt shows. | CAP-02b |

## Report

Post the result table on the device-check issue. For each step, give pass or fail and the console
text. Do not attach audio or a screen recording with sound.
