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
        case "asciivideo":
            return try importASCIIFrameVideo(url)
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
        let adapted = Self.adaptGeneratedModule(repaired)
        try adapted.write(to: assets.appendingPathComponent("pw-user-module.js"),
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
            import UserWallpaper from './assets/pw-user-module.js';
            const __pwGeneratedControls = \(generatedControlJSON(in: code));
            const __pwInstance = new UserWallpaper(document.body);
            window.__pwInstance = __pwInstance;
            \(controlBridgeScript)
            \(cameraOrbitScript)
            """
        }
        if let match = code.range(of: named, options: .regularExpression) {
            let className = String(code[match]).replacingOccurrences(of: #"export\s+class\s+"#,
                                                                     with: "",
                                                                     options: .regularExpression)
            return """
            import { \(className) } from './assets/pw-user-module.js';
            const __pwGeneratedControls = \(generatedControlJSON(in: code));
            const __pwInstance = new \(className)(document.body);
            window.__pwInstance = __pwInstance;
            \(controlBridgeScript)
            \(cameraOrbitScript)
            """
        }
        return "import './assets/pw-user-module.js';"
    }

    /// Builds a derived module for generated particle exports. The original
    /// user-module.js remains untouched and can always be recovered or edited.
    ///
    /// Generated exporters recreate `PARAMS` on every frame and call
    /// `addControl(...)` inside the particle loop. We attach the mutable values
    /// to the instance and hoist simple invariant declarations before that loop.
    static func adaptGeneratedModule(_ code: String) -> String {
        let marker = "/* particlewall-generated-adapter-v1 */"
        guard !code.contains(marker),
              code.contains("const PARAMS"),
              code.contains("addControl(") else { return code }

        var adapted = code
        let paramsPattern = #"const\s+PARAMS\s*=\s*(\{[^\n;]*\})\s*;"#
        if let regex = try? NSRegularExpression(pattern: paramsPattern),
           let match = regex.firstMatch(in: adapted,
                                        range: NSRange(adapted.startIndex..., in: adapted)),
           let fullRange = Range(match.range(at: 0), in: adapted),
           let objectRange = Range(match.range(at: 1), in: adapted) {
            let object = String(adapted[objectRange])
            adapted.replaceSubrange(
                fullRange,
                with: """
                \(marker)
                        this.__pwParams = Object.assign(\(object), this.__pwParams || {});
                        const PARAMS = this.__pwParams;
                """
            )
        } else {
            return code
        }

        // The exporter shape uses invariant `const x = addControl(...)` lines
        // inside its main `for (i...)` particle loop. Move only those simple
        // declarations; nested or stateful expressions remain untouched.
        var lines = adapted.components(separatedBy: "\n")
        guard let paramsIndex = lines.firstIndex(where: { $0.contains(marker) }),
              let loopIndex = lines.indices.first(where: { index in
            index > paramsIndex &&
            lines[index].range(of: #"\bfor\s*\(\s*(?:let|var)\s+i\s*="#,
                              options: .regularExpression) != nil
        }) else {
            return adapted
        }

        let declarationPattern =
            #"^\s*(?:const|let)\s+\w+\s*=\s*(?:Math\.(?:floor|round|ceil)\s*\(\s*)?addControl\s*\(\s*["'][^"']+["'][^;]*\)\s*\)?\s*;\s*$"#
        var hoisted: [String] = []
        var indexes: [Int] = []
        for index in lines.indices where index > loopIndex {
            if lines[index].range(of: declarationPattern, options: .regularExpression) != nil {
                hoisted.append(lines[index])
                indexes.append(index)
            }
        }
        for index in indexes.reversed() {
            lines.remove(at: index)
        }
        if !hoisted.isEmpty {
            let indent = String(lines[loopIndex].prefix { $0 == " " || $0 == "\t" })
            let normalized = hoisted.map {
                indent + $0.trimmingCharacters(in: .whitespaces)
            }
            lines.insert(contentsOf: normalized + [""], at: loopIndex)
        }
        return lines.joined(separator: "\n")
    }

    /// Converts the generated `addControl(id, label, min, max, default)` calls
    /// into the same descriptor contract consumed by the native SwiftUI editor.
    static func generatedControlDescriptors(in code: String) -> [[String: Any]] {
        let pattern =
            #"addControl\s*\(\s*["']([^"']+)["']\s*,\s*["']([^"']+)["']\s*,\s*(-?(?:\d+(?:\.\d+)?|\.\d+))\s*,\s*(-?(?:\d+(?:\.\d+)?|\.\d+))\s*,\s*(-?(?:\d+(?:\.\d+)?|\.\d+))\s*\)"#
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return [] }
        let range = NSRange(code.startIndex..., in: code)
        var seen = Set<String>()
        return regex.matches(in: code, range: range).compactMap { match in
            guard let idRange = Range(match.range(at: 1), in: code),
                  let labelRange = Range(match.range(at: 2), in: code),
                  let minRange = Range(match.range(at: 3), in: code),
                  let maxRange = Range(match.range(at: 4), in: code),
                  let defaultRange = Range(match.range(at: 5), in: code) else { return nil }
            let id = String(code[idRange])
            guard seen.insert(id).inserted,
                  let min = Double(code[minRange]),
                  let max = Double(code[maxRange]),
                  let defaultValue = Double(code[defaultRange]),
                  max > min else { return nil }
            let span = max - min
            let step: Double
            if defaultValue.rounded() == defaultValue && min.rounded() == min && max.rounded() == max {
                step = 1
            } else {
                step = span <= 2 ? 0.01 : (span <= 20 ? 0.05 : 0.1)
            }
            return [
                "id": id,
                "label": String(code[labelRange]),
                "category": "Parámetros del modelo",
                "min": min,
                "max": max,
                "step": step,
                "defaultValue": defaultValue
            ]
        }
    }

    private static func generatedControlJSON(in code: String) -> String {
        let reserved = Set([
            "positionX", "positionY", "positionZ", "rotationX", "rotationY",
            "rotationZ", "scale", "speed", "cameraFOV", "bloomStrength",
            "bloomRadius", "bloomThreshold"
        ])
        let descriptors = generatedControlDescriptors(in: code).map { descriptor -> [String: Any] in
            guard let id = descriptor["id"] as? String, reserved.contains(id) else {
                return descriptor
            }
            var namespaced = descriptor
            namespaced["parameterID"] = id
            namespaced["id"] = "model.\(id)"
            return namespaced
        }
        guard JSONSerialization.isValidJSONObject(descriptors),
              let data = try? JSONSerialization.data(withJSONObject: descriptors,
                                                     options: [.sortedKeys]),
              let json = String(data: data, encoding: .utf8) else { return "[]" }
        return json
    }

    /// Standard runtime bridge used by the native customization editor. Common
    /// Three.js properties are detected safely; a module can additionally expose
    /// `particleWallControls` plus `setParticleWallParameter(id, value)`.
    static let controlBridgeScript = """
    (function () {
      const inst = __pwInstance;
      if (!inst) return;

      const root = [inst.mesh, inst.group, inst.root, inst.model]
        .find(value => value && value.position && value.rotation && value.scale);
      const bloom = inst.bloomPass || (inst.composer && Array.isArray(inst.composer.passes)
        ? inst.composer.passes.find(pass => pass && typeof pass.strength === 'number' &&
            typeof pass.radius === 'number' && typeof pass.threshold === 'number')
        : null);
      const controls = [];
      const add = (id, label, category, min, max, step, defaultValue, kind) => {
        const numericMin = Number(min);
        const numericMax = Number(max);
        const numericStep = Number(step);
        const numericDefault = Number(defaultValue);
        if (typeof id !== 'string' || !id || controls.some(control => control.id === id)) return;
        if (![numericMin, numericMax, numericStep, numericDefault].every(Number.isFinite)) return;
        if (numericMax <= numericMin || numericStep <= 0) return;
        controls.push({
          id,
          label: String(label || id),
          category: String(category || 'Modelo'),
          min: numericMin,
          max: numericMax,
          step: numericStep,
          defaultValue: Math.max(numericMin, Math.min(numericMax, numericDefault)),
          kind: kind || undefined
        });
      };

      if (root) {
        add('positionX', 'Posición X', 'Transformación', -100, 100, 0.5, root.position.x);
        add('positionY', 'Posición Y', 'Transformación', -100, 100, 0.5, root.position.y);
        add('positionZ', 'Posición Z', 'Transformación', -100, 100, 0.5, root.position.z);
        add('rotationX', 'Rotación X', 'Transformación', -180, 180, 1, root.rotation.x * 180 / Math.PI);
        add('rotationY', 'Rotación Y', 'Transformación', -180, 180, 1, root.rotation.y * 180 / Math.PI);
        add('rotationZ', 'Rotación Z', 'Transformación', -180, 180, 1, root.rotation.z * 180 / Math.PI);
        add('scale', 'Escala', 'Transformación', 0.1, 4, 0.05, root.scale.x);
      }
      if (typeof inst.speedMult === 'number') {
        add('speed', 'Velocidad', 'Animación', 0, 4, 0.05, inst.speedMult);
      }
      if (inst.camera && typeof inst.camera.fov === 'number') {
        add('cameraFOV', 'Campo de visión', 'Cámara', 20, 120, 1, inst.camera.fov);
      }
      if (bloom) {
        add('bloomStrength', 'Intensidad', 'Bloom', 0, 4, 0.05, bloom.strength);
        add('bloomRadius', 'Radio', 'Bloom', 0, 1, 0.01, bloom.radius);
        add('bloomThreshold', 'Umbral', 'Bloom', 0, 1, 0.01, bloom.threshold);
      }

      const declared = typeof inst.getParticleWallControls === 'function'
        ? inst.getParticleWallControls()
        : inst.particleWallControls;
      if (Array.isArray(declared)) {
        for (const item of declared) {
          if (!item || typeof item.id !== 'string') continue;
          add(item.id, item.label || item.id, item.category || 'Modelo',
              Number(item.min ?? 0), Number(item.max ?? 1), Number(item.step ?? 0.01),
              Number(item.defaultValue ?? 0));
        }
      }
      if (Array.isArray(__pwGeneratedControls)) {
        for (const item of __pwGeneratedControls) {
          add(item.id, item.label, item.category, item.min, item.max, item.step,
              inst.__pwParams && Number.isFinite(Number(inst.__pwParams[item.parameterID || item.id]))
                ? Number(inst.__pwParams[item.parameterID || item.id])
                : item.defaultValue);
        }
      }

      // Appearance: drive a three.js renderer + material when the export exposes
      // one. Mirrors the Metal wallpaper controls (background/particle color,
      // particle size) so web wallpapers are customizable the same way.
      const appearance = (function () {
        const material = root && (root.material
          || (Array.isArray(root.materials) && root.materials.length ? root.materials[0] : null));
        const threeRenderer = inst.renderer
          || (inst.composer && inst.composer.renderer)
          || null;
        if (!material && !threeRenderer) return null;
        return { material, threeRenderer };
      })();
      if (appearance) {
        if (appearance.material && appearance.material.color) {
          add('particleColor', 'Color de partículas', 'Apariencia', 0, 16777215, 1,
              appearance.material.color.getHex(), 'color');
        }
        if (appearance.material && typeof appearance.material.size === 'number') {
          appearance.baseSize = appearance.material.size;
          add('particleSize', 'Tamaño', 'Apariencia', 0.1, 10, 0.05, 1);
        }
        if (appearance.material && typeof appearance.material.opacity === 'number') {
          appearance.baseOpacity = appearance.material.opacity;
          add('brightness', 'Intensidad de puntos', 'Apariencia', 0.25, 10, 0.05, 1);
        }
        if (appearance.threeRenderer
            && typeof appearance.threeRenderer.getClearColor === 'function') {
          let clearHex = 0x000000;
          try { clearHex = appearance.threeRenderer.getClearColor().getHex(); } catch (e) {}
          add('backgroundColor', 'Color del fondo', 'Apariencia', 0, 16777215, 1, clearHex, 'color');
        }
      }

      window.__pwGetControls = function () { return controls; };
      window.__pwApplySettings = function (settings) {
        if (!settings || typeof settings !== 'object') return;
        const number = key => Number.isFinite(Number(settings[key])) ? Number(settings[key]) : null;

        if (root) {
          const x = number('positionX'), y = number('positionY'), z = number('positionZ');
          if (x !== null) root.position.x = x;
          if (y !== null) root.position.y = y;
          if (z !== null) root.position.z = z;
          const rx = number('rotationX'), ry = number('rotationY'), rz = number('rotationZ');
          if (rx !== null) root.rotation.x = rx * Math.PI / 180;
          if (ry !== null) root.rotation.y = ry * Math.PI / 180;
          if (rz !== null) root.rotation.z = rz * Math.PI / 180;
          const scale = number('scale');
          if (scale !== null) root.scale.setScalar(Math.max(0.01, scale));
        }
        const speed = number('speed');
        if (speed !== null && typeof inst.speedMult === 'number') inst.speedMult = speed;
        const fov = number('cameraFOV');
        if (fov !== null && inst.camera && typeof inst.camera.fov === 'number') {
          inst.camera.fov = fov;
          if (typeof inst.camera.updateProjectionMatrix === 'function') inst.camera.updateProjectionMatrix();
        }
        if (bloom) {
          const strength = number('bloomStrength');
          const radius = number('bloomRadius');
          const threshold = number('bloomThreshold');
          if (strength !== null) bloom.strength = strength;
          if (radius !== null) bloom.radius = radius;
          if (threshold !== null) bloom.threshold = threshold;
        }
        if (typeof inst.setParticleWallParameter === 'function') {
          const builtIn = new Set(['positionX', 'positionY', 'positionZ', 'rotationX',
            'rotationY', 'rotationZ', 'scale', 'speed', 'cameraFOV',
            'bloomStrength', 'bloomRadius', 'bloomThreshold']);
          for (const [key, value] of Object.entries(settings)) {
            if (!builtIn.has(key) && Number.isFinite(Number(value))) {
              inst.setParticleWallParameter(key, Number(value));
            }
          }
        }
        if (Array.isArray(__pwGeneratedControls)) {
          inst.__pwParams = inst.__pwParams || {};
          for (const item of __pwGeneratedControls) {
            const value = number(item.id);
            if (value !== null) inst.__pwParams[item.parameterID || item.id] = value;
          }
        }
        if (appearance) {
          const bg = number('backgroundColor');
          if (bg !== null && appearance.threeRenderer
              && typeof appearance.threeRenderer.setClearColor === 'function') {
            appearance.threeRenderer.setClearColor(bg, 1);
          }
          const pc = number('particleColor');
          if (pc !== null && appearance.material && appearance.material.color) {
            appearance.material.color.setHex(pc);
          }
          const ps = number('particleSize');
          if (ps !== null && appearance.material
              && typeof appearance.material.size === 'number') {
            appearance.material.size = Math.max(0.01,
                (appearance.baseSize || 1) * ps);
          }
          const br = number('brightness');
          if (br !== null && appearance.material
              && typeof appearance.material.opacity === 'number') {
            appearance.material.opacity = Math.max(0, Math.min(1,
                (appearance.baseOpacity || 1) * br));
          }
        }
        window.__pwControlValues = Object.assign({}, window.__pwControlValues || {}, settings);
      };
    })();
    """

    /// Slow camera orbit: keeps static formations (grids, cubes) alive and shows
    /// them in 3D. Runs through requestAnimationFrame, so the injected rAF patch
    /// applies the global pause and FPS cap to it too.
    static let cameraOrbitScript = """
    (function () {
      const inst = __pwInstance;
      if (!inst || !inst.camera || !inst.camera.position || !inst.camera.lookAt) return;
      const cam = inst.camera;
      const controlRoot = [inst.mesh, inst.group, inst.root, inst.model]
        .find(value => value && value.position);
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
        if (controlRoot) {
          cam.lookAt(controlRoot.position);
        } else {
          cam.lookAt(0, 0, 0);
        }
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
            let adaptedURL = wallpaper.folderURL.appendingPathComponent("assets/pw-user-module.js")
            let adapted = Self.adaptGeneratedModule(code)
            if (try? String(contentsOf: adaptedURL, encoding: .utf8)) != adapted {
                try? adapted.write(to: adaptedURL, atomically: true, encoding: .utf8)
            }
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

    private func importASCIIFrameVideo(_ url: URL) throws -> Wallpaper {
        let staging = try makeStagingFolder()
        defer { try? fm.removeItem(at: staging) }
        let destination = staging.appendingPathComponent("clip.asciivideo")
        try fm.copyItem(at: url, to: destination)
        let placeholder = """
        <!doctype html>
        <html><body style=\"margin:0;background:#000\"></body></html>
        """
        try placeholder.write(to: staging.appendingPathComponent("index.html"),
                             atomically: true, encoding: .utf8)
        return try LibraryManager.shared.add(
            folderWithContents: staging,
            name: url.deletingPathExtension().lastPathComponent,
            source: "ascii-video",
            renderer: .asciiVideo
        )
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
