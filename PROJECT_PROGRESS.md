# ParticleWall: estado y registro de progreso

Actualizado: 2026-08-29

Este documento es la fuente de verdad del estado actual del proyecto. El
[`README.md`](README.md) contiene la guía rápida de uso,
[`PERFORMANCE_ROADMAP.md`](PERFORMANCE_ROADMAP.md) conserva la investigación y
el trabajo técnico pendiente, [`PRD_LINUX_PORT.md`](PRD_LINUX_PORT.md) es el PRD
del port a Linux con su seguimiento por fases (sección 18), y
[`Animated Wallpaper for Mac.md`](Animated%20Wallpaper%20for%20Mac.md)
es el plan histórico con el que comenzó la aplicación.

## Port a Linux (Omarchy/Hyprland) — estado actual

Port en Rust de la aplicación macOS a Linux, compartiendo todos los fondos GPU
como módulos WGSL multiplataforma (`shared/backgrounds/engines/particle-v1/`).
Rama de trabajo: `feature/linux-port`.

### Arquitectura del port

- **Daemon Rust** (`linux/crates/particlewall-linux`): servicio systemd --user
  con salida por monitor (GTK4 + gtk4-layer-shell), socket Unix de control,
  tray de barra, CLI, monitor de energía y launcher de escritorio.
- **Motor GPU compartido**: `particle.wgsl` con los nueve fondos + kernels de
  flow/grafo; referencia CPU y tests de paridad en `particlewall-render`;
  `tools/xtask` valida el WGSL y genera el MSL que consume macOS.
- **Renderer web**: WebKitGTK con los fallbacks HTML incluidos y contrato de
  aparición compartido con macOS.

### Funcionalidades terminadas en el port

- [x] M0A: daemon WebKitGTK bajo systemd --user, CLI base (--pause/--resume/--fps/--status).
- [x] M0B: renderer wgpu/Vulkan en superficies zwlr-layer-shell (conexión Wayland
      dedicada; presenters GPU permanentes que se reconfiguran in-place).
- [x] M1: primer módulo compartido (Ondas Paramétricas) con paridad CPU-GPU.
- [x] M2: ocho fondos GPU en WGSL + flowUpdate + grafo por proximidad; contratos
      FR-GPU-06..09 en verde; ciclo completo verificado en vivo sin caídas.
- [x] Noveno fondo compartido "Toro de Esferas" (`sphere-torus`) en ambas plataformas.
- [x] Décimo fondo compartido "Medusa de Puntos" (`jellyfish-points`, dweet de
      10k puntos con XOR JS `y^8` y envoltura de tiempo a 8·PI) en ambas plataformas.
- [x] Undécimo fondo compartido "Nebulosa" (`nebula`, dweet de 921.600 puntos:
      9 capas × 512 ángulos × 200 radios con el value-noise del motor como
      sustituto del Perlin de p5) con variaciones propias ajustadas por
      iteración visual con el usuario: las 9 nubes ocupan objetivos estables
      en una cuadrícula 3×3 con jitter de ±18 px: las columnas compensan el
      aspecto panorámico para caer en los tercios físicos de la pantalla y
      las filas usan 45/165/285 px para dejar visibles sus estelas. Cada nube
      tiene una profundidad determinista (perspectiva 0.85..1.35) y respira
      ±20 px sobre Z, con fases propias y un ciclo de ~52 s, mediante la
      perspectiva p5 WEBGL (eyeZ = 300/tan(PI/6)); no hay órbita que pueda
      reagrupar temporalmente la formación. La dispersión final conserva por
      elección del usuario el sesgo original `flowNoise(...) * i * 400` y el
      tamaño completo de cada nube. Incluyó un fix de contrato: la referencia CPU ahora
      aplica `fract(x) = x - floor(x)` en `flow_hash`/`flow_noise`, igual
      que el WGSL (Rust `f32::fract` preserva el signo y divergía con
      coordenadas negativas; los fixtures existentes no cambiaron).
- [x] Duodécimo fondo compartido "Tesseract Cuántico" (id técnico `torus-knot`,
      20000 puntos portados del THREE.js del usuario: 350 bloques wireframe
      punteados sobre un nudo (2,5) en 4 carriles escalonados + 5% de polvo
      estelar con titileo). Marco local analítico con retorcido del haz,
      rotación global Rx(0.11t)·Ry(0.17t) y presentación adaptada al lenguaje
      visual de ParticleWall: canvas 400, mundo ×2.2, cámara p5 WEBGL
      (eyeZ=200/tan(PI/6)), color monocromo del usuario y puntos uniformes
      (`pointScale=1`). Aristas, caras punteadas, esquinas, pulsos y polvo
      conservan su jerarquía mediante alpha (1/0.4, +0.4, impulso y
      0.15+twinkle·0.85), sin niebla, bloom, cámara THREE ni lerp por partícula.
      Contrato FR-GPU-07 estructural para el modelo 11: los hashes
      fract(sin(·)·43758) del sketch son caóticos por muestra en f32, así
      que la paridad GPU verifica invarianzas (tamaño/color, alpha y finitud)
      y el fixture fija la referencia CPU. Registro en ambas
      plataformas (contratos, daemon Linux, app macOS) con fallback HTML
      propio; fixture + MSL regenerados.
- [x] M3 (parcial): multi-output, persistencia en `~/.config/particlewall/config.json`
      (wallpaper, colores, perfiles), controles de tamaño de partícula (0.25–8) e
      intensidad/brillo (0.25–10) con ventana GTK de sliders, scroll en el tray,
      presets y CLI `--set-color`; curva de brillo asintótica `1 - exp(-0.55·b)`
      compartida WGSL/CPU (el brillo ya no se satura por encima de ~2.6).
- [x] M4: energía — lock/unlock por logind, sleep/wake, pausa por fullscreen de
      Hyprland y UPower; política determinista (`PlaybackFlags.system_paused`).
- [x] M5 (parcial): servicio systemd, `install.sh` y entrada de escritorio con
      ícono (modo `--app` abre ajustes o arranca el servicio).
- [x] Fix de apilamiento en el arranque: el shell de Omarchy (quickshell) mapea
      su fondo estático en la capa Background unos segundos después del
      daemon, y las superficies de una misma capa se apilan por orden de
      mapeo, así que el fondo animado quedaba tapado hasta reiniciar el
      servicio (el proceso vivo y presentando). Ambas superficies propias
      (GPU y GTK/web) suben ahora a la capa Bottom: siempre por encima de
      Background y por debajo de las ventanas y la barra, sin carrera de
      arranque. Descartado el re-map de superficie: cualquier unmap/re-role
      de la wl_surface que alimenta un swapchain vivo segfaulta el WSI
      Wayland de NVIDIA (verificado dos veces con coredump).

### Verificación en Linux

- `cargo test` verde en los tres crates (los de render SIEMPRE con
  `--test-threads=1`: cuatro contextos GPU paralelos cuelgan el driver NVIDIA).
- Fixtures numéricos regenerados para los doce fondos; paridad CPU-GPU en verde
  (Medusa y Nebulosa con tolerancias dedicadas: reducción de argumentos en
  `cos(i - t/4)` hasta i=9999 y contracción FMA del value-noise, FR-GPU-07).
- `tools/xtask -- shadergen --check` sin drift entre WGSL y MSL generado.
- Verificaciones en vivo con capturas: los once fondos (Nebulosa con su
  formación balanceada 3×3 y profundidad animada), sliders de ajustes,
  launcher (`gtk-launch particlewall`) y pausa/reanudación por energía.

### Optimización GPU/CPU en Linux (2026-10-08)

Medido en RTX 3060 Ti, salida DP-3 2560×1440 con escala 1,25, 30 FPS.

- Quads indexados (`vsIndexed`): 4 vértices únicos por partícula con el patrón
  de índices 0 1 2 2 1 3 en lotes de 16.384; la caché post-transformación
  comparte la diagonal y `particleSample` corre 4 veces en vez de 6. Tiempo GPU
  1,07–1,69× menor en los doce fondos (Nebulosa 0,79 → 0,50 ms por frame) e
  imagen idéntica a `vsMain` (diferencia máxima ≤ 1/255, prueba
  `indexed_quads_match_list6_pixels`). Se descartaron quads instanciados (más
  lentos en Espiral Prima) y muestras precalculadas por compute (pierden por
  tráfico de memoria). `vsMain` sigue intacto para macOS.
- Superficies a resolución nativa: con `wp_fractional_scale_v1` +
  `wp_viewporter` una salida 1,25× renderiza 2560×1440 en vez de 4096×2304
  (escala entera 2 reducida por el compositor). `point_scale` conserva el
  tamaño en pantalla de las partículas. Sin esos protocolos se mantiene la
  escala entera.
- Un solo device wgpu, shader y pipelines compartidos por todas las salidas;
  `MemoryHints::MemoryUsage`; latencia máxima de frame 1 (2 imágenes de
  swapchain en vez de 3 en NVIDIA/Wayland).
- VRAM del presenter en vivo: 123 MiB → 47 MiB por salida. Con tres salidas el
  device compartido ahorra además ~18 MiB.
- Corregidos los conflictos de bindings que invalidaban cada frame de Lluvia de
  Ruido y del grafo en wgpu (buffer enlazado de solo lectura y de
  lectura-escritura en el mismo dispatch). Prueba
  `every_model_encodes_without_validation_errors`.
- Pausa: el bucle GPU sondea a 4 Hz en vez de despertar al límite de FPS.
- El WGSL se embebe en el binario (`include_str!`); ya no depende del checkout.
- WebKitGTK opcional (feature `webkit`): `--no-default-features --features
  gpu,power` compila el daemon con ajustes, bandeja y CLI, y lista solo los doce
  fondos GPU. `install.sh` lo elige solo si falta `webkitgtk-6.0` o con
  `--gpu-only`.
- Las ventanas GTK de WebKit solo se mapean con un fondo web: al realizarlas GTK
  creaba un segundo device Vulkan (+70 MiB de VRAM, ~250 cambios de contexto/s).
  Daemon GPU medido en vivo: 47 MiB de VRAM, 0,9 % de un núcleo, 0,1 % en pausa.
  El hilo sin nombre que despierta a 100 Hz pertenece al driver NVIDIA (también
  aparece en `examples/live`).
- CPU en vivo ~0,9 % de un núcleo a 30 FPS, dominada por el present del driver;
  sin cambio medible. Herramientas: `cargo run --release -p particlewall-render
  --example bench` (offscreen, timestamps) y `--example live` (superficie real).

### Pendiente del port

- [ ] Verificación en el Mac físico: `swift test` + `build-app.sh` con el MSL
      regenerado y los once fondos compartidos.
- [ ] M3, remanente: importador y biblioteca de wallpapers de usuario en Linux.
- [ ] M5, remanente: documentación de operación y PKGBUILD (si se aprueba).
- [ ] (Opcional) sincronizar en vivo las etiquetas de la ventana de ajustes tras
      un `--set-color` externo.
- [ ] macOS: adoptar `vsIndexed` con `drawIndexedPrimitives` (el MSL ya lo
      incluye) y medirlo con Instruments.
- [ ] Linux: validar en vivo multi-monitor, cambio de escala y la build con
      WebKit (la build solo GPU ya se validó en vivo en una salida 1,25×).
- [ ] Linux: liberar el contexto GPU de GTK al cerrar la ventana de ajustes.

## Estado general

ParticleWall es una aplicación de barra de menú para macOS que mantiene una
ventana de escritorio por pantalla. La arquitectura actual es híbrida:

- **Metal nativo** para los ocho fondos incluidos.
- **WebKit** como ruta de compatibilidad para HTML, JavaScript, Three.js,
  módulos ES, carpetas y ZIP importados.
- **SwiftUI/AppKit** para galería, ajustes, personalización, ventanas de
  escritorio y gestión de energía.

La aplicación de producción se compila, firma ad-hoc e instala en:

```text
~/Applications/ParticleWall.app
```

## Funcionalidades terminadas

### Aplicación y biblioteca

- [x] Aplicación de barra de menú sin ícono en el Dock.
- [x] Ventana de escritorio que deja atravesar los clics.
- [x] Una ventana y una asignación persistente por pantalla.
- [x] Restauración de asignaciones al iniciar y reacción a cambios de monitores.
- [x] Galería SwiftUI con búsqueda, selección de pantalla, importación y drag & drop.
- [x] Menú rápido desde el ícono de la barra.
- [x] Botón `•••` visible al hacer hover en la esquina inferior derecha de cada tarjeta.
- [x] Aplicar, personalizar, cambiar FPS, renombrar, regenerar miniatura y mostrar en Finder.
- [x] Fondos incluidos protegidos: no pueden eliminarse.
- [x] Reparación automática de un fondo incluido ausente sin afectar imports.
- [x] Sincronización del thumbnail con el wallpaper estático del sistema para
      mantener coherentes la barra de menú y las transiciones de Spaces.

### Importación y compatibilidad

- [x] Importación de HTML completo.
- [x] Importación de snippets JavaScript/Three.js mediante template.
- [x] Importación de módulos ES con import map local de Three.js 0.160.
- [x] Importación de carpetas y archivos ZIP.
- [x] Copias locales de Three.js y módulos de postprocesado.
- [x] Reescritura de referencias CDN conocidas.
- [x] Bloqueo de navegación WebKit fuera de los archivos locales permitidos.
- [x] Generación serial de thumbnails con diagnóstico de errores JavaScript.
- [x] Actualización idempotente de módulos ya importados.
- [x] Copia derivada `pw-user-module.js` sin modificar el archivo original.
- [x] Órbita de cámara lenta para módulos ES compatibles que no animan su cámara.

### Personalización y persistencia

- [x] Editor SwiftUI con vista previa en vivo.
- [x] Posición X/Y/Z, rotación X/Y/Z, escala y velocidad.
- [x] Detección de cámara, bloom y propiedades comunes en módulos ES.
- [x] Contrato para parámetros propios mediante `particleWallControls`,
      `getParticleWallControls()` y `setParticleWallParameter`.
- [x] Extracción de controles generados con `PARAMS` y `addControl(...)`.
- [x] Tipos numérico, color y booleano.
- [x] Configuración independiente por wallpaper y UUID de pantalla.
- [x] Configuración comodín para “Todas las pantallas”.
- [x] Guardado explícito con `Cmd-S`.
- [x] Restauración real de los valores predeterminados.
- [x] Selector de color nativo para fondo y partículas.
- [x] Biblioteca personal global de perfiles de color.
- [x] Guardado conjunto del color de fondo y partículas con nombre opcional.
- [x] Aplicación y eliminación independiente de perfiles.
- [x] Adaptación al tamaño de pantalla.
- [x] Límites horizontal y vertical configurables.
- [x] Tamaño de partículas.
- [x] Intensidad de puntos configurable hasta `10`.

### Grafo por proximidad

- [x] Interruptor “Conectar puntos cercanos”.
- [x] Distancia de conexión configurable entre `0.02` y `0.25`.
- [x] Entre una y tres conexiones por nodo.
- [x] Intensidad de líneas configurable hasta `10`.
- [x] Búsqueda de vecinos y generación de aristas en compute shaders Metal.
- [x] Muestra acotada de 768 nodos representativos.
- [x] Máximo de 2.304 aristas y 4.608 vértices de línea por frame.
- [x] Dos pases Metal separados para evitar una caída del driver al alternar
      líneas y puntos dentro del mismo render encoder.
- [x] Costo de búsqueda eliminado cuando el modo está desactivado.

El grafo calcula proximidad real entre los 768 nodos muestreados. No compara
todos los puntos originales entre sí porque eso haría crecer el trabajo de
forma cuadrática en escenas de hasta 119.958 vértices.

### Energía y rendimiento

- [x] Renderer lazy: no se crea hasta tener contenido.
- [x] Eliminación de la doble restauración del arranque.
- [x] Power Save y pausa por batería.
- [x] Deep sleep por bloqueo, sleep de pantalla y sesión inactiva.
- [x] Destrucción de WebKit durante deep sleep y reconstrucción al reanudar.
- [x] Imagen congelada o thumbnail durante reposo.
- [x] Pausa automática cuando el escritorio está ocluido.
- [x] Teardown de previews al ocultar o cerrar la galería.
- [x] FPS global y por wallpaper.
- [x] Throttle WebKit con `setTimeout` para evitar despertar a frecuencia ProMotion.
- [x] Escala de render configurable.
- [x] Calidad adaptativa con histéresis.
- [x] Script reproducible de CPU/RSS: `Scripts/measure-runtime.sh`.
- [x] Smoke test de todos los renderers Metal.
- [x] Diagnóstico CLI de FPS, escala, controles y errores.

## Fondos Metal incluidos

Todos son protegidos, conservan configuración independiente y tienen un
fallback Canvas para miniaturas o equipos sin Metal.

| Fondo | Renderer | Carga nativa |
|---|---|---:|
| Ondas Paramétricas | `metal-particles` | 10.000 puntos |
| Vórtice Gemelo | `metal-twin-vortex` | 30.000 puntos |
| Flor Orbital | `metal-orbital-bloom` | 30.000 puntos |
| Roseta Hexagonal | `metal-hexagonal-rosette` | 119.958 vértices |
| Lluvia de Ruido | `metal-noise-rain` | 6.480 partículas × 12 muestras = 77.760 vértices |
| Espiral Prima | `metal-prime-spiral` | 78.498 números primos |
| Órbita Toroidal | `metal-torus-orbit` | 37.376 puntos |
| Anillos Cromáticos | `metal-chromatic-rings` | 6.225 puntos × 8 muestras = 49.800 vértices |

Decisiones de optimización específicas:

- Ondas, vórtices y flores se calculan proceduralmente en el vertex shader.
- La roseta genera seis simetrías sin copiar el framebuffer.
- La lluvia conserva estado y estelas en buffers GPU.
- La espiral guarda únicamente los 78.498 primos, no un millón de enteros.
- La órbita usa una nube acotada en lugar de teselación extrema.
- Los anillos sustituyen el desenfoque de pantalla completa por ocho muestras
  temporales.

## Arquitectura implementada

```text
WallpaperWindowController
└── WallpaperRenderer
    ├── MetalParticleRenderer
    │   ├── vertex/fragment shaders de partículas
    │   ├── compute shader de Lluvia de Ruido
    │   └── compute shaders del grafo por proximidad
    └── WebWallpaperRenderer
        └── WKWebView + puente de controles + throttle

WallpaperManager
├── asignación por pantalla
├── persistencia y restauración
└── sincronización con wallpaper del sistema

LibraryManager
├── wallpapers importados
├── ocho recursos incluidos protegidos
└── instalación y reparación idempotente

WallpaperCustomizationView
├── preview en vivo
├── controles por categoría
├── persistencia por wallpaper/pantalla
└── biblioteca global de perfiles de color
```

## Estado de verificación

Última verificación funcional completa:

- `swift test -c release`: **20/20 pruebas correctas**.
- `git diff --check`: correcto.
- Smoke test con grafo activo e intensidades de puntos/líneas en `10`.
- Los ocho renderers completaron 62 frames en dos segundos.
- Rendimiento observado: aproximadamente `29.1–29.4 FPS` con límite de 30.
- Compilación release, firma ad-hoc e instalación correctas.
- Aplicación instalada abierta sin errores registrados al arrancar.

Comandos:

```bash
swift test -c release
./build-app.sh
~/Applications/ParticleWall.app/Contents/MacOS/ParticleWall --renderer-smoke-test
Scripts/measure-runtime.sh 300 /tmp/particlewall-runtime.csv
```

## Problemas importantes ya resueltos

- Fondo estático después de pausar/reanudar: el reloj Metal ahora acumula solo
  tiempo efectivamente dibujado.
- Modelos ES reportados falsamente como incompatibles: consulta con reintentos y
  fallback de transformación.
- Fondos incluidos eliminados accidentalmente: bloqueo de eliminación y reparación.
- Menú de tarjeta difícil de descubrir: botón `•••` por hover.
- Botón desalineado: colocado dentro de la geometría real de la tarjeta.
- Partículas con poco contraste: controles de color, tamaño e intensidad.
- Configuración compartida accidentalmente: persistencia separada por wallpaper
  y pantalla.
- Caída del driver Metal al mezclar líneas y puntos: render en dos pases.

## Trabajo pendiente

Prioridad alta:

- [ ] Medición de cinco minutos con Instruments: CPU, RSS, wakeups y tiempo GPU.
- [ ] Compartir device, pipelines y buffers inmutables Metal entre pantallas.
- [ ] Revisar el template de snippets para evitar un sistema de partículas
      precreado cuando el snippet monta el suyo.
- [ ] Pruebas UI automatizadas para galería, personalización y perfiles.
- [ ] Manifest v2 formal y versionado.

Compatibilidad y formatos:

- [ ] `VideoRenderer` nativo con AVFoundation.
- [ ] Importación de `.mp4` y `.mov`.
- [x] Formato offline `.asciivideo` y conversor de celdas ASCII sin análisis en runtime.
- [x] Renderer Metal inicial para wallpapers `.asciivideo` importados.
- [ ] Renderer wgpu compartido para `ascii-video-v1`.
- [ ] API opt-in de controles para HTML arbitrario.
- [ ] Controles de lista, vectores agrupados y presets de parámetros no cromáticos.
- [ ] Conversión declarativa segura de fórmulas conocidas a Metal.
- [ ] Indicador visible del motor usado por cada tarjeta.

Mejoras opcionales:

- [ ] Renombrar perfiles de color existentes.
- [ ] Exportar e importar perfiles/configuraciones.
- [ ] Color independiente para las líneas del grafo.
- [ ] Cantidad de nodos del grafo ajustable por nivel de calidad.
- [ ] Baseline comparativo documentado WebKit vs Metal en una y dos pantallas.

## Regla para continuar el proyecto

Después de cada cambio importante:

1. Actualizar este documento y la sección relevante del roadmap.
2. Añadir o modificar una prueba cuando exista lógica persistente o límites.
3. Ejecutar `swift test -c release` y `git diff --check`.
4. Ejecutar `--renderer-smoke-test` si cambia Metal.
5. Registrar aquí el resultado y cualquier decisión técnica nueva.
