# ParticleWall

App de barra de menú para macOS que renderiza animaciones de partículas (Vanilla JS / Three.js)
como fondo de pantalla animado, detrás de los íconos del escritorio.

## Compilar e instalar

```bash
./build-app.sh
open ~/Applications/ParticleWall.app
```

El script compila el paquete Swift (release), ensambla `build/ParticleWall.app`, lo firma
ad-hoc y lo instala en `~/Applications`.

> **Importante:** no ejecutes la app desde `Desktop/`, `Documents/` o `Downloads/`.
> macOS (TCC) bloquea la lectura de los recursos del bundle con un diálogo de permisos
> y la app se queda colgada al arrancar. Por eso se instala en `~/Applications`.
> Si alguna vez queda colgada por un diálogo de permisos pendiente:
> `tccutil reset All com.particlewall.app`

## Uso

- **Ícono ✨ en la barra de menú** (sin Dock):
  - Click izquierdo → galería de wallpapers.
  - Click derecho → menú rápido: Pausar/Reanudar, Power Save, Galería, Ajustes, Salir.
- **Importar**: botón "Importar" o arrastra a la ventana de galería:
  - `.html` completo (referencias CDN a three.js se reescriben a copia local),
  - snippet `.js` de Vanilla JS/Three.js (se envuelve en un template con scene/camera/renderer
    ya montados; define `pwUpdate(t)` para lógica por frame),
  - **módulo ES** `.js` (`import * as THREE from 'three'` + `export class Foo`): se detecta
    automáticamente, se sirve con import map hacia una copia local de three 0.160 ESM
    (incluye `examples/jsm` de postprocessing: EffectComposer, UnrealBloomPass, etc.)
    y se instancia la clase exportada con `document.body` como container,
  - carpeta o `.zip` con `index.html` + assets.
- **Aplicar**: click en la tarjeta. Con varios monitores, selector "Aplicar en: …".
- **FPS**: global en Ajustes o en el menú rápido del ícono (click derecho → Límite de FPS);
  por wallpaper en el context menu de su tarjeta (Global/15/30/60/120, guardado en su
  manifest). El cap efectivo es el menor de los dos no-cero.
- **Ajustes**: iniciar al arrancar sesión, pausar con batería, Power Save (congela frame),
  límite de FPS (default 30), resolución de render (default 1.5x), abrir carpeta de wallpapers.
- **CLI**: `ParticleWall --import <ruta>` importa desde terminal; `--diag` loguea FPS reales
  y devicePixelRatio de cada pantalla a los ~8s.

### Sincronización con el wallpaper del sistema

La barra de menú de macOS toma su tinte del wallpaper *del sistema*, no de la ventana de
ParticleWall. Al aplicar un wallpaper, la app fija además el wallpaper del sistema al
thumbnail correspondiente (por pantalla) para que no queden colores residuales del fondo
anterior en la barra de menú, el reloj ni las transiciones de Space.

### Órbita de cámara (módulos ES)

Los exports de módulo ES no traen animación de cámara (formaciones estáticas quedan
congeladas de frente). El bootstrap agrega una órbita lenta (~1 vuelta / 2.5 min) alrededor
del origen cuando la clase exportada expone `.camera`; respeta pausa y FPS cap. Los
wallpapers es-module existentes se regeneran automáticamente al arrancar (upgrade
idempotente de su index.html).

## Arquitectura

```
Sources/ParticleWall/
├── main.swift              bootstrap AppKit (accessory, sin Dock)
├── AppDelegate.swift       NSStatusItem + ventanas de galería/ajustes
├── WallpaperManager.swift  1 ventana por pantalla, persistencia por display-UUID
├── WallpaperWindow.swift   NSWindow nivel desktop, clicks atraviesan
├── WebViewFactory.swift    WKWebView + parche de requestAnimationFrame (__pwPaused/__pwFPSCap)
│                           + bloqueo de navegación no-local
├── LibraryManager.swift    ~/Library/Application Support/ParticleWall/wallpapers/<uuid>/
├── ImportPipeline.swift    html/js/zip/carpeta → carpeta normalizada con three.js local
├── ThumbnailGenerator.swift snapshot en ventana casi invisible (WebKit suspende rAF
│                           en ventanas ocluidas — no puede ser offscreen)
├── PowerManager.swift      pausa por lock/sleep/batería/oclusión + kick tras unlock
├── GalleryView.swift       grid SwiftUI con preview en vivo
└── SettingsView.swift      SMAppService, energía, FPS
```

Wallpapers son HTML arbitrario: el `WKNavigationDelegate` cancela toda navegación que no sea
`file://` dentro de la carpeta del wallpaper, y el parche de `requestAnimationFrame` inyectado
a document-start permite pausar y limitar FPS sin cooperación del contenido.
