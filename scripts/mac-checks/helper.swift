// helper.swift: the macOS side of mac-checks (run.py compiles it with swiftc into the results
// folder). One subcommand per call; coordinates are global screen points, top-left origin (the
// same space as CGWindowList's bounds). Needs Accessibility (key, type, mouse, ax) and Screen
// Recording (screencapture of wubble's windows; `wins` titles) for the terminal that runs it.
//
//   trust                          prints "accessibility 0|1" and "screen 0|1"
//   wins PID                       wubble's on-screen windows: "id x y w h layer" per line
//   activate PID                   bring the app to the front
//   key CHORD                      e.g. cmd+shift+n, escape, return, down, cmd+backslash
//   type TEXT                      types TEXT as Unicode keystrokes
//   move X Y | click X Y | rclick X Y | down X Y | drag X Y | up X Y
//   ax PID minimize|restore|size W H|pos X Y|frame   the app's focused (or first) window
//   px PNG X Y                     the pixel at image (X, Y) as RRGGBB
//   stats PNG X Y W H [RRGGBB TOL] a region of the image (pixels): black (or transparent), match,
//                                  mean, luma sd
//   bbox PNG RRGGBB TOL            bounding box (pixels) of pixels within TOL of the colour
//   diff A.png B.png X Y W H       fraction of pixels in the region differing by > 24
//   rusage PID...                  per process: "pid cpu_ns instructions cycles energy_nj"
//                                  (proc_pid_rusage; a process it may not read is left out)
import AppKit
import ApplicationServices
import CoreGraphics
import Foundation
import ImageIO

let args = Array(CommandLine.arguments.dropFirst())
func die(_ s: String) -> Never { FileHandle.standardError.write((s + "\n").data(using: .utf8)!); exit(2) }
func num(_ i: Int) -> Double { guard i < args.count, let v = Double(args[i]) else { die("bad number at \(i)") }; return v }

// MARK: keys
let keyCodes: [String: CGKeyCode] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9, "b": 11, "q": 12,
    "w": 13, "e": 14, "r": 15, "y": 16, "t": 17, "1": 18, "2": 19, "3": 20, "4": 21, "6": 22, "5": 23,
    "equal": 24, "9": 25, "7": 26, "minus": 27, "8": 28, "0": 29, "o": 31, "u": 32, "i": 34, "p": 35,
    "return": 36, "l": 37, "j": 38, "k": 40, "backslash": 42, "comma": 43, "n": 45, "m": 46,
    "period": 47, "tab": 48, "space": 49, "delete": 51, "escape": 53, "left": 123, "right": 124,
    "down": 125, "up": 126, "f11": 103,
]
func postKey(_ chord: String) {
    var flags: CGEventFlags = []
    var key: CGKeyCode?
    for part in chord.lowercased().split(separator: "+").map(String.init) {
        switch part {
        case "cmd": flags.insert(.maskCommand)
        case "shift": flags.insert(.maskShift)
        case "alt", "opt": flags.insert(.maskAlternate)
        case "ctrl": flags.insert(.maskControl)
        default:
            guard let k = keyCodes[part] else { die("unknown key \(part)") }
            key = k
        }
    }
    guard let code = key else { die("no key in \(chord)") }
    let src = CGEventSource(stateID: .hidSystemState)
    for down in [true, false] {
        let e = CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: down)!
        e.flags = flags
        e.post(tap: .cghidEventTap)
        usleep(12_000)
    }
}
func typeText(_ s: String) {
    let src = CGEventSource(stateID: .hidSystemState)
    for ch in s.utf16 {
        for down in [true, false] {
            let e = CGEvent(keyboardEventSource: src, virtualKey: 0, keyDown: down)!
            var c = ch
            e.keyboardSetUnicodeString(stringLength: 1, unicodeString: &c)
            e.post(tap: .cghidEventTap)
        }
        usleep(8_000)
    }
}

// MARK: mouse
func mouse(_ type: CGEventType, _ x: Double, _ y: Double, _ button: CGMouseButton = .left) {
    let e = CGEvent(mouseEventSource: CGEventSource(stateID: .hidSystemState), mouseType: type,
                    mouseCursorPosition: CGPoint(x: x, y: y), mouseButton: button)!
    e.post(tap: .cghidEventTap)
}

// MARK: windows
func windows(_ pid: pid_t) -> [[String: Any]] {
    let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
        as? [[String: Any]] ?? []
    return list.filter { ($0[kCGWindowOwnerPID as String] as? pid_t) == pid }
}
func axWindow(_ pid: pid_t) -> AXUIElement {
    let app = AXUIElementCreateApplication(pid)
    var v: CFTypeRef?
    if AXUIElementCopyAttributeValue(app, kAXFocusedWindowAttribute as CFString, &v) == .success, let w = v {
        return (w as! AXUIElement)
    }
    if AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &v) == .success,
       let ws = v as? [AXUIElement], let w = ws.first { return w }
    die("no AX window for \(pid) (Accessibility permission?)")
}
func axAllWindows(_ pid: pid_t) -> [AXUIElement] {
    var v: CFTypeRef?
    let app = AXUIElementCreateApplication(pid)
    guard AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &v) == .success else { return [] }
    return v as? [AXUIElement] ?? []
}
func setAX(_ w: AXUIElement, _ attr: String, _ value: CFTypeRef) {
    let r = AXUIElementSetAttributeValue(w, attr as CFString, value)
    if r != .success { die("AX set \(attr) failed: \(r.rawValue)") }
}

// MARK: images
struct Img { let w: Int; let h: Int; let px: [UInt8] }  // RGBA8
func load(_ path: String) -> Img {
    guard let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
          let img = CGImageSourceCreateImageAtIndex(src, 0, nil) else { die("cannot read \(path)") }
    let w = img.width, h = img.height
    var px = [UInt8](repeating: 0, count: w * h * 4)
    let ctx = CGContext(data: &px, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
                        space: CGColorSpace(name: CGColorSpace.sRGB)!,
                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))
    return Img(w: w, h: h, px: px)
}
func hex(_ s: String) -> (Int, Int, Int) {
    guard let v = Int(s, radix: 16) else { die("bad colour \(s)") }
    return ((v >> 16) & 255, (v >> 8) & 255, v & 255)
}
func clampRect(_ img: Img, _ x: Int, _ y: Int, _ w: Int, _ h: Int) -> (Int, Int, Int, Int) {
    let x0 = max(0, min(img.w, x)), y0 = max(0, min(img.h, y))
    return (x0, y0, max(0, min(img.w, x + w)) - x0, max(0, min(img.h, y + h)) - y0)
}

guard let cmd = args.first else { die("usage: helper <command> ...") }
switch cmd {
case "trust":
    print("accessibility \(AXIsProcessTrusted() ? 1 : 0)")
    print("screen \(CGPreflightScreenCaptureAccess() ? 1 : 0)")
case "wins":
    for w in windows(pid_t(num(1))) {
        let b = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
        print(w[kCGWindowNumber as String] as? Int ?? 0, Int(b["X"] ?? 0), Int(b["Y"] ?? 0),
              Int(b["Width"] ?? 0), Int(b["Height"] ?? 0), w[kCGWindowLayer as String] as? Int ?? 0)
    }
case "activate":
    NSRunningApplication(processIdentifier: pid_t(num(1)))?.activate(options: [.activateAllWindows])
    usleep(150_000)
case "key": postKey(args[1])
case "type": typeText(args.dropFirst().joined(separator: " "))
case "move": mouse(.mouseMoved, num(1), num(2))
case "click":
    mouse(.mouseMoved, num(1), num(2)); usleep(40_000)
    mouse(.leftMouseDown, num(1), num(2)); usleep(40_000); mouse(.leftMouseUp, num(1), num(2))
case "rclick":
    mouse(.mouseMoved, num(1), num(2)); usleep(40_000)
    mouse(.rightMouseDown, num(1), num(2), .right); usleep(40_000); mouse(.rightMouseUp, num(1), num(2), .right)
case "down": mouse(.mouseMoved, num(1), num(2)); usleep(40_000); mouse(.leftMouseDown, num(1), num(2))
case "drag": mouse(.leftMouseDragged, num(1), num(2))
case "up": mouse(.leftMouseUp, num(1), num(2))
case "ax":
    let pid = pid_t(num(1))
    switch args[2] {
    case "minimize": setAX(axWindow(pid), kAXMinimizedAttribute, kCFBooleanTrue)
    case "restore":
        for w in axAllWindows(pid) { AXUIElementSetAttributeValue(w, kAXMinimizedAttribute as CFString, kCFBooleanFalse) }
    case "size":
        var s = CGSize(width: num(3), height: num(4))
        setAX(axWindow(pid), kAXSizeAttribute, AXValueCreate(.cgSize, &s)!)
    case "pos":
        var p = CGPoint(x: num(3), y: num(4))
        setAX(axWindow(pid), kAXPositionAttribute, AXValueCreate(.cgPoint, &p)!)
    case "frame":
        let w = axWindow(pid)
        var p = CGPoint.zero, s = CGSize.zero, v: CFTypeRef?
        if AXUIElementCopyAttributeValue(w, kAXPositionAttribute as CFString, &v) == .success { AXValueGetValue(v as! AXValue, .cgPoint, &p) }
        if AXUIElementCopyAttributeValue(w, kAXSizeAttribute as CFString, &v) == .success { AXValueGetValue(v as! AXValue, .cgSize, &s) }
        print(Int(p.x), Int(p.y), Int(s.width), Int(s.height))
    default: die("unknown ax \(args[2])")
    }
case "px":
    let img = load(args[1]); let x = Int(num(2)), y = Int(num(3))
    guard x >= 0, y >= 0, x < img.w, y < img.h else { die("outside \(img.w)x\(img.h)") }
    let i = (y * img.w + x) * 4
    print(String(format: "%02x%02x%02x", img.px[i], img.px[i + 1], img.px[i + 2]))
case "stats":
    let img = load(args[1])
    let (x0, y0, w, h) = clampRect(img, Int(num(2)), Int(num(3)), Int(num(4)), Int(num(5)))
    let target = args.count > 7 ? hex(args[6]) : nil
    let tol = args.count > 7 ? Int(num(7)) : 0
    var black = 0, match = 0, n = 0
    var sr = 0.0, sg = 0.0, sb = 0.0, sl = 0.0, sll = 0.0
    for y in stride(from: y0, to: y0 + h, by: 2) {
        for x in stride(from: x0, to: x0 + w, by: 2) {
            let i = (y * img.w + x) * 4
            let r = Int(img.px[i]), g = Int(img.px[i + 1]), b = Int(img.px[i + 2])
            n += 1
            // a transparent pixel is a hole with nothing in it: as bad as black
            if (r < 14 && g < 14 && b < 14) || img.px[i + 3] < 16 { black += 1 }
            if let t = target, abs(r - t.0) <= tol && abs(g - t.1) <= tol && abs(b - t.2) <= tol { match += 1 }
            let l = 0.2126 * Double(r) + 0.7152 * Double(g) + 0.0722 * Double(b)
            sr += Double(r); sg += Double(g); sb += Double(b); sl += l; sll += l * l
        }
    }
    let d = Double(max(n, 1)), mean = sl / d
    print(String(format: "n %d black %.4f match %.4f mean %02x%02x%02x sd %.1f size %d %d", n,
                 Double(black) / d, Double(match) / d, Int(sr / d), Int(sg / d), Int(sb / d),
                 (max(0, sll / d - mean * mean)).squareRoot(), img.w, img.h))
case "bbox":
    let img = load(args[1]); let t = hex(args[2]); let tol = Int(num(3))
    var minX = Int.max, minY = Int.max, maxX = -1, maxY = -1, n = 0
    for y in stride(from: 0, to: img.h, by: 2) {
        for x in stride(from: 0, to: img.w, by: 2) {
            let i = (y * img.w + x) * 4
            if abs(Int(img.px[i]) - t.0) <= tol && abs(Int(img.px[i + 1]) - t.1) <= tol && abs(Int(img.px[i + 2]) - t.2) <= tol {
                n += 1; minX = min(minX, x); minY = min(minY, y); maxX = max(maxX, x); maxY = max(maxY, y)
            }
        }
    }
    if n == 0 { print("none \(img.w) \(img.h)") } else {
        let frac = Double(n) * 4 / Double(img.w * img.h)
        print(minX, minY, maxX - minX + 1, maxY - minY + 1, String(format: "%.4f", frac), img.w, img.h)
    }
case "diff":
    let a = load(args[1]), b = load(args[2])
    guard a.w == b.w, a.h == b.h else { print("1.0000 size"); break }
    let (x0, y0, w, h) = clampRect(a, Int(num(3)), Int(num(4)), Int(num(5)), Int(num(6)))
    var n = 0, d = 0
    for y in stride(from: y0, to: y0 + h, by: 2) {
        for x in stride(from: x0, to: x0 + w, by: 2) {
            let i = (y * a.w + x) * 4
            n += 1
            if abs(Int(a.px[i]) - Int(b.px[i])) + abs(Int(a.px[i + 1]) - Int(b.px[i + 1])) + abs(Int(a.px[i + 2]) - Int(b.px[i + 2])) > 24 { d += 1 }
        }
    }
    print(String(format: "%.4f", Double(d) / Double(max(n, 1))))
case "rusage":
    var tb = mach_timebase_info_data_t()
    mach_timebase_info(&tb)
    for a in args.dropFirst() {
        guard let pid = Int32(a) else { continue }
        var info = rusage_info_v4()
        let r = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: rusage_info_t?.self, capacity: 1) { proc_pid_rusage(pid, RUSAGE_INFO_V4, $0) }
        }
        if r != 0 { continue }
        let cpu = (info.ri_user_time + info.ri_system_time) * UInt64(tb.numer) / UInt64(tb.denom)
        print(pid, cpu, info.ri_instructions, info.ri_cycles, info.ri_billed_energy)
    }
default: die("unknown command \(cmd)")
}
