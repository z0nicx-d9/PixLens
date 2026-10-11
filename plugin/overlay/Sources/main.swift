// Pixlens Overlay: draws real pictures over the block previews pixlens leaves
// in Apple's Terminal, which cannot draw pixels itself.
//
// pixlens marks each block picture with a tag in its top-left cells: U+28FF
// then three braille cells holding a 24-bit id. `pixlens render --overlay`
// records the id's file in the overlay folder. Ten times a second this app
// reads the visible text of each Terminal window on screen through the
// Accessibility API, finds the tags, asks where on screen each one is drawn,
// and puts a borderless window holding the real picture over that box.
// Clicks pass through the windows to the terminal.

import AppKit
import ApplicationServices

let terminalBundle = "com.apple.Terminal"
let tagStart: unichar = 0x28FF
let bottomStart: unichar = 0x28FE
let rowMark: unichar = 0x28FD
/// Passes a second while anything on screen moves, and while it is still.
let busyRate = 60.0
let idleRate = 20.0
/// How long after the last change the overlay counts as still.
let settleAfter = 0.5
/// How long a picture stays up after its tag goes missing: longer than a
/// terminal redraw, short enough not to linger over text that replaced it.
let hideAfter = 0.2
/// After a wheel turn or key: a pass every `burstStep` for `burstFor`.
let burstStep = 0.005
let burstFor = 0.12
/// How far down from a window's top a press counts as on its title bar.
let titleBarHeight: CGFloat = 32

/// What `pixlens render --overlay` records for an id.
struct Entry: Decodable {
    /// A file apps can read: the picture, or a PNG made from it.
    let path: String
    /// The picture's own file; absent in entries from before it was kept.
    let original: String?
    let columns: Int
    let rows: Int
}

/// What the pointer can be on: a picture, or the name under it.
struct Target: Equatable {
    enum Part { case picture, name }
    let key: String
    let part: Part
}

/// A tag found on screen: which picture, the screen rect of its box, and
/// of the name drawn under it.
struct Spot {
    let id: String
    let frame: CGRect
    /// The part of the box on screen; the rest is cut away.
    let clip: CGRect
    /// Cells of the box the terminal draws something over; cut away too.
    var drawnOver: [CGRect] = []
    var caption: CGRect?
}

let folder: URL = {
    if let dir = ProcessInfo.processInfo.environment["PIXLENS_CACHE_DIR"] {
        return URL(fileURLWithPath: dir).appendingPathComponent("overlay")
    }
    let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
    return caches.appendingPathComponent("pixlens/overlay")
}()

func log(_ text: String) {
    guard ProcessInfo.processInfo.environment["PIXLENS_OVERLAY_DEBUG"] != nil else { return }
    let line = "\(Date()) \(text)\n"
    let url = folder.appendingPathComponent("overlay.log")
    if let handle = try? FileHandle(forWritingTo: url) {
        handle.seekToEndOfFile()
        handle.write(line.data(using: .utf8)!)
        try? handle.close()
    } else {
        try? line.write(to: url, atomically: true, encoding: .utf8)
    }
}

// MARK: Accessibility

func attribute<T>(_ element: AXUIElement, _ name: String) -> T? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
    return value as? T
}

func children(_ element: AXUIElement) -> [AXUIElement] {
    attribute(element, kAXChildrenAttribute) ?? []
}

/// Every text area under `element`: one per Terminal split.
func textAreas(_ element: AXUIElement, depth: Int = 0) -> [AXUIElement] {
    if depth > 6 { return [] }
    let role: String? = attribute(element, kAXRoleAttribute)
    if role == kAXTextAreaRole { return [element] }
    return children(element).flatMap { textAreas($0, depth: depth + 1) }
}

func frame(of element: AXUIElement) -> CGRect? {
    guard let position: AXValue = attribute(element, kAXPositionAttribute),
          let size: AXValue = attribute(element, kAXSizeAttribute) else { return nil }
    var point = CGPoint.zero
    var extent = CGSize.zero
    AXValueGetValue(position, .cgPoint, &point)
    AXValueGetValue(size, .cgSize, &extent)
    return CGRect(origin: point, size: extent)
}

/// The screen rect (top-left origin) of the characters at `range`.
func bounds(_ area: AXUIElement, _ range: CFRange) -> CGRect? {
    var range = range
    guard let parameter = AXValueCreate(.cfRange, &range) else { return nil }
    var value: CFTypeRef?
    let status = AXUIElementCopyParameterizedAttributeValue(
        area, kAXBoundsForRangeParameterizedAttribute as CFString, parameter, &value)
    guard status == .success, let rect = value else { return nil }
    var out = CGRect.zero
    AXValueGetValue(rect as! AXValue, .cgRect, &out)
    return out
}

/// The tags visible in one text area, with the box each picture takes.
func spots(in area: AXUIElement, entries: (String) -> Entry?) -> [Spot] {
    // The text area holds the whole scrollback; its scroll area is what shows.
    let parent: AXUIElement? = attribute(area, kAXParentAttribute)
    guard let areaFrame = parent.flatMap(frame(of:)) ?? frame(of: area),
          let range: AXValue = attribute(area, kAXVisibleCharacterRangeAttribute) else { return [] }
    var visible = CFRange()
    AXValueGetValue(range, .cfRange, &visible)
    // Only the visible text crosses over: the scrollback can be long.
    guard let text = string(area, visible) else { return [] }
    let units = Array(text.utf16)

    // A picture's first row starts with its tag, its last row with the
    // bottom tag; either one places it, so a picture scrolled half away
    // still shows its other half.
    var anchors: [String: (index: Int, isTop: Bool)] = [:]
    var i = 0
    while i + 3 < units.count {
        defer { i += 1 }
        guard units[i] == tagStart || units[i] == bottomStart else { continue }
        let digits = units[(i + 1)...(i + 3)]
        guard digits.allSatisfy({ $0 >= 0x2800 && $0 <= 0x28FF }) else { continue }
        let id = digits.map { String(format: "%02x", $0 - 0x2800) }.joined()
        let isTop = units[i] == tagStart
        if isTop || anchors[id] == nil { anchors[id] = (i, isTop) }
        i += 3
    }

    let lines = Lines(units)
    var found: [Spot] = []
    for (id, anchor) in anchors {
        guard let entry = entries(id),
              let cell = bounds(area, CFRange(location: visible.location + anchor.index, length: 1)) else { continue }
        let height = cell.height * CGFloat(entry.rows)
        let box = CGRect(x: cell.minX, y: anchor.isTop ? cell.minY : cell.maxY - height,
                         width: cell.width * CGFloat(entry.columns), height: height)
        // Rows and name are read off the text already in hand: asking the
        // terminal row by row stalls while it redraws, and the pictures lag.
        let (line, column) = lines.place(of: anchor.index)
        let topLine = anchor.isTop ? line : line - (entry.rows - 1)
        let idUnits = Array(units[(anchor.index + 1)...(anchor.index + 3)])
        var mine: [Int] = []
        var drawnOver: [CGRect] = []
        for row in 0..<entry.rows {
            let rowTop = box.minY + CGFloat(row) * cell.height
            guard rowTop >= areaFrame.minY - 1, rowTop + cell.height <= areaFrame.maxY + 1 else { continue }
            // Rows above the text the terminal reports as visible, yet on
            // screen, cannot be checked; above the bottom tag they are ours.
            if topLine + row < 0 {
                mine.append(row)
                continue
            }
            guard let at = lines.mark(line: topLine + row, near: column, id: idUnits) else { continue }
            mine.append(row)
            // The picture's cells are blank past its mark; text there is
            // something drawn over it (a "Jump to bottom" pill) and shows.
            for run in lines.text(line: topLine + row, from: at + 4, to: at + entry.columns) {
                drawnOver.append(CGRect(x: box.minX + CGFloat(run.lowerBound - at) * cell.width, y: rowTop,
                                        width: CGFloat(run.count) * cell.width, height: cell.height))
            }
        }
        guard let first = mine.first, let last = mine.last else { continue }
        let shown = CGRect(x: box.minX, y: box.minY + CGFloat(first) * cell.height,
                           width: box.width, height: CGFloat(last - first + 1) * cell.height)
        var spot = Spot(id: id, frame: box, clip: shown, drawnOver: drawnOver)
        if last == entry.rows - 1, let run = lines.run(line: topLine + entry.rows, under: column, width: entry.columns) {
            spot.caption = CGRect(x: box.minX + CGFloat(run.lowerBound - column) * cell.width, y: box.maxY,
                                  width: CGFloat(run.count) * cell.width, height: cell.height)
        }
        found.append(spot)
    }
    return found
}

/// The visible text cut into screen rows, to read cells by row and column.
/// Columns count UTF-16 units, which is one a cell for the tags, the block
/// glyphs and the names pixlens draws.
struct Lines {
    let units: [UInt16]
    /// Where each row starts in `units`.
    let starts: [Int]

    init(_ units: [UInt16]) {
        self.units = units
        var starts = [0]
        for (i, unit) in units.enumerated() where unit == 0x0A { starts.append(i + 1) }
        self.starts = starts
    }

    func row(_ line: Int) -> ArraySlice<UInt16>? {
        guard line >= 0, line < starts.count else { return nil }
        let end = line + 1 < starts.count ? starts[line + 1] - 1 : units.count
        return units[starts[line]..<max(starts[line], end)]
    }

    /// The row and column of the unit at `index`.
    func place(of index: Int) -> (line: Int, column: Int) {
        var low = 0
        var high = starts.count - 1
        while low < high {
            let middle = (low + high + 1) / 2
            if starts[middle] <= index { low = middle } else { high = middle - 1 }
        }
        return (low, index - starts[low])
    }

    /// The column of this picture's mark (a tag or a row mark, then its
    /// `id`) in the row, at `column` or a few cells off: text before it in
    /// that row (a pane beside) may count differently. Nil without one.
    func mark(line: Int, near column: Int, id: [UInt16]) -> Int? {
        guard let row = row(line) else { return nil }
        let base = row.startIndex
        for offset in [0, -1, 1, -2, 2, -3, 3] {
            let at = base + column + offset
            guard at >= base, at + 3 < row.endIndex,
                  [tagStart, bottomStart, rowMark].contains(row[at]) else { continue }
            if Array(row[(at + 1)...(at + 3)]) == id { return column + offset }
        }
        return nil
    }

    /// The runs of text in a row between two columns, a cell wider on each
    /// side (a pill's padding); words one space apart make one run.
    func text(line: Int, from start: Int, to end: Int) -> [Range<Int>] {
        guard let row = row(line), start < end else { return [] }
        let isBlank = { (column: Int) -> Bool in
            let at = row.startIndex + column
            return at >= row.endIndex || row[at] == 0x20 || row[at] == 0xA0
        }
        var runs: [Range<Int>] = []
        var open: Int?
        var column = start
        while column < end {
            if !isBlank(column) {
                if open == nil { open = column }
            } else if let from = open, column + 1 >= end || isBlank(column + 1) {
                runs.append(max(start, from - 1)..<min(end, column + 1))
                open = nil
            }
            column += 1
        }
        if let from = open { runs.append(max(start, from - 1)..<end) }
        return runs
    }

    /// The run of text in the row under a picture at `column`, `width` cells
    /// wide, nearest its middle: its columns. Runs end at two spaces or a
    /// frame line, so a name with single spaces stays whole.
    func run(line: Int, under column: Int, width: Int) -> Range<Int>? {
        guard let row = row(line) else { return nil }
        let cells = Array(row)
        let isBreak = { (unit: UInt16) in unit >= 0x2500 && unit <= 0x257F }
        let middle = Double(column) + Double(width) / 2
        var best: Range<Int>?
        var start: Int?
        for i in 0...cells.count {
            let isGap = i == cells.count || isBreak(cells[i])
                || (cells[i] == 0x20 && (i + 1 == cells.count || cells[i + 1] == 0x20))
            if isGap {
                if let from = start, i > column, from < column + width {
                    let run = from..<i
                    let distance = abs(Double(from + i) / 2 - middle)
                    if best.map({ abs(Double($0.lowerBound + $0.upperBound) / 2 - middle) > distance }) ?? true {
                        best = run
                    }
                }
                start = nil
            } else if start == nil, cells[i] != 0x20 {
                start = i
            }
        }
        return best
    }
}

func parameterized<T>(_ element: AXUIElement, _ name: String, _ parameter: CFTypeRef) -> T? {
    var value: CFTypeRef?
    guard AXUIElementCopyParameterizedAttributeValue(element, name as CFString, parameter, &value) == .success else {
        return nil
    }
    return value as? T
}

/// The text at `range` of a text area.
func string(_ area: AXUIElement, _ range: CFRange) -> String? {
    var range = range
    guard let parameter = AXValueCreate(.cfRange, &range) else { return nil }
    var value: CFTypeRef?
    let status = AXUIElementCopyParameterizedAttributeValue(
        area, kAXStringForRangeParameterizedAttribute as CFString, parameter, &value)
    return status == .success ? value as? String : nil
}

// MARK: Windows

/// Opens a picture in Preview, or the default app when Preview is missing.
func openInPreview(_ file: URL) {
    if let app = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.Preview") {
        NSWorkspace.shared.open([file], withApplicationAt: app, configuration: NSWorkspace.OpenConfiguration())
    } else {
        NSWorkspace.shared.open(file)
    }
}

/// Converts a top-left-origin screen rect to AppKit's bottom-left origin.
func appKitFrame(_ rect: CGRect) -> CGRect {
    let height = NSScreen.screens.first?.frame.height ?? 0
    return CGRect(x: rect.minX, y: height - rect.maxY, width: rect.width, height: rect.height)
}

final class Overlay: NSObject, NSApplicationDelegate {
    var panels: [String: NSPanel] = [:]
    var entries: [String: Entry] = [:]
    var images: [String: NSImage] = [:]
    /// Each Terminal window's text areas, found once: walking the tree is
    /// the dearest part of a pass.
    var areas: [AXUIElement: [AXUIElement]] = [:]
    /// What the last pass drew, to tell a still screen from a moving one.
    var lastDrawn: [String: [CGRect]] = [:]
    var lastChange = Date()
    var lastPass = Date.distantPast
    var observer: AXObserver?
    /// Where each shown picture is and what covers it (top-left origin), for
    /// telling which one the pointer is on.
    var placed: [String: (spot: Spot, covers: [CGRect])] = [:]
    var hovered: Target?
    var pressed: (target: Target, at: CGPoint)?
    /// The ring over a name under the pointer.
    var nameRing: NSPanel?
    var wasReady: Bool?
    /// A Terminal window being dragged: where the press was, and where its
    /// pictures were then.
    var dragging: (start: CGPoint, frames: [String: CGRect])?
    var burstTimer: Timer?
    var burstUntil = Date.distantPast
    /// When each shown picture's tag went missing.
    var missingSince: [String: Date] = [:]
    var observedPid: pid_t = 0

    func applicationDidFinishLaunching(_ notification: Notification) {
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
        log("started, trusted=\(AXIsProcessTrustedWithOptions(options))")
        // A busy terminal must not hold the overlay up: give up on a slow
        // answer and try again on the next pass.
        AXUIElementSetMessagingTimeout(AXUIElementCreateSystemWide(), 0.1)
        // The pictures let clicks through to the terminal; these watch the
        // pointer from outside to light the picture under it and open it.
        NSEvent.addGlobalMonitorForEvents(matching: .mouseMoved) { [weak self] _ in
            self?.hover()
        }
        NSEvent.addGlobalMonitorForEvents(matching: .leftMouseDragged) { [weak self] _ in
            guard let self else { return }
            if self.dragging != nil { self.followDrag() } else { self.hover() }
        }
        NSEvent.addGlobalMonitorForEvents(matching: .leftMouseDown) { [weak self] _ in
            guard let self else { return }
            self.startDrag()
            guard let target = self.target(at: NSEvent.mouseLocation) else { return }
            self.pressed = (target, NSEvent.mouseLocation)
        }
        NSEvent.addGlobalMonitorForEvents(matching: .leftMouseUp) { [weak self] _ in
            guard let self else { return }
            if self.dragging != nil {
                self.dragging = nil
                self.update()
            }
            guard let pressed = self.pressed else { return }
            self.pressed = nil
            let point = NSEvent.mouseLocation
            // A drag (selecting text) is not a click.
            guard hypot(point.x - pressed.at.x, point.y - pressed.at.y) < 4,
                  self.target(at: point) == pressed.target else { return }
            self.click(pressed.target)
        }

        // A wheel turn or a key moves the terminal's text within a few
        // milliseconds: look again and again just after, to move with it.
        NSEvent.addGlobalMonitorForEvents(matching: [.scrollWheel, .keyDown]) { [weak self] _ in
            self?.burst()
        }

        markReady()
        Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in self?.markReady() }
        NotificationCenter.default.addObserver(
            forName: NSApplication.willTerminateNotification, object: nil, queue: .main
        ) { _ in try? FileManager.default.removeItem(at: folder.appendingPathComponent("ready")) }

        // Ticks at the busy rate; a still screen is looked at less often.
        let timer = Timer(timeInterval: 1 / busyRate, repeats: true) { [weak self] _ in self?.tick() }
        timer.tolerance = 0.002
        RunLoop.main.add(timer, forMode: .common)
        NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main
        ) { [weak self] _ in
            // The window list catches up with an app switch over a few frames.
            for delay in [0.0, 0.03, 0.06, 0.12] {
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) { self?.update() }
            }
        }
    }

    /// A press on a Terminal window's title bar may start dragging it: the
    /// pictures over it are then moved with the pointer, frame for frame,
    /// instead of after the terminal reports where the window went.
    func startDrag() {
        let height = NSScreen.screens.first?.frame.height ?? 0
        let point = NSEvent.mouseLocation
        let spot = CGPoint(x: point.x, y: height - point.y)
        guard let terminal = NSRunningApplication.runningApplications(withBundleIdentifier: terminalBundle).first,
              let window = onScreen().first(where: { $0.bounds.contains(spot) }),
              window.pid == terminal.processIdentifier,
              spot.y < window.bounds.minY + titleBarHeight else { return }
        let prefix = "\(window.number)-"
        let frames = panels.filter { $0.key.hasPrefix(prefix) && $0.value.isVisible }.mapValues(\.frame)
        guard !frames.isEmpty else { return }
        dragging = (point, frames)
        hover()
    }

    func followDrag() {
        guard let dragging else { return }
        let point = NSEvent.mouseLocation
        let dx = point.x - dragging.start.x
        let dy = point.y - dragging.start.y
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for (key, frame) in dragging.frames {
            panels[key]?.setFrameOrigin(CGPoint(x: frame.minX + dx, y: frame.minY + dy))
        }
        CATransaction.commit()
    }

    /// Passes every few milliseconds for a short while, so the pictures land
    /// in the same screen refresh as the terminal's redraw.
    func burst() {
        lastChange = Date()
        burstUntil = Date().addingTimeInterval(burstFor)
        guard burstTimer == nil else { return }
        let timer = Timer(timeInterval: burstStep, repeats: true) { [weak self] timer in
            guard let self else { return }
            if Date() > self.burstUntil {
                timer.invalidate()
                self.burstTimer = nil
                return
            }
            self.update()
        }
        RunLoop.main.add(timer, forMode: .common)
        burstTimer = timer
    }

    /// Says, in the overlay folder, whether this process may read the screen
    /// and so will draw: pixlens leaves pictures blank under it only then.
    func markReady() {
        let url = folder.appendingPathComponent("ready")
        let isTrusted = AXIsProcessTrusted()
        guard isTrusted != wasReady else { return }
        wasReady = isTrusted
        if isTrusted {
            try? String(ProcessInfo.processInfo.processIdentifier).write(to: url, atomically: true, encoding: .utf8)
        } else {
            try? FileManager.default.removeItem(at: url)
        }
    }

    func tick() {
        let isStill = Date().timeIntervalSince(lastChange) > settleAfter
        if isStill && Date().timeIntervalSince(lastPass) < 1 / idleRate { return }
        update()
    }

    /// Wakes the overlay the moment a Terminal window moves or resizes, or its
    /// text changes, rather than on its next pass.
    func observe(_ pid: pid_t) {
        guard pid != observedPid else { return }
        observedPid = pid
        areas = [:]
        var created: AXObserver?
        let callback: AXObserverCallback = { _, _, notification, context in
            guard let context else { return }
            let overlay = Unmanaged<Overlay>.fromOpaque(context).takeUnretainedValue()
            // A window moving keeps the overlay busy; text changes (a spinner,
            // streaming output) only get a pass, at most one a frame, and the
            // pass itself tells whether any picture moved.
            if (notification as String) != kAXValueChangedNotification { overlay.lastChange = Date() }
            if Date().timeIntervalSince(overlay.lastPass) > 1 / busyRate { overlay.update() }
        }
        guard AXObserverCreate(pid, callback, &created) == .success, let created else { return }
        let app = AXUIElementCreateApplication(pid)
        let context = Unmanaged.passUnretained(self).toOpaque()
        for name in [kAXWindowMovedNotification, kAXWindowResizedNotification, kAXFocusedWindowChangedNotification,
                     kAXWindowCreatedNotification, kAXUIElementDestroyedNotification,
                     // The terminal's text changing: follow a redraw at once.
                     kAXValueChangedNotification] {
            AXObserverAddNotification(created, app, name as CFString, context)
        }
        CFRunLoopAddSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(created), .commonModes)
        observer = created
    }

    func entry(_ id: String) -> Entry? {
        if let known = entries[id] { return known }
        let url = folder.appendingPathComponent("\(id).json")
        guard let data = try? Data(contentsOf: url),
              let entry = try? JSONDecoder().decode(Entry.self, from: data) else { return nil }
        entries[id] = entry
        return entry
    }

    /// Lays the pictures over every Terminal window on screen, frontmost or
    /// not. The pictures float, so raising Terminal never hides them for a
    /// moment; instead each one is cut where other windows cover its own.
    func update() {
        guard AXIsProcessTrusted(),
              let terminal = NSRunningApplication.runningApplications(withBundleIdentifier: terminalBundle).first,
              !terminal.isHidden else {
            hideAll()
            return
        }
        // The drag moves the pictures itself until the button comes up.
        if dragging != nil { return }
        lastPass = Date()
        observe(terminal.processIdentifier)
        let screen = onScreen()
        let element = AXUIElementCreateApplication(terminal.processIdentifier)
        let windows: [AXUIElement] = attribute(element, kAXWindowsAttribute) ?? []
        areas = areas.filter { windows.contains($0.key) }

        var wanted: [String: (spot: Spot, covers: [CGRect])] = [:]
        for window in windows {
            if attribute(window, kAXMinimizedAttribute) == true { continue }
            // Windows on another Space are not on screen: nothing to draw.
            guard let bounds = frame(of: window),
                  let index = screen.firstIndex(where: { $0.pid == terminal.processIdentifier && $0.bounds.equalTo(bounds) })
            else { continue }
            let covers = screen[..<index].map(\.bounds)
            let found = areas[window] ?? textAreas(window)
            areas[window] = found
            for spot in found.flatMap({ spots(in: $0, entries: entry) }) {
                wanted["\(screen[index].number)-\(spot.id)"] = (spot, covers)
            }
        }
        let drawn = wanted.mapValues { place in
            [place.spot.frame] + place.covers.filter { $0.intersects(place.spot.frame) }
        }
        if drawn != lastDrawn {
            lastDrawn = drawn
            lastChange = Date()
        }

        CATransaction.begin()
        CATransaction.setDisableActions(true)
        // While the terminal redraws, a tag can be missing for a pass or two;
        // a picture is taken down only once it has stayed gone a moment.
        let now = Date()
        for key in wanted.keys { missingSince[key] = nil }
        for (key, panel) in panels where wanted[key] == nil && panel.isVisible {
            let since = missingSince[key] ?? now
            missingSince[key] = since
            if now.timeIntervalSince(since) >= hideAfter {
                panel.orderOut(nil)
                placed[key] = nil
                missingSince[key] = nil
            }
        }
        for (key, place) in wanted { show(key, place.spot, covers: place.covers) }
        CATransaction.commit()
        // A picture may have moved under a still pointer.
        hover()
    }

    /// The picture or name under `point` (AppKit coordinates), where nothing
    /// covers it.
    func target(at point: CGPoint) -> Target? {
        let height = NSScreen.screens.first?.frame.height ?? 0
        let spot = CGPoint(x: point.x, y: height - point.y)
        for (key, place) in placed where !place.covers.contains(where: { $0.contains(spot) }) {
            if place.spot.frame.contains(spot) { return Target(key: key, part: .picture) }
            if let name = place.spot.caption, name.insetBy(dx: -2, dy: -1).contains(spot) {
                return Target(key: key, part: .name)
            }
        }
        return nil
    }

    /// Lights what is under the pointer, so it reads as something to click:
    /// a picture gets a ring in the accent color and a little brightening,
    /// a name a tinted pill.
    func hover() {
        let now = target(at: NSEvent.mouseLocation)
        guard now != hovered else { return }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        if let old = hovered, old.part == .picture, let layer = panels[old.key]?.contentView?.layer {
            layer.sublayers?.filter { $0.name == "hover" }.forEach { $0.removeFromSuperlayer() }
        }
        nameRing?.orderOut(nil)
        if let now, now.part == .picture, let layer = panels[now.key]?.contentView?.layer {
            let ring = CALayer()
            ring.name = "hover"
            ring.frame = layer.bounds
            ring.autoresizingMask = [.layerWidthSizable, .layerHeightSizable]
            ring.backgroundColor = NSColor.white.withAlphaComponent(0.12).cgColor
            ring.borderColor = NSColor.controlAccentColor.cgColor
            ring.borderWidth = 2
            ring.cornerRadius = 3
            layer.addSublayer(ring)
        }
        if let now, now.part == .name, let name = placed[now.key]?.spot.caption {
            let ring = nameRing ?? makeNameRing()
            ring.setFrame(appKitFrame(name.insetBy(dx: -4, dy: -1)), display: false, animate: false)
            ring.orderFrontRegardless()
            nameRing = ring
        }
        CATransaction.commit()
        hovered = now
    }

    func makeNameRing() -> NSPanel {
        let panel = NSPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel],
                            backing: .buffered, defer: true)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.animationBehavior = .none
        panel.ignoresMouseEvents = true
        panel.level = .floating
        panel.collectionBehavior = [.fullScreenAuxiliary, .stationary, .ignoresCycle]
        let view = NSView()
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.controlAccentColor.withAlphaComponent(0.28).cgColor
        view.layer?.borderColor = NSColor.controlAccentColor.cgColor
        view.layer?.borderWidth = 1.5
        view.layer?.cornerRadius = 5
        panel.contentView = view
        return panel
    }

    /// A click: a picture opens in Preview, a name shows the picture's menu.
    func click(_ target: Target) {
        guard let id = placed[target.key]?.spot.id, let entry = entries[id] else { return }
        switch target.part {
        case .picture:
            openInPreview(URL(fileURLWithPath: entry.path))
        case .name:
            guard let me = Bundle.main.executableURL else { return }
            let menu = Process()
            menu.executableURL = me
            menu.arguments = ["--menu", entry.path, entry.original ?? entry.path]
            try? menu.run()
        }
    }

    /// The ordinary windows on screen, front to back, ours left out, with
    /// their bounds (top-left origin, as Accessibility gives them).
    func onScreen() -> [(number: CGWindowID, pid: pid_t, bounds: CGRect)] {
        let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
            as? [[String: Any]] ?? []
        let me = ProcessInfo.processInfo.processIdentifier
        var windows: [(CGWindowID, pid_t, CGRect)] = []
        for info in list {
            guard let number = info[kCGWindowNumber as String] as? CGWindowID,
                  let owner = info[kCGWindowOwnerPID as String] as? pid_t, owner != me,
                  info[kCGWindowLayer as String] as? Int == 0,
                  (info[kCGWindowAlpha as String] as? Double ?? 1) > 0,
                  let dict = info[kCGWindowBounds as String] as? NSDictionary,
                  let bounds = CGRect(dictionaryRepresentation: dict) else { continue }
            windows.append((number, owner, bounds))
        }
        return windows
    }

    func hideAll() {
        for panel in panels.values { panel.orderOut(nil) }
        placed = [:]
        hover()
    }

    /// Places the picture over `spot`, cut where `covers` (the windows in
    /// front of its Terminal window) lie over it; hidden when covered whole.
    func show(_ key: String, _ spot: Spot, covers: [CGRect]) {
        guard let entry = entries[spot.id] else { return }
        let panel = panels[key] ?? makePanel(key, entry)
        guard let panel, let view = panel.contentView else { return }
        let target = appKitFrame(spot.frame)
        if panel.frame != target { panel.setFrame(target, display: false, animate: false) }

        // What lies outside the rows on screen is cut away like a cover.
        let outside = [
            CGRect(x: spot.frame.minX, y: spot.frame.minY, width: spot.frame.width, height: spot.clip.minY - spot.frame.minY),
            CGRect(x: spot.frame.minX, y: spot.clip.maxY, width: spot.frame.width, height: spot.frame.maxY - spot.clip.maxY),
        ].filter { $0.height > 0.5 } + spot.drawnOver
        let hidden = (covers + outside).map { $0.intersection(spot.frame) }.filter { !$0.isNull && !$0.isEmpty }
        if hidden.contains(where: { $0.contains(spot.frame) }) {
            panel.orderOut(nil)
            placed[key] = nil
            return
        }
        placed[key] = (spot, covers + outside)
        view.wantsLayer = true
        if hidden.isEmpty {
            view.layer?.mask = nil
        } else {
            let mask = (view.layer?.mask as? CAShapeLayer) ?? CAShapeLayer()
            mask.path = visiblePath(of: spot.frame, less: hidden)
            view.layer?.mask = mask
        }
        if !panel.isVisible {
            // Drawn before it shows, so it never flashes empty.
            panel.displayIfNeeded()
            panel.orderFrontRegardless()
        }
    }

    /// The part of `box` no rect of `hidden` covers, in the panel's own
    /// coordinates (bottom-left origin). The box is cut along every edge of
    /// the covers into a grid and the free cells kept, so overlapping covers
    /// cut once.
    func visiblePath(of box: CGRect, less hidden: [CGRect]) -> CGPath {
        let xs = Set([box.minX, box.maxX] + hidden.flatMap { [$0.minX, $0.maxX] }).sorted()
        let ys = Set([box.minY, box.maxY] + hidden.flatMap { [$0.minY, $0.maxY] }).sorted()
        let path = CGMutablePath()
        for (x0, x1) in zip(xs, xs.dropFirst()) {
            for (y0, y1) in zip(ys, ys.dropFirst()) {
                let cell = CGRect(x: x0, y: y0, width: x1 - x0, height: y1 - y0)
                let middle = CGPoint(x: cell.midX, y: cell.midY)
                if !box.contains(middle) || hidden.contains(where: { $0.contains(middle) }) { continue }
                path.addRect(CGRect(x: cell.minX - box.minX, y: box.maxY - cell.maxY,
                                    width: cell.width, height: cell.height))
            }
        }
        return path
    }

    func makePanel(_ key: String, _ entry: Entry) -> NSPanel? {
        // By the picture's id, not its path: a file drawn again after it
        // changed gets a new id, and must not show the old picture.
        let id = String(key.split(separator: "-").last ?? Substring(key))
        guard let image = images[id] ?? NSImage(contentsOfFile: entry.path) else {
            log("cannot load \(entry.path)")
            return nil
        }
        images[id] = image
        let panel = NSPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel],
                            backing: .buffered, defer: true)
        // Clear, so the parts a mask cuts away show what lies beneath.
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.animationBehavior = .none
        panel.ignoresMouseEvents = true
        panel.level = .floating
        panel.collectionBehavior = [.fullScreenAuxiliary, .stationary, .ignoresCycle]
        let view = NSImageView(image: image)
        view.wantsLayer = true
        view.imageScaling = .scaleAxesIndependently
        view.imageFrameStyle = .none
        panel.contentView = view
        panels[key] = panel
        return panel
    }
}

/// `--dump`: prints what the overlay sees in Terminal's front window, then exits.
if CommandLine.arguments.contains("--dump") {
    print("trusted", AXIsProcessTrusted())
    guard let terminal = NSRunningApplication.runningApplications(withBundleIdentifier: terminalBundle).first else {
        print("Terminal not running")
        exit(1)
    }
    let element = AXUIElementCreateApplication(terminal.processIdentifier)
    let windows: [AXUIElement] = attribute(element, kAXWindowsAttribute) ?? []
    print("windows", windows.count)
    let overlay = Overlay()
    for window in windows {
        let title: String = attribute(window, kAXTitleAttribute) ?? "?"
        let areas = textAreas(window)
        print("window", title, "areas", areas.count)
        for area in areas {
            let text: String = attribute(area, kAXValueAttribute) ?? ""
            print("  area", frame(of: area) ?? .zero, "chars", text.utf16.count,
                  "spots", spots(in: area, entries: overlay.entry).map { "\($0.id) \($0.frame) clip \($0.clip) name \(String(describing: $0.caption))" })
        }
    }
    if CommandLine.arguments.contains("--bench") {
        let started = Date()
        for _ in 0..<50 {
            for window in windows { _ = textAreas(window).flatMap { spots(in: $0, entries: overlay.entry) } }
            _ = overlay.onScreen()
        }
        print(String(format: "one pass: %.2f ms", Date().timeIntervalSince(started) * 1000 / 50))
        let areas = windows.flatMap { textAreas($0) }
        var t = Date()
        for _ in 0..<50 { for window in windows { _ = textAreas(window) } }
        print(String(format: "  find areas: %.2f ms", Date().timeIntervalSince(t) * 1000 / 50))
        t = Date()
        for _ in 0..<50 { for area in areas { _ = spots(in: area, entries: overlay.entry) } }
        print(String(format: "  spots: %.2f ms", Date().timeIntervalSince(t) * 1000 / 50))
        t = Date()
        for _ in 0..<50 { _ = overlay.onScreen() }
        print(String(format: "  window list: %.2f ms", Date().timeIntervalSince(t) * 1000 / 50))
    }
    exit(0)
}

/// `--menu <viewable> <original>`: a native menu at the pointer for one
/// picture (what a click on its name shows), then exits. `viewable` is a
/// file apps can read; `original` is the picture's own file.
final class Menu: NSObject, NSApplicationDelegate {
    let viewable: URL
    let original: URL
    var previous: NSRunningApplication?
    /// Whether the pick opened another app, which then keeps the focus.
    var hasOpened = false

    init(viewable: String, original: String) {
        self.viewable = URL(fileURLWithPath: viewable)
        self.original = URL(fileURLWithPath: original)
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        previous = NSWorkspace.shared.frontmostApplication
        let menu = NSMenu()
        menu.autoenablesItems = false
        let title = NSMenuItem(title: original.lastPathComponent, action: nil, keyEquivalent: "")
        title.isEnabled = false
        menu.addItem(title)
        menu.addItem(.separator())
        for (label, action) in [("Open in Preview", #selector(preview)), ("Quick Look", #selector(quickLook)),
                                ("Copy Image", #selector(copyImage)), ("Show in Finder", #selector(reveal))] {
            let item = NSMenuItem(title: label, action: action, keyEquivalent: "")
            item.target = self
            menu.addItem(item)
        }
        // A menu tracks only for the active app; the terminal gets the focus back after.
        NSApp.activate(ignoringOtherApps: true)
        menu.popUp(positioning: nil, at: NSEvent.mouseLocation, in: nil)
        finish()
    }

    func finish() {
        if !hasOpened { previous?.activate() }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { exit(0) }
    }

    @objc func preview() {
        hasOpened = true
        openInPreview(viewable)
    }

    @objc func quickLook() {
        hasOpened = true
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/qlmanage")
        task.arguments = ["-p", viewable.path]
        task.standardOutput = FileHandle.nullDevice
        task.standardError = FileHandle.nullDevice
        try? task.run()
    }

    @objc func copyImage() {
        // The data goes on the pasteboard now, not as a promise: this
        // process is gone by the time anyone pastes.
        guard let image = NSImage(contentsOf: viewable), let tiff = image.tiffRepresentation,
              let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) else { return }
        let board = NSPasteboard.general
        board.clearContents()
        board.setData(png, forType: .png)
        board.setData(tiff, forType: .tiff)
    }

    @objc func reveal() {
        hasOpened = true
        NSWorkspace.shared.activateFileViewerSelecting([original])
    }
}

if let at = CommandLine.arguments.firstIndex(of: "--menu"), CommandLine.arguments.count > at + 2 {
    let menu = Menu(viewable: CommandLine.arguments[at + 1], original: CommandLine.arguments[at + 2])
    let app = NSApplication.shared
    app.delegate = menu
    app.setActivationPolicy(.accessory)
    app.run()
}

// One overlay a login: a second start leaves the first one drawing.
let others = NSRunningApplication.runningApplications(withBundleIdentifier: "dev.pixlens.overlay")
if others.contains(where: { $0.processIdentifier != ProcessInfo.processInfo.processIdentifier }) {
    exit(0)
}

let app = NSApplication.shared
let delegate = Overlay()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
