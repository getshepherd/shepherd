// Reads program logos out of Rex Beta.app's icon catalog into assets/icons/<name>-mark.svg, with square viewBoxes
// (GPUI scales an svg by its width and draws it from the top left corner, so a tall one would spill out of its tile).
//
//   swift scripts/rex-icons.swift            rewrites the marks taken from Rex (`taken` below)
//   swift scripts/rex-icons.swift yazi ...   takes these too; add them to `taken`, assets/NOTICE and tab_bar.rs
//   swift scripts/rex-icons.swift --list     the catalog's glyphs Shepherd has no mark for yet
//
// Only logos of open-source programs: not Rex's own interface glyphs (top, terminal, log, ...), nor commercial
// services' logos. Simple Icons comes first; Rex fills its gaps. Check a new one at tile size (11 pt), some turn to mush.
import Foundation

// Shepherd's mark name -> the catalog's `block-glyph-<name>`.
let taken = [
    "btop": "btop", "d2": "d2", "fx": "fx", "gnu": "gnu", "kakoune": "kakoune",
    "moon": "moon", "pixi": "pixi", "proto": "proto", "yazi": "yazi",
]

let app = ProcessInfo.processInfo.environment["REX_APP"] ?? "/Applications/Rex Beta.app"
let car = URL(fileURLWithPath: app + "/Contents/Resources/SuperBlockIcons_SuperBlockIcons.bundle/Contents/Resources/Assets.car")
let out = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("../assets/icons").standardized

func fail(_ message: String) -> Never {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
    exit(1)
}

// CoreUI's CUICatalog reads an Assets.car; CoreSVG writes a vector rendition back out as svg. Both are private, so
// this is reached through the runtime and breaks when Apple renames them.
guard dlopen("/System/Library/PrivateFrameworks/CoreUI.framework/CoreUI", RTLD_NOW) != nil,
      let svg = dlopen("/System/Library/PrivateFrameworks/CoreSVG.framework/CoreSVG", RTLD_NOW),
      let write = dlsym(svg, "CGSVGDocumentWriteToData"),
      let cls = NSClassFromString("CUICatalog") as? NSObject.Type
else { fail("CoreUI or CoreSVG is missing") }
let writeToData = unsafeBitCast(write, to: (@convention(c) (UnsafeRawPointer, CFMutableData, CFDictionary?) -> Void).self)

let catalog = cls.init()
typealias InitURL = @convention(c) (AnyObject, Selector, NSURL, UnsafeMutablePointer<NSError?>?) -> AnyObject?
let initURL = NSSelectorFromString("initWithURL:error:")
guard FileManager.default.fileExists(atPath: car.path),
      let loaded = unsafeBitCast(catalog.method(for: initURL), to: InitURL.self)(catalog, initURL, car as NSURL, nil)
else { fail("no catalog at \(car.path) (REX_APP names the app)") }

// A CGSVGDocumentRef, a CF type, which key-value coding can't hand back.
typealias Document = @convention(c) (AnyObject, Selector) -> UnsafeRawPointer?
let document = NSSelectorFromString("svgDocument")

func glyph(_ name: String) -> String? {
    typealias Named = @convention(c) (AnyObject, Selector, NSString, Double, Int, Int) -> AnyObject?
    let sel = NSSelectorFromString("namedVectorImageWithName:scaleFactor:displayGamut:layoutDirection:")
    guard let image = unsafeBitCast(loaded.method(for: sel), to: Named.self)(loaded, sel, "block-glyph-\(name)" as NSString, 2, 0, 0),
          let doc = unsafeBitCast(image.method(for: document), to: Document.self)(image, document)
    else { return nil }
    let data = CFDataCreateMutable(nil, 0)!
    writeToData(doc, data, nil)
    return String(data: data as Data, encoding: .utf8)
}

// Drops CoreSVG's xml prologue and <title>, and grows the shorter side of the viewBox around its center.
func squared(_ written: String) -> String {
    var svg = written.range(of: "<svg").map { String(written[$0.lowerBound...]) } ?? written
    svg = svg.replacingOccurrences(of: #"\n *<title>[^<]*</title>"#, with: "", options: .regularExpression)
    guard let match = svg.range(of: #"viewBox="[^"]*""#, options: .regularExpression) else { return svg }
    let v = svg[match].dropFirst(9).dropLast().split(separator: " ").compactMap { Double($0) }
    guard v.count == 4 else { return svg }
    let side = max(v[2], v[3])
    let box = [v[0] - (side - v[2]) / 2, v[1] - (side - v[3]) / 2, side, side].map { String(format: "%g", $0) }
    return svg.replacingCharacters(in: match, with: "viewBox=\"\(box.joined(separator: " "))\"")
}

let args = Array(CommandLine.arguments.dropFirst())
if args == ["--list"] {
    let names = (loaded.value(forKey: "allImageNames") as? [String] ?? [])
        .filter { $0.hasPrefix("block-glyph-") }.map { String($0.dropFirst(12)) }
    let have = Set((try? FileManager.default.contentsOfDirectory(atPath: out.path)) ?? [])
    let rexNames = Set(taken.values)
    for name in names.sorted() where !rexNames.contains(name) && !have.contains("\(name)-mark.svg") { print(name) }
    exit(0)
}

let wanted = args.isEmpty ? taken : Dictionary(uniqueKeysWithValues: args.map { ($0, taken[$0] ?? $0) })
var missing = false
for (mark, name) in wanted.sorted(by: { $0.key < $1.key }) {
    guard let svg = glyph(name) else {
        print("\(mark): no block-glyph-\(name) in the catalog")
        missing = true
        continue
    }
    let file = out.appendingPathComponent("\(mark)-mark.svg")
    let old = try? String(contentsOf: file, encoding: .utf8)
    let new = squared(svg)
    try! new.write(to: file, atomically: true, encoding: .utf8)
    print("\(mark): \(old == nil ? "new" : old == new ? "same" : "updated")")
}
exit(missing ? 1 : 0)
