// SP-01 WKWebView + NSPanel probe (NOT Tauri). Tauri uses WKWebView on macOS
// (wry), so this measures the same platform layer when the Tauri build cannot
// run. Build: swiftc -O -o out/probe probe-swift/Probe.swift
//
// Env: SP01_WEB (web dir), SP01_RIV, SP01_RUNTIME (webgl2|canvas),
//      SP01_MODE (animating|idle), SP01_LOG (jsonl), SP01_X, SP01_Y,
//      SP01_KIND (panel|window), SP01_BG (checker|none),
//      SP01_SNAP_DIR (write WKWebView RGBA snapshots), SP01_SNAP_AT (s, csv),
//      SP01_FLASH_DIR (snapshots every 30 ms for the first 1.5 s),
//      SP01_QUIT_AFTER (s)
import AppKit
import WebKit

let env = ProcessInfo.processInfo.environment
func e(_ k: String, _ d: String) -> String { env[k] ?? d }
let logPath = e("SP01_LOG", "/tmp/sp01-events.jsonl")
let t0 = Date().timeIntervalSince1970 * 1000

func nowMs() -> Double { Date().timeIntervalSince1970 * 1000 }

func logLine(_ obj: [String: Any]) {
    var o = obj
    o["host_t"] = nowMs()
    guard let d = try? JSONSerialization.data(withJSONObject: o),
          var s = String(data: d, encoding: .utf8) else { return }
    s += "\n"
    if let h = FileHandle(forWritingAtPath: logPath) {
        h.seekToEndOfFile(); h.write(s.data(using: .utf8)!); h.closeFile()
    } else {
        FileManager.default.createFile(atPath: logPath, contents: s.data(using: .utf8))
    }
}

final class Panel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

final class Win: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

let csp = "default-src 'self'; connect-src ipc: http://ipc.localhost; script-src 'self' 'wasm-unsafe-eval'"

// Fallback "own fix" B: a native tracking area (always active) whose owner forwards
// mouse moves into the page as synthetic DOM PointerEvents (kind "pointer", fwd=true).
final class Forwarder: NSObject {
    weak var web: WKWebView?
    var sent = 0
    func fwd(_ ev: NSEvent, _ type: String) {
        guard let w = web else { return }
        let p = w.convert(ev.locationInWindow, from: nil)
        let y = w.bounds.height - p.y
        sent += 1
        logLine(["kind": "native", "type": type])
        w.evaluateJavaScript("window.sp01Forward && window.sp01Forward('\(type)', \(p.x), \(y))")
    }
    @objc func mouseMoved(with ev: NSEvent) { fwd(ev, "pointermove") }
    @objc func mouseEntered(with ev: NSEvent) { fwd(ev, "pointerenter") }
    @objc func mouseExited(with ev: NSEvent) { fwd(ev, "pointerleave") }
}
let forwarder = Forwarder()

final class Scheme: NSObject, WKURLSchemeHandler {
    let root: URL
    init(root: URL) { self.root = root }
    func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        let path = task.request.url!.path
        let file = root.appendingPathComponent(path == "/" ? "index.html" : String(path.dropFirst()))
        guard let data = try? Data(contentsOf: file) else {
            task.didFailWithError(NSError(domain: "sp01", code: 404)); return
        }
        let mime: String
        switch file.pathExtension {
        case "html": mime = "text/html"
        case "js": mime = "text/javascript"
        case "css": mime = "text/css"
        case "wasm": mime = "application/wasm"
        default: mime = "application/octet-stream"
        }
        let resp = HTTPURLResponse(url: task.request.url!, statusCode: 200, httpVersion: "HTTP/1.1",
                                   headerFields: ["Content-Type": mime, "Content-Security-Policy": csp])!
        task.didReceive(resp); task.didReceive(data); task.didFinish()
    }
    func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {}
}

final class Bridge: NSObject, WKScriptMessageHandlerWithReply {
    let web: URL
    init(web: URL) { self.web = web }
    func userContentController(_ c: WKUserContentController, didReceive m: WKScriptMessage,
                               replyHandler: @escaping (Any?, String?) -> Void) {
        guard let body = m.body as? [String: Any], let cmd = body["cmd"] as? String else {
            replyHandler(nil, "bad message"); return
        }
        let arg = body["arg"] as? [String: Any] ?? [:]
        switch cmd {
        case "sp01_config":
            replyHandler(["runtime": e("SP01_RUNTIME", "webgl2"), "mode": e("SP01_MODE", "animating")], nil)
        case "sp01_blob":
            let name = arg["name"] as? String ?? ""
            let url: URL
            if name == "riv" { url = URL(fileURLWithPath: e("SP01_RIV", "")) }
            else if name.hasPrefix("wasm-") {
                url = web.appendingPathComponent("vendor/\(name.dropFirst(5))/rive.wasm")
            } else { replyHandler(nil, "unknown blob"); return }
            guard let d = try? Data(contentsOf: url) else { replyHandler(nil, "read failed \(url.path)"); return }
            replyHandler(d.base64EncodedString(), nil)
        case "sp01_log":
            if let line = arg["line"] as? String,
               let d = line.data(using: .utf8),
               let o = try? JSONSerialization.jsonObject(with: d) as? [String: Any] {
                logLine(o)
            }
            replyHandler(nil, nil)
        default:
            replyHandler(nil, "unknown cmd")
        }
    }
}

func savePNG(_ img: NSImage, _ path: String) {
    guard let tiff = img.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff),
          let png = rep.representation(using: .png, properties: [:]) else { return }
    try? png.write(to: URL(fileURLWithPath: path))
}

final class App: NSObject, NSApplicationDelegate {
    var win: NSWindow!
    var bg: NSWindow?
    var webView: WKWebView!
    var bridge: Bridge!
    var scheme: Scheme!

    func applicationDidFinishLaunching(_ n: Notification) {
        let front0 = NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "?"
        let web = URL(fileURLWithPath: e("SP01_WEB", "web"))
        let x = Double(e("SP01_X", "1468"))!, y = Double(e("SP01_Y", "20"))!
        let size = Double(e("SP01_SIZE", "240"))!
        let rect = NSRect(x: x, y: y, width: size, height: size)

        if e("SP01_BG", "none") == "checker" {
            // Known background: a normal-level borderless window with an 8 px checker
            // (40/200 grey) behind the mog, slightly larger than it.
            let b = NSWindow(contentRect: rect.insetBy(dx: -40, dy: -40), styleMask: [.borderless],
                             backing: .buffered, defer: false)
            let v = NSImageView(frame: NSRect(origin: .zero, size: b.frame.size))
            v.image = NSImage(contentsOfFile: e("SP01_BG_PNG", "/tmp/sp01-checker.png"))
            v.imageScaling = .scaleAxesIndependently
            b.contentView = v
            b.level = .normal
            b.orderFrontRegardless()
            bg = b
        }

        let kind = e("SP01_KIND", "panel")
        if kind == "panel" {
            win = Panel(contentRect: rect, styleMask: [.borderless, .nonactivatingPanel],
                        backing: .buffered, defer: false)
            (win as! NSPanel).isFloatingPanel = true
            (win as! NSPanel).becomesKeyOnlyIfNeeded = true
        } else {
            win = Win(contentRect: rect, styleMask: [.borderless], backing: .buffered, defer: false)
        }
        win.isOpaque = false
        win.backgroundColor = .clear
        win.hasShadow = false
        win.level = .floating
        win.hidesOnDeactivate = false
        win.acceptsMouseMovedEvents = true
        win.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]

        let cfg = WKWebViewConfiguration()
        scheme = Scheme(root: web)
        bridge = Bridge(web: web)
        cfg.setURLSchemeHandler(scheme, forURLScheme: "sp01")
        cfg.userContentController.addScriptMessageHandler(bridge, contentWorld: .page, name: "sp01")
        webView = WKWebView(frame: NSRect(origin: .zero, size: rect.size), configuration: cfg)
        webView.setValue(false, forKey: "drawsBackground")
        webView.underPageBackgroundColor = .clear
        win.contentView = webView
        if e("SP01_TRACK", "0") == "2" {
            forwarder.web = webView
            let area = NSTrackingArea(rect: .zero,
                                      options: [.activeAlways, .mouseMoved, .mouseEnteredAndExited, .inVisibleRect],
                                      owner: forwarder, userInfo: nil)
            webView.addTrackingArea(area)
        }
        if e("SP01_TRACK", "0") == "1" {
            // Fallback "own fix": an always-active tracking area owned by the WKWebView,
            // so AppKit sends mouseMoved:/mouseEntered: to it while the window is not key.
            let area = NSTrackingArea(rect: .zero,
                                      options: [.activeAlways, .mouseMoved, .mouseEnteredAndExited, .inVisibleRect],
                                      owner: webView, userInfo: nil)
            webView.addTrackingArea(area)
        }
        webView.load(URLRequest(url: URL(string: "sp01://app/index.html")!))
        win.orderFrontRegardless()

        logLine(["kind": "info", "msg": "shown", "kind_window": kind, "front_before": front0,
                 "pid": ProcessInfo.processInfo.processIdentifier,
                 "rect": [x, y, size, size]])

        // Non-activating check: frontmost app, key window, NSApp.isActive.
        for s in [0.5, 2.0, 5.0] {
            DispatchQueue.main.asyncAfter(deadline: .now() + s) {
                logLine(["kind": "focus", "after_s": s,
                         "front": NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "?",
                         "front_is_probe": NSWorkspace.shared.frontmostApplication?.processIdentifier
                             == ProcessInfo.processInfo.processIdentifier,
                         "app_active": NSApp.isActive, "is_key": self.win.isKeyWindow,
                         "is_main": self.win.isMainWindow, "key_window_nil": NSApp.keyWindow == nil])
            }
        }

        if let dir = env["SP01_FLASH_DIR"] {
            for i in 0..<50 {
                DispatchQueue.main.asyncAfter(deadline: .now() + Double(i) * 0.03) {
                    let t = nowMs() - t0
                    self.webView.takeSnapshot(with: nil) { img, _ in
                        if let img { savePNG(img, "\(dir)/f_\(Int(t)).png") }
                    }
                }
            }
        }
        if let dir = env["SP01_SNAP_DIR"] {
            for s in (e("SP01_SNAP_AT", "3").split(separator: ",").compactMap { Double($0) }) {
                DispatchQueue.main.asyncAfter(deadline: .now() + s) {
                    self.webView.takeSnapshot(with: nil) { img, err in
                        if let img { savePNG(img, "\(dir)/snap_\(Int(s * 1000)).png") }
                        logLine(["kind": "info", "msg": "snapshot", "at_s": s, "ok": img != nil,
                                 "err": err.map { "\($0)" } ?? ""])
                    }
                }
            }
        }
        if let q = env["SP01_QUIT_AFTER"], let s = Double(q) {
            DispatchQueue.main.asyncAfter(deadline: .now() + s) { NSApp.terminate(nil) }
        }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(e("SP01_POLICY", "accessory") == "regular" ? .regular : .accessory)
let delegate = App()
app.delegate = delegate
app.run()
