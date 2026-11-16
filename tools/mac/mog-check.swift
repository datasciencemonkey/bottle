// #11 device check (needs:human, needs:mac): MOG-01b, MOG-01c and MOG-02 on a real Mac.
//
// Run Bottle with the demo MOG first (debug build: BOTTLE_DEMO_WINDOWS=1 pnpm tauri dev).
// Then run: swift tools/mac/mog-check.swift
// The terminal needs Accessibility (System Settings > Privacy & Security) to post mouse events.
// The script moves the real cursor and clicks the MOG window 100 times. Do not touch the
// mouse until it prints "done". It puts the cursor back at the end.
//
// Output: one JSON line per check, then "done". Exit 0 only if every check passes.
import AppKit
import CoreGraphics

struct Win {
  let pid: pid_t
  let bounds: CGRect
  let layer: Int
}

func pump(_ seconds: Double) {
  RunLoop.current.run(until: Date().addingTimeInterval(seconds))
}

// The demo MOG window: owner "Bottle", 160 x 160 points.
func findMog() -> Win? {
  let opts: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
  guard let list = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] else {
    return nil
  }
  for w in list {
    guard let owner = w[kCGWindowOwnerName as String] as? String,
      owner.lowercased().contains("bottle"),
      let pid = w[kCGWindowOwnerPID as String] as? Int,
      let b = w[kCGWindowBounds as String] as? [String: Double],
      let width = b["Width"], let height = b["Height"],
      Int(width) == 160, Int(height) == 160
    else { continue }
    let rect = CGRect(x: b["X"] ?? 0, y: b["Y"] ?? 0, width: width, height: height)
    return Win(pid: pid_t(pid), bounds: rect, layer: w[kCGWindowLayer as String] as? Int ?? -1)
  }
  return nil
}

func post(_ type: CGEventType, _ p: CGPoint) {
  CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: p, mouseButton: .left)?
    .post(tap: .cghidEventTap)
}

func front() -> String {
  NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "none"
}

func report(_ check: String, _ pass: Bool, _ detail: String) {
  print("{\"check\":\"\(check)\",\"pass\":\(pass),\(detail)}")
}

guard AXIsProcessTrusted() else {
  print("{\"error\":\"the terminal needs Accessibility permission\"}")
  exit(2)
}
guard let mog = findMog() else {
  print("{\"error\":\"no 160x160 Bottle window on screen; start BOTTLE_DEMO_WINDOWS=1 pnpm tauri dev\"}")
  exit(2)
}
let restore = CGEvent(source: nil)?.location ?? .zero
var allPass = true

// MOG-02: NSStatusWindowLevel is 25 (the CG window layer is the window level).
report("mog_02_status_level", mog.layer == 25, "\"layer\":\(mog.layer)")
allPass = allPass && mog.layer == 25

// MOG-01b: 0 focus steals in 100 clicks.
let before = front()
let bottle = NSRunningApplication(processIdentifier: mog.pid)
let center = CGPoint(x: mog.bounds.midX, y: mog.bounds.midY)
var steals = 0
var activations = 0
for _ in 0..<100 {
  post(.leftMouseDown, center)
  post(.leftMouseUp, center)
  pump(0.08)
  if front() != before { steals += 1 }
  if bottle?.isActive == true { activations += 1 }
}
let clicksPass = steals == 0 && activations == 0
report(
  "mog_01b_focus_steals_in_100_clicks", clicksPass,
  "\"front_before\":\"\(before)\",\"steals\":\(steals),\"bottle_active\":\(activations)")
allPass = allPass && clicksPass

// MOG-01c: a drag of 80 points moves the MOG and takes no focus.
post(.leftMouseDown, center)
pump(0.05)
for step in 1...16 {
  post(.leftMouseDragged, CGPoint(x: center.x + Double(step) * 5, y: center.y))
  pump(0.02)
}
post(.leftMouseUp, CGPoint(x: center.x + 80, y: center.y))
pump(0.5)
let moved = (findMog()?.bounds.minX ?? mog.bounds.minX) - mog.bounds.minX
let dragPass = moved >= 70 && front() == before && bottle?.isActive != true
report(
  "mog_01c_drag_moves_the_mog", dragPass,
  "\"moved_pt\":\(moved),\"front_after\":\"\(front())\"")
allPass = allPass && dragPass

post(.mouseMoved, restore)
print("done")
exit(allPass ? 0 : 1)
