# ParticleWall

App de barra de menú para macOS que renderiza animaciones de partículas (Vanilla JS / Three.js)
como fondo de pantalla animado, detrás de los íconos del escritorio.

El estado completo de funcionalidades, decisiones, pruebas y trabajo pendiente
se mantiene en [`PROJECT_PROGRESS.md`](PROJECT_PROGRESS.md).

## Compilar e instalar

```bash
./build-app.sh
open ~/Applications/ParticleWall.app
```

El script compila el paquete Swift (release), ensambla `build/ParticleWall.app`, lo firma
ad-hoc y lo instala en `~/Applications`.

El demo incluido usa Metal de forma nativa. Los fondos HTML/JavaScript siguen
usando WebKit como ruta de compatibilidad.

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
  - carpeta o `.zip` con `index.html` + assets,
  - `.asciivideo` preprocesado, renderizado nativamente con Metal sin reproducir video.
- **Aplicar**: click en la tarjeta. Con varios monitores, selector "Aplicar en: …".
- **Fondos incluidos protegidos**: los wallpapers distribuidos con la app muestran
  un candado y no pueden eliminarse. Si faltan por una versión anterior, se
  reinstalan automáticamente sin tocar los fondos importados.
- **Opciones y personalización**: pasa el cursor sobre una tarjeta y pulsa `•••`
  en la esquina inferior derecha. En módulos ES
  compatibles permite mover el modelo en X/Y/Z, rotarlo, escalarlo y ajustar cámara,
  velocidad, bloom o parámetros propios. “Guardar configuración” conserva una versión
  independiente por wallpaper y pantalla; “Restaurar valores predeterminados” elimina
  los ajustes personalizados y vuelve a los valores declarados por el modelo.
  La biblioteca personal de perfiles de color permite guardar juntos el fondo y
  las partículas, aplicar ese par a cualquier wallpaper compatible y eliminarlo
  cuando ya no se necesite.
  En fondos Metal, la sección “Grafo” conecta una muestra acotada de nodos por
  proximidad visual; permite ajustar distancia, conexiones por nodo e intensidad
  sin consumir recursos cuando está desactivada. Tanto la intensidad de puntos
  como la intensidad de líneas admiten valores de hasta 10.
  Los ocho fondos Metal incluidos permiten además elegir color de fondo, color
  de partículas, tamaño y brillo, todos con vista previa en vivo. La sección
  “Pantalla” permite activar el ajuste al tamaño del monitor y modificar por
  separado los límites horizontal y vertical.
- **FPS**: global en Ajustes o en el menú rápido del ícono (click derecho → Límite de FPS);
  por wallpaper desde el botón `•••` de su tarjeta (Global/15/30/60/120, guardado en su
  manifest). El cap efectivo es el menor de los dos no-cero.
- **Ajustes**: iniciar al arrancar sesión, pausar con batería, Power Save, límite de FPS
  (default 30), calidad adaptativa, resolución de render (default 1.5x), abrir carpeta.
- **CLI**: `ParticleWall --import <ruta>` importa desde terminal; `--diag` loguea FPS reales,
  devicePixelRatio, escala y controles detectados de cada pantalla a los ~8s;
  `--renderer-smoke-test` prueba las ocho variantes Metal, `--apply-bundled-default` activa el demo
  nativo y `--powersave-test` prueba deep sleep.

### Optimizaciones de consumo

- **Reposo profundo**: Power Save y la pausa por batería congelan el último frame en un
  `NSImageView`; lock, screen sleep y sesión inactiva usan el thumbnail. En ambos casos
  se destruye por completo el `WKWebView` y se vuelve a crear solo al reanudar.
- **Throttle con setTimeout**: los ticks saltados por el FPS cap duermen con `setTimeout`
  en vez de re-encolar rAF — WebKit despierta ~cap veces/s (no 120/s en ProMotion) y el
  panel puede bajar su refresh.
- **Escala de render**: además del clamp de `devicePixelRatio`, con escala <2x los exports
  ven `innerWidth/innerHeight` reducidos (×0.75 en Media, ×0.5 en Baja) y el canvas se
  estira a pantalla completa por CSS — 44-58% menos píxeles también para renderers que
  ignoran el pixel ratio.
- **Pausa por oclusión**: si ventanas/apps fullscreen tapan el escritorio, el wallpaper se
  pausa solo (por eso el CPU en uso normal es ~0).
- **Ocho fondos incluidos en Metal**: `Ondas Paramétricas`, `Vórtice Gemelo`,
  `Flor Orbital`, `Roseta Hexagonal`, `Lluvia de Ruido`, `Espiral Prima` y
  `Órbita Toroidal`, además de `Anillos Cromáticos`, reproducen fórmulas
  creativas directamente en GPU. La
  roseta genera su simetría sin copiar el framebuffer; la lluvia conserva
  posiciones y estelas en buffers de cómputo; la espiral sube únicamente los
  78.498 primos, no el millón de marcadores, y la órbita usa una nube de puntos
  acotada. Los anillos generan 6.225 puntos base y ocho muestras temporales
  en vez de aplicar desenfoque a todo el framebuffer.
- **Calidad adaptativa**: si un callback excede de forma sostenida su presupuesto,
  el renderer web reduce gradualmente el límite y lo recupera con histéresis.
- **Grafo por proximidad en GPU**: al activarlo, 768 nodos representativos buscan
  hasta tres vecinos dentro del radio elegido mediante un compute shader. El
  trabajo queda acotado y las líneas se generan sin leer posiciones de vuelta a CPU.

Medición rápida de la app y sus helpers WebKit:

```bash
Scripts/measure-runtime.sh 300 /tmp/particlewall-runtime.csv
```

El plan de medición y migración progresiva a AVFoundation/Metal está documentado en
[`PERFORMANCE_ROADMAP.md`](PERFORMANCE_ROADMAP.md).

Documentación:

- [`PROJECT_PROGRESS.md`](PROJECT_PROGRESS.md): fuente de verdad del estado actual.
- [`PERFORMANCE_ROADMAP.md`](PERFORMANCE_ROADMAP.md): investigación y próximos pasos.
- [`ASCII_VIDEO_NATIVE.md`](ASCII_VIDEO_NATIVE.md): formato y renderer nativo ASCII precomputado.
- [`Animated Wallpaper for Mac.md`](Animated%20Wallpaper%20for%20Mac.md): plan inicial histórico.

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
├── WallpaperRenderer.swift abstracción WebKit + ocho fondos nativos Metal
├── WallpaperControls.swift persistencia y descriptores de parámetros por pantalla
├── WallpaperCustomizationView.swift editor SwiftUI con preview en vivo
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
