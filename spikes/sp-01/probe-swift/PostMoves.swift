// SP-01 hover helper: post CGEvent mouse moves into a window rect while
// another app stays frontmost. Prints one JSON line with what it sent.
// Build: swiftc -O -o out/post_moves probe-swift/PostMoves.swift
// Usage: post_moves <x> <y> <w> <h> [count=200] [interval_ms=10] [tap=hid|session|pid:<pid>]
// x/y are top-left screen points (CG coordinates, origin top-left of main display).
import AppKit
import ApplicationServices

let a = CommandLine.arguments
guard a.count >= 5, let x = Double(a[1]), let y = Double(a[2]),
      let w = Double(a[3]), let h = Double(a[4]) else {
    FileHandle.standardError.write("usage: post_moves x y w h [count] [interval_ms] [tap]\n".data(using: .utf8)!)
    exit(2)
}
let count = a.count > 5 ? Int(a[5]) ?? 200 : 200
let intervalMs = a.count > 6 ? Double(a[6]) ?? 10 : 10
let tap = a.count > 7 ? a[7] : "hid"

let trusted = AXIsProcessTrusted()
let front0 = NSWorkspace.shared.frontmostApplication?.localizedName ?? "?"
let start = CGEvent(source: nil)?.location ?? .zero
let src = CGEventSource(stateID: .hidSystemState)
var posted = 0
var nilEvents = 0
let t0 = Date().timeIntervalSince1970 * 1000
for i in 0..<count {
    // Zig-zag path inside the rect, 20 px margin.
    let fx = Double(i % 40) / 39.0
    let fy = Double((i / 40) % 5) / 4.0
    let p = CGPoint(x: x + 20 + fx * (w - 40), y: y + 20 + fy * (h - 40))
    guard let ev = CGEvent(mouseEventSource: src, mouseType: .mouseMoved, mouseCursorPosition: p,
                           mouseButton: .left) else { nilEvents += 1; continue }
    if tap == "hid" { ev.post(tap: .cghidEventTap) }
    else if tap == "session" { ev.post(tap: .cgSessionEventTap) }
    else if tap.hasPrefix("pid:"), let pid = Int32(tap.dropFirst(4)) { ev.postToPid(pid) }
    posted += 1
    usleep(useconds_t(intervalMs * 1000))
}
let t1 = Date().timeIntervalSince1970 * 1000
let end = CGEvent(source: nil)?.location ?? .zero
// Put the cursor back where it was.
if tap == "hid" || tap == "session" {
    CGEvent(mouseEventSource: src, mouseType: .mouseMoved, mouseCursorPosition: start, mouseButton: .left)?
        .post(tap: .cghidEventTap)
}
let front1 = NSWorkspace.shared.frontmostApplication?.localizedName ?? "?"
let out: [String: Any] = [
    "kind": "sent", "tap": tap, "sent": posted, "nil_events": nilEvents, "ax_trusted": trusted,
    "t_start": t0, "t_end": t1, "cursor_after_last": [end.x, end.y],
    "front_before": front0, "front_after": front1,
]
let d = try! JSONSerialization.data(withJSONObject: out)
print(String(data: d, encoding: .utf8)!)
