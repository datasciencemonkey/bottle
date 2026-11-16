// #78 device check (needs:human, needs:mac, #336): MOG-03a, MOG-03b, MOG-18b and UX-19a on a real Mac.
//
// 1. Quit Safari and every other app that uses WebKit (the memory check counts WebKit processes).
// 2. Start Bottle with the demo card cycle (debug build):
//      BOTTLE_DEMO_WINDOWS=1 BOTTLE_DEMO_CARD_CYCLE=1 pnpm tauri dev
//    20 s after the windows show, Bottle hides and shows the demo card 20 times, 1 s apart,
//    then leaves it hidden.
// 3. At once, in a terminal: swift tools/mac/card-check.swift
//    The terminal needs Accessibility (System Settings > Privacy & Security). Do not touch the
//    mouse or keyboard until it prints "done" (about 7 min: the cycle, then 5.5 min hidden).
//
// Output: one JSON line per check, then "done". Exit 0 only if every check passes.
import AppKit
import ApplicationServices
import CoreGraphics

struct Win {
  let pid: pid_t
  let bounds: CGRect
  let layer: Int
}

func pump(_ seconds: Double) {
  RunLoop.current.run(until: Date().addingTimeInterval(seconds))
}

// The demo card window: owner "Bottle", 360 x 240 points. `onScreen` false also finds a hidden one.
func findCard(onScreen: Bool) -> Win? {
  let opts: CGWindowListOption = onScreen ? [.optionOnScreenOnly, .excludeDesktopElements] : [.optionAll]
  guard let list = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] else {
    return nil
  }
  for w in list {
    guard let owner = w[kCGWindowOwnerName as String] as? String,
      owner.lowercased().contains("bottle"),
      let pid = w[kCGWindowOwnerPID as String] as? Int,
      let b = w[kCGWindowBounds as String] as? [String: Double],
      let width = b["Width"], let height = b["Height"],
      Int(width) == 360, Int(height) == 240
    else { continue }
    let rect = CGRect(x: b["X"] ?? 0, y: b["Y"] ?? 0, width: width, height: height)
    return Win(pid: pid_t(pid), bounds: rect, layer: w[kCGWindowLayer as String] as? Int ?? -1)
  }
  return nil
}

// The size of the focused (key) window of an app, through Accessibility.
func focusedWindowSize(_ pid: pid_t) -> CGSize? {
  let app = AXUIElementCreateApplication(pid)
  var win: CFTypeRef?
  guard AXUIElementCopyAttributeValue(app, kAXFocusedWindowAttribute as CFString, &win) == .success,
    let w = win
  else { return nil }
  var value: CFTypeRef?
  guard AXUIElementCopyAttributeValue(w as! AXUIElement, kAXSizeAttribute as CFString, &value) == .success,
    let v = value
  else { return nil }
  var size = CGSize.zero
  AXValueGetValue(v as! AXValue, .cgSize, &size)
  return size
}

// Total resident memory (KB) of the WebKit WebContent processes.
func webContentKB() -> Int {
  let p = Process()
  p.executableURL = URL(fileURLWithPath: "/bin/ps")
  p.arguments = ["-axo", "rss=,comm="]
  let out = Pipe()
  p.standardOutput = out
  try? p.run()
  p.waitUntilExit()
  let text = String(data: out.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
  return text.split(separator: "\n")
    .filter { $0.contains("com.apple.WebKit.WebContent") }
    .compactMap { Int($0.trimmingCharacters(in: .whitespaces).split(separator: " ").first ?? "") }
    .reduce(0, +)
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
guard let first = findCard(onScreen: true) else {
  print("{\"error\":\"no 360x240 Bottle card on screen; start BOTTLE_DEMO_WINDOWS=1 BOTTLE_DEMO_CARD_CYCLE=1 pnpm tauri dev\"}")
  exit(2)
}
var allPass = true
let before = front()
let bottle = NSRunningApplication(processIdentifier: first.pid)

// MOG-03a: NSFloatingWindowLevel is 3 (the CG window layer is the window level).
report("mog_03a_card_floating_level", first.layer == 3, "\"layer\":\(first.layer)")
allPass = allPass && first.layer == 3

// MOG-03b / UX-19a: sample every 50 ms until 20 opens are seen (or 90 s pass).
var opens = 0
var shown = true
var steals = 0
var activations = 0
var keyWhileShown = 0
var shownSamples = 0
let deadline = Date().addingTimeInterval(90)
while opens < 20 && Date() < deadline {
  pump(0.05)
  let now = findCard(onScreen: true) != nil
  if now && !shown { opens += 1 }
  shown = now
  if front() != before { steals += 1 }
  if bottle?.isActive == true { activations += 1 }
  if now {
    shownSamples += 1
    if let s = focusedWindowSize(first.pid), Int(s.width) == 360, Int(s.height) == 240 {
      keyWhileShown += 1
    }
  }
}
let opensPass = opens >= 20 && steals == 0 && activations == 0
report(
  "mog_03b_frontmost_unchanged_in_20_card_opens", opensPass,
  "\"opens\":\(opens),\"front\":\"\(before)\",\"steals\":\(steals),\"bottle_active\":\(activations)")
allPass = allPass && opensPass
// The card is the focused window of Bottle while it shows (a few samples fall between steps).
let keyPass = shownSamples > 0 && Double(keyWhileShown) >= 0.8 * Double(shownSamples)
report(
  "ux_19a_card_is_the_key_window", keyPass,
  "\"key_samples\":\(keyWhileShown),\"shown_samples\":\(shownSamples)")
allPass = allPass && keyPass

// MOG-18b: after the last hide, wait 5 min 30 s; the card window is gone and memory fell.
pump(2)
let hiddenKB = webContentKB()
let stillThere = findCard(onScreen: false) != nil
pump(330)
let gone = findCard(onScreen: false) == nil
let afterKB = webContentKB()
let reapPass = stillThere && gone && afterKB < hiddenKB
report(
  "mog_18b_card_destroyed_and_memory_falls_after_5_min_hidden", reapPass,
  "\"card_before\":\(stillThere),\"card_after\":\(!gone),\"webcontent_kb_before\":\(hiddenKB),\"webcontent_kb_after\":\(afterKB)")
allPass = allPass && reapPass

print("done")
exit(allPass ? 0 : 1)
