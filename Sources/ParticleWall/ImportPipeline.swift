import Foundation
import UniformTypeIdentifiers

enum ImportError: LocalizedError {
    case unsupportedType(String)
    case noIndexHTML
    case unzipFailed(String)
    case templateMissing

    var errorDescription: String? {
        switch self {
        case .unsupportedType(let ext): return "Tipo de archivo no soportado: .\(ext)"
        case .noIndexHTML: return "No se encontró un index.html (ni un único .html) en el contenido importado."
        case .unzipFailed(let message): return "No se pudo descomprimir el .zip: \(message)"
        case .templateMissing: return "Falta template.html en el bundle de la app."
        }
    }
}

/// Turns user input (.html file, JS snippet, folder, .zip) into a normalized
/// wallpaper folder: index.html + assets/ (three.min.js always local).
final class ImportPipeline {
    static let shared = ImportPipeline()
    private let fm = FileManager.default
    private init() {}

    // MARK: - Entry points

    /// Import a file-system item chosen via open panel or drag & drop.
    @discardableResult
    func importItem(at url: URL) throws -> Wallpaper {
        var isDirectory: ObjCBool = false
        fm.fileExists(atPath: url.path, isDirectory: &isDirectory)

        if isDirectory.boolValue {
            return try importFolder(url, name: url.lastPathComponent, source: "folder")
        }
        switch url.pathExtension.lowercased() {
        case "html", "htm":
            return try importHTMLFile(url)
        case "zip":
            return try importZip(url)
        case "js", "txt":
            let snippet = try String(contentsOf: url, encoding: .utf8)
            return try importSnippet(snippet, name: url.deletingPathExtension().lastPathComponent)
        default:
            throw ImportError.unsupportedType(url.pathExtension)
        }
    }

    /// Wrap a raw Vanilla JS / Three.js snippet in the bundled template.
    /// ES-module exports (import/export statements) get the module template with
    /// a local import map; plain snippets get the classic UMD template.
    @discardableResult
    func importSnippet(_ snippet: String, name: String) throws -> Wallpaper {
        if Self.isESModule(snippet) {
            return try importModuleSnippet(snippet, name: name)
        }
        guard let templateURL = Bundle.module.url(forResource: "template", withExtension: "html") else {
            throw ImportError.templateMissing
        }
        let template = try String(contentsOf: templateURL, encoding: .utf8)
        let html = template.replacingOccurrences(of: "/*__PW_SNIPPET__*/", with: snippet)

        let staging = try makeStagingFolder()
        defer { try? fm.removeItem(at: staging) }
        try html.write(to: staging.appendingPathComponent("index.html"), atomically: true, encoding: .utf8)
        try installLocalThree(in: staging)
        return try LibraryManager.shared.add(folderWithContents: staging, name: name, source: "vanilla-js")
    }

    // MARK: - ES module snippets

    static func isESModule(_ code: String) -> Bool {
        let pattern = #"(?m)^\s*(import\s.*from\s|import\s*["']|export\s+(default\s+)?(class|function|const|let|var)\s)"#
        return code.range(of: pattern, options: .regularExpression) != nil
    }

    private func importModuleSnippet(_ snippet: String, name: String) throws -> Wallpaper {
        guard let templateURL = Bundle.module.url(forResource: "template-module", withExtension: "html"),
              let esmFolder = Bundle.module.url(forResource: "three-esm", withExtension: nil) else {
            throw ImportError.templateMissing
        }

        let repaired = Self.dedupeLetDeclarations(in: snippet)

        let staging = try makeStagingFolder()
        defer { try? fm.removeItem(at: staging) }
        let assets = staging.appendingPathComponent("assets", isDirectory: true)
        try fm.createDirectory(at: assets, withIntermediateDirectories: true)
        try repaired.write(to: assets.appendingPathComponent("user-module.js"),
                           atomically: true, encoding: .utf8)
        try fm.copyItem(at: esmFolder, to: assets.appendingPathComponent("three-esm"))

        let template = try String(contentsOf: templateURL, encoding: .utf8)
        let html = template.replacingOccurrences(of: "/*__PW_MODULE_BOOTSTRAP__*/",
                                                 with: Self.moduleBootstrap(for: repaired))
        try html.write(to: staging.appendingPathComponent("index.html"), atomically: true, encoding: .utf8)

        return try LibraryManager.shared.add(folderWithContents: staging, name: name, source: "es-module")
    }

    /// Some exporters emit the same `let x = …;` stub twice in one scope, which is
    /// a SyntaxError. Keep the first occurrence, comment out identical repeats.
    static func dedupeLetDeclarations(in code: String) -> String {
        var seen = Set<String>()
        return code.components(separatedBy: "\n").map { line in
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            guard trimmed.hasPrefix("let "), trimmed.hasSuffix(";") else { return line }
            if seen.contains(trimmed) {
                return line.replacingOccurrences(of: trimmed, with: "// pw-dedup: \(trimmed)")
            }
            seen.insert(trimmed)
            return line
        }.joined(separator: "\n")
    }

    /// Import the user module and instantiate its exported class with the body
    /// element as container (the shape the particle-export tools produce).
    /// The exports have no camera animation of their own, so the bootstrap adds
    /// a slow orbit around the origin when the instance exposes a camera.
    static func moduleBootstrap(for code: String) -> String {
        let named = #"export\s+class\s+(\w+)"#
        let defaulted = #"export\s+default\s+class\s+(\w+)?"#

        if code.range(of: defaulted, options: .regularExpression) != nil {
            return """
            import UserWallpaper from './assets/user-module.js';
            const __pwInstance = new UserWallpaper(document.body);
            \(cameraOrbitScript)
            """
        }
        if let match = code.range(of: named, options: .regularExpression) {
            let className = String(code[match]).replacingOccurrences(of: #"export\s+class\s+"#,
                                                                     with: "",
                                                                     options: .regularExpression)
            return """
            import { \(className) } from './assets/user-module.js';
            const __pwInstance = new \(className)(document.body);
            \(cameraOrbitScript)
            """
        }
        return "import './assets/user-module.js';"
    }

    /// Slow camera orbit: keeps static formations (grids, cubes) alive and shows
    /// them in 3D. Runs through requestAnimationFrame, so the injected rAF patch
    /// applies the global pause and FPS cap to it too.
    static let cameraOrbitScript = """
    (function () {
      const inst = __pwInstance;
      if (!inst || !inst.camera || !inst.camera.position || !inst.camera.lookAt) return;
      const cam = inst.camera;
      const p = cam.position;
      const R = Math.sqrt(p.x * p.x + p.y * p.y + p.z * p.z) || 100;
      const el0 = Math.asin(Math.max(-1, Math.min(1, p.y / R)));
      let angle = Math.atan2(p.x, p.z);
      function orbit() {
        requestAnimationFrame(orbit);
        if (window.__pwPaused) return;
        angle += 0.0015;
        const el = el0 + Math.sin(angle * 0.7) * 0.15;
        cam.position.set(
          R * Math.cos(el) * Math.sin(angle),
          R * Math.sin(el),
          R * Math.cos(el) * Math.cos(angle)
        );
        cam.lookAt(0, 0, 0);
      }
      requestAnimationFrame(orbit);
    })();
    """

    /// Regenerate index.html of existing es-module wallpapers from the current
    /// template + bootstrap. Idempotent: rewrites only when the output differs.
    func upgradeModuleWallpapers() {
        guard let templateURL = Bundle.module.url(forResource: "template-module", withExtension: "html"),
              let template = try? String(contentsOf: templateURL, encoding: .utf8) else { return }
        for wallpaper in LibraryManager.shared.wallpapers where wallpaper.manifest.source == "es-module" {
            let moduleURL = wallpaper.folderURL.appendingPathComponent("assets/user-module.js")
            guard let code = try? String(contentsOf: moduleURL, encoding: .utf8) else { continue }
            let html = template.replacingOccurrences(of: "/*__PW_MODULE_BOOTSTRAP__*/",
                                                     with: Self.moduleBootstrap(for: code))
            let indexURL = wallpaper.folderURL.appendingPathComponent("index.html")
            if (try? String(contentsOf: indexURL, encoding: .utf8)) != html {
                try? html.write(to: indexURL, atomically: true, encoding: .utf8)
                LibraryManager.shared.regenerateThumbnail(wallpaper)
            }
        }
    }

    // MARK: - Kinds

    private func importHTMLFile(_ url: URL) throws -> Wallpaper {
        let staging = try makeStagingFolder()
        defer { try? fm.removeItem(at: staging) }
        try fm.copyItem(at: url, to: staging.appendingPathComponent("index.html"))
        try localizeExternalScripts(in: staging)
        return try LibraryManager.shared.add(folderWithContents: staging,
                                             name: url.deletingPathExtension().lastPathComponent,
                                             source: "html")
    }

    private func importFolder(_ url: URL, name: String, source: String) throws -> Wallpaper {
        let staging = try makeStagingFolder()
        defer { try? fm.removeItem(at: staging) }
        for item in try fm.contentsOfDirectory(at: url, includingPropertiesForKeys: nil) {
            try fm.copyItem(at: item, to: staging.appendingPathComponent(item.lastPathComponent))
        }
        try normalizeIndexHTML(in: staging)
        try localizeExternalScripts(in: staging)
        return try LibraryManager.shared.add(folderWithContents: staging, name: name, source: source)
    }

    private func importZip(_ url: URL) throws -> Wallpaper {
        let extracted = try makeStagingFolder()
        defer { try? fm.removeItem(at: extracted) }

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/ditto")
        process.arguments = ["-x", "-k", url.path, extracted.path]
        let stderrPipe = Pipe()
        process.standardError = stderrPipe
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            let data = stderrPipe.fileHandleForReading.readDataToEndOfFile()
            throw ImportError.unzipFailed(String(data: data, encoding: .utf8) ?? "ditto exit \(process.terminationStatus)")
        }

        // Zips often wrap everything in a single top-level folder — unwrap it.
        var contentRoot = extracted
        let entries = try fm.contentsOfDirectory(at: extracted, includingPropertiesForKeys: nil,
                                                 options: [.skipsHiddenFiles])
            .filter { $0.lastPathComponent != "__MACOSX" }
        if entries.count == 1, (try? entries[0].resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true {
            contentRoot = entries[0]
        }
        return try importFolder(contentRoot,
                                name: url.deletingPathExtension().lastPathComponent,
                                source: "zip")
    }

    // MARK: - Normalization

    /// Ensure the folder has an index.html; accept a single *.html as substitute.
    private func normalizeIndexHTML(in folder: URL) throws {
        let indexURL = folder.appendingPathComponent("index.html")
        if fm.fileExists(atPath: indexURL.path) { return }
        let htmlFiles = try fm.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)
            .filter { ["html", "htm"].contains($0.pathExtension.lowercased()) }
        guard htmlFiles.count == 1 else { throw ImportError.noIndexHTML }
        try fm.moveItem(at: htmlFiles[0], to: indexURL)
    }

    /// Rewrite external <script src="https://..."> references so wallpapers work
    /// offline (the runtime WebView blocks all non-local requests anyway).
    /// Any three.js CDN reference is replaced by the bundled copy; other scripts
    /// are downloaded into assets/ when possible.
    private func localizeExternalScripts(in folder: URL) throws {
        let indexURL = folder.appendingPathComponent("index.html")
        var html = try String(contentsOf: indexURL, encoding: .utf8)

        let pattern = #"src=["'](https?://[^"']+)["']"#
        let regex = try NSRegularExpression(pattern: pattern, options: [.caseInsensitive])
        let matches = regex.matches(in: html, range: NSRange(html.startIndex..., in: html)).reversed()
        guard !matches.isEmpty else { return }

        let assets = folder.appendingPathComponent("assets", isDirectory: true)
        try fm.createDirectory(at: assets, withIntermediateDirectories: true)
        var threeInstalled = false

        for match in matches {
            guard let range = Range(match.range(at: 1), in: html),
                  let remote = URL(string: String(html[range])) else { continue }

            var localName: String?
            if remote.lastPathComponent.lowercased().contains("three") {
                if !threeInstalled {
                    try installLocalThree(in: folder)
                    threeInstalled = true
                }
                localName = "three.min.js"
            } else if let downloaded = downloadSync(remote) {
                let name = remote.lastPathComponent.isEmpty ? "lib-\(UUID().uuidString.prefix(6)).js"
                                                            : remote.lastPathComponent
                let dest = assets.appendingPathComponent(String(name))
                try? fm.removeItem(at: dest)
                try? fm.moveItem(at: downloaded, to: dest)
                localName = String(name)
            } else {
                NSLog("ParticleWall: could not localize \(remote); it will be blocked at runtime")
            }

            if let localName, let fullRange = Range(match.range, in: html) {
                html.replaceSubrange(fullRange, with: "src=\"assets/\(localName)\"")
            }
        }
        try html.write(to: indexURL, atomically: true, encoding: .utf8)
    }

    /// Copy the bundled three.min.js into <folder>/assets/.
    private func installLocalThree(in folder: URL) throws {
        guard let bundled = Bundle.module.url(forResource: "three.min", withExtension: "js") else { return }
        let assets = folder.appendingPathComponent("assets", isDirectory: true)
        try fm.createDirectory(at: assets, withIntermediateDirectories: true)
        let dest = assets.appendingPathComponent("three.min.js")
        if !fm.fileExists(atPath: dest.path) {
            try fm.copyItem(at: bundled, to: dest)
        }
    }

    private func downloadSync(_ url: URL, timeout: TimeInterval = 15) -> URL? {
        var result: URL?
        let semaphore = DispatchSemaphore(value: 0)
        let task = URLSession.shared.downloadTask(with: url) { location, response, _ in
            if let location,
               let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) {
                let temp = FileManager.default.temporaryDirectory
                    .appendingPathComponent("pw-dl-\(UUID().uuidString)")
                try? FileManager.default.moveItem(at: location, to: temp)
                result = temp
            }
            semaphore.signal()
        }
        task.resume()
        _ = semaphore.wait(timeout: .now() + timeout)
        return result
    }

    private func makeStagingFolder() throws -> URL {
        let url = fm.temporaryDirectory.appendingPathComponent("pw-import-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }
}
