# Plan: App de fondos animados (partículas Three.js) para macOS

## Contexto

El usuario tiene animaciones de partículas exportables como Vanilla JS / Three.js (código que renderiza un swarm de partículas en un canvas). El objetivo es una app nativa de macOS que renderice estos archivos HTML como fondo de pantalla animado (detrás de los íconos del escritorio), con una galería para importar y cambiar de fondo.

- Máquina: MacBook Pro M4 (Apple Silicon), macOS reciente
- Stack: Swift + SwiftUI/AppKit, Xcode
- Inspiración de referencia: Plash (sindresorhus/Plash), Waraq (bahamut42/waraq), LiveWallpaperMacOS (thusvill)
- Nombre de trabajo: **ParticleWall** (cambiable)

## Arquitectura general

```
ParticleWall.app (menu bar app, sin Dock)
├── WallpaperWindowController   → 1 NSWindow por pantalla, a nivel de escritorio
│   └── WKWebView               → carga el index.html del wallpaper activo
├── LibraryManager              → CRUD de wallpapers en Application Support
├── PowerManager                → pausas (lock screen, batería, fullscreen)
├── MenuBarUI (SwiftUI)         → galería en popover/ventana + settings
└── ImportPipeline              → envuelve exports de Vanilla JS en template HTML
```

### Mecanismo clave: ventana a nivel de escritorio

```swift
let window = NSWindow(
    contentRect: screen.frame,
    styleMask: [.borderless],
    backing: .buffered,
    defer: false,
    screen: screen
)
window.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopWindow)))
window.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
window.isOpaque = true
window.backgroundColor = .black
window.ignoresMouseEvents = true   // clicks pasan al escritorio
window.hasShadow = false
```

Esto coloca la ventana DETRÁS de los íconos del Finder pero DELANTE del wallpaper estático del sistema. Los clicks deben atravesar (`ignoresMouseEvents = true`).

### WKWebView

```swift
let config = WKWebViewConfiguration()
config.preferences.setValue(true, forKey: "developerExtrasEnabled") // solo debug
let webView = WKWebView(frame: window.contentView!.bounds, configuration: config)
webView.autoresizingMask = [.width, .height]
// Cargar con acceso a archivos locales:
webView.loadFileURL(indexURL, allowingReadAccessTo: wallpaperFolderURL)
```

Notas:
- Fondo del webView en negro o transparente según el wallpaper.
- Inyectar JS para control de FPS y pausa (ver PowerManager).
- Desactivar scroll, selección y context menu vía CSS/JS inyectado.

## Estructura de la biblioteca

```
~/Library/Application Support/ParticleWall/
└── wallpapers/
    └── <uuid>/
        ├── manifest.json    { "name": "...", "createdAt": "...", "fps": 60, "source": "vanilla-js" }
        ├── index.html       (el wallpaper renderizable)
        ├── assets/          (three.min.js local, texturas, etc.)
        └── thumbnail.png    (captura generada al importar)
```

Regla: **Three.js se guarda local** (no CDN) para que el wallpaper funcione sin internet. Al importar, si el HTML referencia un CDN, descargar la lib y reescribir el `<script src>`.

## Pipeline de importación

Entradas aceptadas:
1. **Archivo .html** completo → se copia tal cual.
2. **Snippet de Vanilla JS / Three.js** (lo que exporta la herramienta del usuario, ej. lógica de posiciones con `Math.acos`, `target.set(...)`, `color.setHex(0x00ff88)`) → se envuelve en un template.
3. **Carpeta o .zip** con html + assets → se descomprime y valida que exista un index.html.

Template para snippets (esqueleto):

```html
<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  html, body { margin: 0; padding: 0; overflow: hidden; background: #000; }
  canvas { display: block; }
  * { user-select: none; -webkit-user-select: none; }
</style>
</head>
<body>
<script src="assets/three.min.js"></script>
<script>
  // Boilerplate: scene, camera, renderer full-screen, resize handler,
  // sistema de partículas (BufferGeometry + PointsMaterial),
  // loop con requestAnimationFrame respetando window.__pwPaused y window.__pwFPSCap
  // === SNIPPET DEL USUARIO SE INSERTA AQUÍ (lógica de formación/animación) ===
</script>
</body>
</html>
```

Al importar, generar el thumbnail: cargar el HTML en un WKWebView offscreen 2s y hacer `takeSnapshot`.

## PowerManager (crítico para batería)

| Evento | Acción |
|---|---|
| Pantalla bloqueada / sesión inactiva | pausar render (inyectar `window.__pwPaused = true`) |
| `NSWorkspace.screensDidSleepNotification` | pausar |
| En batería (si opción activada) | pausar o congelar frame |
| App fullscreen delante (opcional, fase 7) | pausar |
| Ocluido (`window.occlusionState`) | pausar |

Detección de batería: `ProcessInfo.processInfo.isLowPowerModeEnabled` + IOKit (`IOPSCopyPowerSourcesInfo`) para saber si está desenchufado.

El JS inyectado en cada wallpaper debe respetar:
```js
function loop(t) {
  requestAnimationFrame(loop);
  if (window.__pwPaused) return;
  // throttle a __pwFPSCap si está definido
  render(t);
}
```

## UI (menu bar app)

**Sin ícono en el Dock** (`LSUIElement = YES` en Info.plist). Todo vive en la barra de menú.

### 1. Ícono de barra de menú
- Ícono simple (SF Symbol tipo `sparkles` o logo propio).
- Click izquierdo → abre el popover/ventana de galería.
- Click derecho → menú rápido: Play/Pause, Power Save, Abrir galería, Quit.

### 2. Galería (ventana SwiftUI, ~720×480)
- **Grid de tarjetas** con el thumbnail de cada wallpaper, nombre debajo.
- Tarjeta activa marcada con borde de acento.
- **Click en tarjeta → se aplica al instante** en la(s) pantalla(s).
- Hover → botón de preview (reproduce el thumbnail animado en un mini WKWebView).
- Botón **"+ Importar"** (arriba a la derecha): abre NSOpenPanel (html/zip/carpeta) y también aceptar **drag & drop** de archivos sobre la ventana.
- Barra superior: buscador por nombre + selector de pantalla si hay multi-monitor ("Aplicar en: Todas / Pantalla 1 / Pantalla 2").
- Click derecho en tarjeta: Renombrar, Regenerar thumbnail, Mostrar en Finder, Eliminar.

### 3. Settings (pestaña o ventana aparte)
- Toggle: Iniciar al arrancar sesión (SMAppService.mainApp.register()).
- Toggle: Pausar con batería.
- Toggle: Power Save (congela un frame → CPU/GPU a ~0%).
- Slider: límite de FPS (30/60/120).
- Botón: Abrir carpeta de wallpapers.

### Flujo del usuario final
1. Exporta su formación de partículas como Vanilla JS desde su herramienta.
2. Abre la galería de ParticleWall → arrastra el archivo → la app lo envuelve, genera thumbnail y lo agrega al grid.
3. Click en la tarjeta → el escritorio cambia al instante.
4. La app queda en la barra de menú consumiendo casi nada; pausa sola al bloquear pantalla o desenchufar.

## Fases de desarrollo (para Claude Code)

**Fase 1 — Esqueleto (½ día)**
- Proyecto Xcode: app SwiftUI, LSUIElement, MenuBarExtra.
- Sin sandbox al inicio (simplifica acceso a archivos); evaluar sandbox al final.
- Criterio de éxito: ícono en barra de menú con menú Quit.

**Fase 2 — Ventana de escritorio + WebView (1 día)**
- WallpaperWindowController con la config de arriba, un HTML de prueba con partículas hardcodeado.
- Criterio: animación visible detrás de los íconos, clicks atraviesan, sobrevive a Mission Control y cambio de Space.

**Fase 3 — Multi-pantalla (½ día)**
- Una ventana por NSScreen; reaccionar a `NSApplication.didChangeScreenParametersNotification` (conectar/desconectar monitor).

**Fase 4 — LibraryManager + ImportPipeline (1 día)**
- Estructura de carpetas, manifest, template wrapper, three.min.js embebido en el bundle, generación de thumbnails.

**Fase 5 — Galería UI (1 día)**
- Grid SwiftUI, aplicar al click, importar por panel y drag & drop, acciones de context menu.

**Fase 6 — PowerManager (½ día)**
- Pausas por lock/sleep/batería/oclusión, Power Save, FPS cap.

**Fase 7 — Pulido (½–1 día)**
- Launch at login, persistir wallpaper activo entre reinicios (UserDefaults con uuid por pantalla), manejo de errores de import, empaquetado (.app zip; sin notarizar por ahora → instrucción de right-click → Open).

## Riesgos y decisiones ya tomadas

- **No usar la galería nativa de macOS**: eso requiere transcodificar a video HEVC (Opción B, fuera de alcance de esta app). Esta app renderiza en vivo.
- **WKWebView vs Metal nativo**: WKWebView elegido porque los exports ya son JS/Three.js; portar a Metal sería reescribir todo.
- **Consumo**: render continuo usa GPU; mitigado con PowerManager. En M4 un swarm simple debería quedar en ~1-5% GPU.
- **Bug conocido del ecosistema**: al desbloquear la pantalla algunos renders quedan congelados → al recibir `sessionDidBecomeActive`, forzar `window.__pwPaused = false` + un `webView.evaluateJavaScript` de "kick" (y como fallback, reload del webView).
- **Seguridad**: los wallpapers son HTML arbitrario ejecutándose local. Desactivar navegación externa en el WKWebView (WKNavigationDelegate que cancela todo request no-local).

## Primer prompt sugerido para Claude Code

"Lee PLAN.md. Implementa la Fase 1 y la Fase 2: crea el proyecto Xcode (app menu bar SwiftUI, LSUIElement) y la ventana a nivel de escritorio con un WKWebView que cargue un HTML de prueba con partículas Three.js incluido en el bundle. Al terminar, dame las instrucciones para compilar y probar desde Xcode."
