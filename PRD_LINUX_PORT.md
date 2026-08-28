# PRD: ParticleWall para Linux y fondos GPU compartidos

## 1. Control del documento

| Campo | Valor |
|---|---|
| Estado | Aprobado para iniciar fase de pruebas y viabilidad |
| Version | 1.0 |
| Fecha | 2026-08-25 |
| Producto actual | ParticleWall para macOS |
| Plataforma objetivo | Omarchy / Hyprland / Wayland |
| Implementacion Linux | Rust |
| Estrategia compartida | Modulos de fondos Nivel A: assets y contratos compartidos, runtimes nativos |
| Paridad objetivo | Visual y funcional con tolerancias entre Metal y Vulkan |

Este documento es la fuente de verdad para desarrollar y validar el port de
ParticleWall a Linux. Los cambios de alcance, contratos, arquitectura o criterios
de aceptacion deben actualizarse aqui antes de implementarse.

## 2. Resumen ejecutivo

ParticleWall es una aplicacion de barra de menu para macOS que muestra fondos
animados detras de los iconos del escritorio. Actualmente soporta:

- Wallpapers HTML, JavaScript y Three.js mediante WKWebView.
- Ocho fondos de particulas nativos mediante Metal.
- Un wallpaper diferente por monitor.
- Importacion de HTML, JS, carpetas y ZIP.
- Controles de apariencia y transformacion por fondo y monitor.
- Limite de FPS, pausa, calidad adaptativa y ahorro de energia.
- Captura y persistencia del ultimo frame.

El objetivo es crear una implementacion para Omarchy/Hyprland usando Rust,
WebKitGTK y wgpu, sin duplicar la definicion de los ocho fondos GPU. Los fondos
se convertiran en modulos compartidos que macOS y Linux puedan cargar desde el
mismo contrato y la misma fuente WGSL.

La aplicacion macOS mantendra Swift, AppKit y Metal. La aplicacion Linux usara
Rust, GTK4, gtk4-layer-shell, WebKitGTK 6 y wgpu. No se introducira una biblioteca
Rust mediante FFI en la aplicacion macOS durante esta version.

## 3. Contexto y problema

### 3.1 Arquitectura actual

La implementacion macOS contiene tres grupos principales:

1. Capa de plataforma: AppKit, NSScreen, NSWindow, SwiftUI, IOKit y SMAppService.
2. Renderer web: WKWebView con scripts inyectados para pausa, FPS, calidad y seguridad.
3. Renderer GPU: un MetalParticleRenderer con los ocho modelos, buffers, shaders,
   simulacion, grafo, controles y snapshots dentro de WallpaperRenderer.swift.

La capa de plataforma no es compilable en Linux. Los recursos web y el contrato
JavaScript si son portables. Los fondos Metal son portables conceptualmente, pero
su implementacion actual mezcla logica del host, shaders y datos especificos de
cada modelo.

### 3.2 Problemas que debe resolver el port

- Linux no dispone de AppKit, WKWebView ni Metal.
- Wayland no permite controlar libremente el nivel de una ventana normal.
- Los ocho fondos GPU estan definidos dentro del codigo Swift y no como modulos.
- MSL y WGSL no ofrecen las mismas primitivas para point sprites.
- La persistencia actual usa UserDefaults y rutas propias de macOS.
- Los tests existentes dependen del target macOS y no se ejecutan en Linux.
- No existe un contrato neutral que ambas implementaciones puedan validar.

## 4. Objetivos

### O-01: Wallpaper funcional en Hyprland

Mostrar un wallpaper animado en la capa BACKGROUND de cada salida Wayland,
sin recibir input y siempre detras de las ventanas normales.

### O-02: Compatibilidad con wallpapers web

Mantener el funcionamiento de wallpapers HTML, JavaScript y Three.js existentes,
incluyendo pausa, FPS, escala de render y controles dinamicos.

### O-03: Fondos GPU compartidos

Definir los ocho fondos incluidos como modulos importables por macOS y Linux,
con una fuente WGSL unica y MSL generado de forma reproducible.

### O-04: Paridad funcional

Conservar asignacion por monitor, controles, persistencia, pausa, deep sleep,
FPS cap, snapshots e importacion.

### O-05: Desarrollo test-first

Crear contratos, fixtures y pruebas de aceptacion antes de implementar cada
segmento de produccion.

### O-06: Validacion cruzada

Validar la misma semantica en Rust/Linux y Swift/macOS mediante fixtures
compartidos, pruebas numericas y comparaciones visuales con tolerancia.

## 5. Fuera de alcance

- Soporte inicial para GNOME, KDE, X11, Sway u otros compositores.
- Reescritura de la interfaz macOS en Rust.
- Core Rust compartido mediante FFI con Swift.
- Tienda o descarga remota de wallpapers.
- Sincronizacion de configuracion entre equipos.
- Comparacion pixel-perfect entre Metal y Vulkan.
- GUI completa de galeria y ajustes en la primera entrega Linux.
- Notarizacion, App Store o publicacion AUR durante los spikes de viabilidad.

## 6. Usuarios y casos de uso

### Usuario principal

Usuario de Omarchy con Hyprland que quiere utilizar los wallpapers incluidos o
importados de ParticleWall con bajo consumo y control por monitor.

### Casos de uso

| ID | Caso |
|---|---|
| UC-01 | Iniciar el daemon y restaurar los fondos asignados |
| UC-02 | Aplicar un fondo a un monitor o a todos los monitores |
| UC-03 | Importar HTML, JS, una carpeta o un ZIP |
| UC-04 | Pausar, reanudar y limitar FPS |
| UC-05 | Cambiar colores, escala, velocidad, brillo y grafo |
| UC-06 | Suspender el renderer al bloquear, dormir o usar Power Save |
| UC-07 | Reanudar sin mostrar un frame negro |
| UC-08 | Usar el mismo modulo GPU en macOS y Linux |
| UC-09 | Diagnosticar renderer, FPS, output y adapter GPU desde CLI |

## 7. Requisitos funcionales

### 7.1 Ventanas y monitores

| ID | Requisito | Prioridad |
|---|---|---|
| FR-OUT-01 | Crear una superficie layer-shell BACKGROUND por output de Hyprland | P0 |
| FR-OUT-02 | Anclar la superficie a los cuatro bordes del output | P0 |
| FR-OUT-03 | Mantener una region de input vacia para que los clicks atraviesen | P0 |
| FR-OUT-04 | Detectar conexion, desconexion y cambios de geometria de outputs | P0 |
| FR-OUT-05 | Persistir asignaciones usando una identidad estable del connector | P0 |
| FR-OUT-06 | Liberar renderer y recursos cuando desaparece un output | P1 |

### 7.2 Renderer web

| ID | Requisito | Prioridad |
|---|---|---|
| FR-WEB-01 | Cargar index.html local con acceso limitado a su carpeta | P0 |
| FR-WEB-02 | Inyectar los mismos scripts compartidos en macOS y Linux | P0 |
| FR-WEB-03 | Bloquear navegacion y subrecursos no permitidos | P0 |
| FR-WEB-04 | Implementar pausa y FPS cap mediante requestAnimationFrame | P0 |
| FR-WEB-05 | Aplicar escala de render y clamp de devicePixelRatio | P1 |
| FR-WEB-06 | Exponer __pwApplySettings y __pwGetControls | P0 |
| FR-WEB-07 | Destruir completamente WebKit durante deep sleep | P0 |
| FR-WEB-08 | Capturar y restaurar el ultimo frame | P1 |

### 7.3 Renderer GPU

| ID | Requisito | Prioridad |
|---|---|---|
| FR-GPU-01 | Renderizar mediante wgpu sobre Vulkan en Linux | P0 |
| FR-GPU-02 | Mantener Metal como backend de macOS | P0 |
| FR-GPU-03 | Usar WGSL como fuente unica de shaders | P0 |
| FR-GPU-04 | Generar MSL reproduciblemente mediante una herramienta versionada | P0 |
| FR-GPU-05 | Usar quads instanciados en lugar de point sprites | P0 |
| FR-GPU-06 | Soportar los ocho fondos GPU incluidos | P0 |
| FR-GPU-07 | Soportar simulacion persistente de Noise Rain a paso fijo | P0 |
| FR-GPU-08 | Cargar datos precomputados de Prime Spiral desde primes.bin | P0 |
| FR-GPU-09 | Soportar grafo por proximidad cuando el modulo lo declara | P1 |
| FR-GPU-10 | Capturar un frame sin bloquear indefinidamente el renderer | P1 |

### 7.4 Biblioteca e importacion

| ID | Requisito | Prioridad |
|---|---|---|
| FR-LIB-01 | Usar XDG_DATA_HOME y XDG_CONFIG_HOME en Linux | P0 |
| FR-LIB-02 | Mantener compatibilidad con manifest v1 existente | P0 |
| FR-LIB-03 | Introducir manifest v2 con referencia neutral a gpu-module | P0 |
| FR-LIB-04 | Importar HTML, JS clasico, ES modules, carpetas y ZIP | P1 |
| FR-LIB-05 | Rechazar path traversal, symlinks de escape y ZIPs inseguros | P0 |
| FR-LIB-06 | Mantener three.js y sus modulos como recursos locales | P1 |
| FR-LIB-07 | Proteger fondos incluidos contra eliminacion | P1 |

### 7.5 Energia y lifecycle

| ID | Requisito | Prioridad |
|---|---|---|
| FR-PWR-01 | Escuchar sleep y wake mediante logind | P0 |
| FR-PWR-02 | Detectar lock y unlock de sesion | P0 |
| FR-PWR-03 | Detectar bateria mediante UPower | P1 |
| FR-PWR-04 | Detectar ventana fullscreen mediante IPC de Hyprland | P1 |
| FR-PWR-05 | Aplicar una politica determinista de pausa y deep sleep | P0 |
| FR-PWR-06 | Ignorar callbacks de snapshots pertenecientes a renderers obsoletos | P0 |
| FR-PWR-07 | Reanudar mostrando el ultimo frame hasta recibir uno nuevo | P1 |

### 7.6 CLI

| ID | Requisito | Prioridad |
|---|---|---|
| FR-CLI-01 | Listar wallpapers y outputs | P0 |
| FR-CLI-02 | Aplicar un wallpaper por ID o nombre | P0 |
| FR-CLI-03 | Pausar y reanudar | P0 |
| FR-CLI-04 | Importar una ruta | P1 |
| FR-CLI-05 | Mostrar diagnostico estructurado en JSON | P0 |
| FR-CLI-06 | Usar exit codes estables para automatizacion | P0 |

## 8. Requisitos no funcionales

| ID | Requisito |
|---|---|
| NFR-01 | No realizar accesos de red durante tests ni al ejecutar wallpapers incluidos |
| NFR-02 | No mantener loops ocupados mientras el wallpaper esta pausado |
| NFR-03 | No filtrar procesos WebKit ni recursos GPU al cambiar fondo o output |
| NFR-04 | Ninguna operacion de snapshot o cierre puede bloquear indefinidamente |
| NFR-05 | Los formatos persistidos deben incluir schemaVersion |
| NFR-06 | Los shaders generados no pueden editarse manualmente |
| NFR-07 | Las diferencias visuales entre backends se validan con tolerancias documentadas |
| NFR-08 | El core Rust debe compilar sin dependencias GTK o Wayland |
| NFR-09 | La primera version Linux solo promete soporte en Omarchy/Hyprland/Wayland |
| NFR-10 | Toda funcionalidad P0 debe tener prueba automatizada o una prueba fisica documentada |

## 9. Arquitectura propuesta

### 9.1 Estructura del repositorio

```text
live-wallpaper/
├── Sources/ParticleWall/                 # Aplicacion macOS
├── Tests/ParticleWallTests/              # Tests Swift y contratos compartidos
├── shared/
│   ├── contracts/
│   │   ├── schemas/
│   │   └── fixtures/
│   ├── scripts/
│   │   ├── raf-patch.js
│   │   ├── dpr-clamp.js
│   │   └── harden.js
│   └── backgrounds/
│       ├── engines/particle-v1/
│       │   ├── particle.wgsl
│       │   └── generated/particle.metal
│       └── modules/<background-id>/
│           ├── background.json
│           ├── preview.png
│           └── resources/
├── linux/
│   ├── Cargo.toml
│   └── crates/
│       ├── particlewall-contracts/       # Modelos, validacion y migraciones
│       ├── particlewall-core/            # Biblioteca, controles y politica de energia
│       ├── particlewall-render/          # wgpu y motor particle-v1
│       └── particlewall-linux/           # GTK4, layer-shell, WebKitGTK y CLI
└── tools/xtask/                          # Validacion y generacion WGSL -> MSL
```

### 9.2 Limites de responsabilidad

| Componente | Responsabilidad |
|---|---|
| contracts | Formatos compartidos, validadores, fixtures y migraciones |
| particle-v1 | ABI de uniforms, formulas, recursos, simulacion, grafo y pases |
| modulo | ID, modelo, defaults, controles, capabilities, recursos y preview |
| host macOS | AppKit, Metal, WKWebView, snapshots y lifecycle Apple |
| host Linux | GTK4, layer-shell, WebKitGTK, wgpu, D-Bus e IPC Hyprland |
| xtask | Validar WGSL, generar MSL y detectar drift de artefactos |

### 9.3 Formato minimo de background.json

```json
{
  "schemaVersion": 1,
  "id": "noise-rain",
  "version": 1,
  "engine": "particle-v1",
  "minimumEngineVersion": 1,
  "model": "noise-rain",
  "capabilities": ["fixed-step-simulation", "graph"],
  "resources": [],
  "controls": [],
  "defaults": {}
}
```

La version 1 no intentara describir un render graph universal. Solo modelara las
capacidades soportadas por particle-v1. Un tipo de fondo completamente diferente
debera declarar otro engine.

### 9.4 Estrategia de shaders

1. particle.wgsl es la fuente editable.
2. xtask valida el WGSL mediante Naga.
3. xtask genera particle.metal usando una version fija de Naga.
4. El MSL generado se versiona junto con su hash de origen.
5. CI regenera en un directorio temporal y compara el resultado.
6. Linux consume WGSL mediante wgpu.
7. macOS compila el MSL generado mediante Metal.

Los point sprites actuales se reemplazaran por quads instanciados. Esta decision
forma parte del contrato particle-v1 y evita implementar dos representaciones
visuales diferentes.

## 10. Compatibilidad y migracion

### 10.1 Persistencia existente

Existe una necesidad concreta de compatibilidad: usuarios actuales pueden tener
manifests y asignaciones persistidas. Por tanto:

- manifest v1 seguira siendo legible.
- Los valores metal-* se mapearan a IDs neutrales de modulos.
- Las carpetas importadas por el usuario no se reescribiran destructivamente.
- La migracion sera idempotente.
- Los datos originales se conservaran si la migracion falla.

### 10.2 Mapeo inicial

| Renderer actual | Modulo nuevo |
|---|---|
| metal-particles | parametric-waves |
| metal-twin-vortex | twin-vortex |
| metal-orbital-bloom | orbital-bloom |
| metal-hexagonal-rosette | hexagonal-rosette |
| metal-noise-rain | noise-rain |
| metal-prime-spiral | prime-spiral |
| metal-torus-orbit | torus-orbit |
| metal-chromatic-rings | chromatic-rings |

## 11. Estrategia de pruebas

Los tests son una fase bloqueante, no una actividad posterior. Para cada segmento
se seguira el ciclo: test rojo, implementacion minima, test verde y refactor.

### T0: Baseline macOS

- Ejecutar los 30 XCTest existentes en el Mac fisico.
- Corregir la documentacion que todavia indica 20 tests.
- Crear smoke tests estructurados para los ocho fondos.
- Capturar referencias visuales deterministas.
- Registrar hardware, macOS, resolucion y escala usados.

### T1: Contratos compartidos

Fixtures consumidos por XCTest y cargo test:

- manifest v1/v2 y migraciones;
- modulos validos e invalidos;
- controles y defaults;
- persistencia por monitor;
- las 16 combinaciones de politica energetica;
- navegacion local y escapes de ruta;
- importacion y ZIPs hostiles.

### T2: JavaScript compartido

Con reloj y scheduler falsos:

- FPS 0, 15, 30 y 60;
- pausa y reanudacion;
- varios callbacks en el mismo frame;
- ausencia de busy-loop;
- degradacion y recuperacion adaptativa;
- DPR 1, 1.5 y 2;
- lectura y aplicacion de controles.

### T3: Shaders y recursos

- Parseo y validacion WGSL.
- Generacion MSL reproducible.
- Deteccion de drift del MSL generado.
- Layout y alineacion de uniforms.
- Recursos declarados presentes.
- Conteo y hash de primes.bin.
- IDs y versiones unicos.

### T4: Paridad numerica GPU

Vectores deterministas:

```text
(model, vertex_id, time, aspect, controls)
  -> position, size, color, alpha
```

Los resultados CPU, wgpu y Metal se compararan con tolerancia flotante. Se
incluiran los limites de cada modelo, varios tiempos, Noise Rain, Prime Spiral,
Chromatic Rings y una muestra pequena del grafo.

### T5: Integracion Linux

- Superficie background por output.
- Input passthrough.
- Resize y cambio de escala.
- WebKitGTK local-only.
- wgpu sobre la superficie layer-shell.
- Snapshot y deep sleep.
- Eventos logind, UPower e IPC Hyprland.
- Conexion y desconexion de monitor.

### T6: Integracion macOS

- Lectura del mismo modulo.
- Compilacion del MSL generado.
- Los ocho fondos.
- Fallback WebKit.
- Nivel desktop y click-through.
- Lock, sleep, wake y monitores.
- Migracion de manifests existentes.

### T7: Comparacion visual funcional

No se compararan bytes exactos entre GPUs. Se validaran:

- dimensiones;
- imagen no vacia ni completamente negra;
- porcentaje de pixeles activos;
- colores dominantes e histograma;
- bounding box y distribucion espacial;
- hash perceptual con tolerancia;
- ausencia de NaN y valores no finitos.

## 12. Matriz de validacion

| Requisito | Test principal | Plataforma | Gate |
|---|---|---|---|
| FR-OUT-01..06 | layer_shell_outputs | Omarchy | M0/M3 |
| FR-WEB-01..08 | web_renderer_contract | Ambas | M0/M3 |
| FR-GPU-01..05 | shader_and_surface_smoke | Ambas | M0/M1 |
| FR-GPU-06 | bundled_modules_smoke | Ambas | M2 |
| FR-GPU-07 | noise_rain_fixed_step | Ambas | M2 |
| FR-GPU-08 | prime_resource_contract | Ambas | M2 |
| FR-GPU-09 | graph_compute_contract | Ambas | M2 |
| FR-LIB-01..07 | library_import_contract | Ambas | M3 |
| FR-PWR-01..07 | playback_lifecycle | Ambas | M4 |
| FR-CLI-01..06 | cli_black_box | Linux | M3/M5 |
| NFR-01 | offline_test_guard | Ambas | Todos |
| NFR-02..04 | lifecycle_resource_tests | Ambas | M3/M4 |
| NFR-05..07 | schema_shader_visual_tests | Ambas | M1/M2 |
| NFR-08 | particlewall-core cargo check | Linux | Todos |
| NFR-09 | Hyprland acceptance | Omarchy | Release |
| NFR-10 | requirements traceability check | Ambas | Release |

## 13. Fases de implementacion

### Fase T0: Preparacion de tests

Entregables:

- Baseline macOS.
- Carpeta shared/contracts.
- Fixtures iniciales.
- Test targets Swift, Rust y JavaScript.
- Harness de comparacion visual.

Criterio de salida:

- Los tests actuales macOS pasan.
- Los nuevos tests contractuales existen antes de su implementacion.
- Los fallos esperados estan identificados y no se ocultan con skip.

### Fase M0: Spikes de viabilidad

M0A valida GTK4 + gtk4-layer-shell + WebKitGTK 6.

M0B valida wgpu presentando en una superficie layer-shell.

Criterio de salida:

- Ambos spikes muestran contenido detras de ventanas y sin capturar input.
- Web y GPU producen frames y se destruyen sin filtrar recursos.

### Fase M1: Motor y primer modulo compartido

Implementar particle-v1 y Parametric Waves en ambas plataformas.

Criterio de salida:

- El mismo modulo pasa contratos, paridad numerica y validacion visual en Metal y Vulkan.

### Fase M2: Ocho fondos GPU

Orden recomendado:

1. Twin Vortex.
2. Orbital Bloom.
3. Hexagonal Rosette.
4. Torus Orbit.
5. Chromatic Rings.
6. Prime Spiral.
7. Noise Rain.
8. Grafo compartido.

Criterio de salida:

- Los ocho fondos pasan smoke, paridad numerica y validacion visual.

### Fase M3: Daemon funcional

- Multi-monitor.
- Persistencia.
- Renderer web.
- FPS y pausa.
- Biblioteca y CLI.

Criterio de salida:

- Reiniciar el daemon restaura asignaciones y controles en todos los outputs.

### Fase M4: Energia y deep sleep

- logind.
- Lock/unlock.
- UPower.
- Fullscreen Hyprland.
- Snapshot y restauracion.

Criterio de salida:

- El consumo del renderer cae a cero durante deep sleep y la reanudacion no muestra negro.

### Fase M5: Empaquetado

- Servicio systemd --user.
- Instalacion en Omarchy.
- Diagnosticos.
- Documentacion de operacion.
- PKGBUILD, si se aprueba como alcance de release.

## 14. Gates obligatorios

- G-01: Ningun cambio del renderer macOS antes de completar el baseline.
- G-02: No portar los ocho fondos antes de validar uno en ambas plataformas.
- G-03: No aceptar fixtures contractuales ignorados.
- G-04: No acceder a red durante tests.
- G-05: No aceptar ZIP traversal, symlink escape ni rutas por prefijo inseguro.
- G-06: Todo WGSL debe pasar Naga y generar MSL reproducible.
- G-07: Todos los fondos deben pasar paridad numerica y visual funcional.
- G-08: Los callbacks tardios no pueden revivir un renderer destruido.
- G-09: Toda funcionalidad P0 debe estar vinculada a un test en la matriz.
- G-10: La release Linux debe validarse en el equipo Omarchy objetivo.
- G-11: La release macOS debe validarse en el Mac fisico disponible.

## 15. Dependencias previstas

### Linux / sistema

- Rust stable.
- GTK4.
- gtk4-layer-shell.
- webkitgtk-6.0.
- Vulkan loader y driver compatible.
- pkg-config.
- systemd user session.
- Hyprland IPC.

### Rust

La seleccion exacta se fijara durante T0/M0. Se espera usar:

- gtk4 y gtk4-layer-shell;
- webkit6;
- wgpu;
- serde y serde_json;
- zbus;
- tracing;
- naga mediante xtask;
- crates de imagen y comparacion visual solo donde sean necesarias.

Todas las versiones criticas, especialmente wgpu y Naga, deben quedar fijadas.

## 16. Riesgos

| Riesgo | Impacto | Mitigacion |
|---|---|---|
| wgpu no presenta correctamente dentro de GTK layer-shell | Alto | Spike M0B antes del daemon |
| Diferencias WGSL/MSL o Naga | Alto | Paridad numerica, artefactos generados y version fija |
| Cambio visual por reemplazo de point sprites | Alto | Quads en ambos backends y baseline visual |
| WebKitGTK presenta problemas DMABUF | Medio | Probar fallback WEBKIT_DISABLE_DMABUF_RENDERER=1 |
| Identidad de monitor inestable | Medio | Contrato por connector y tests de reconexion |
| Noise Rain diverge por timestep | Alto | Reloj inyectable y fixed-step probado |
| Migracion rompe bibliotecas existentes | Alto | Fixtures v1, migracion idempotente y no destructiva |
| Scope crece hacia otros escritorios Linux | Medio | Limite explicito a Omarchy/Hyprland |

## 17. Definicion de terminado

La version Linux se considera terminada cuando:

- Los ocho wallpapers GPU funcionan en macOS y Omarchy desde modulos compartidos.
- Los wallpapers web existentes funcionan offline en ambas plataformas.
- Las asignaciones por output se restauran despues de reiniciar.
- Pausa, FPS cap, lock, sleep, bateria y fullscreen cumplen su politica.
- Deep sleep libera WebKit y recursos GPU.
- Los contratos compartidos pasan en XCTest y cargo test.
- WGSL y MSL generado no presentan drift.
- Las pruebas numericas y visuales pasan dentro de sus tolerancias.
- No existen fallos P0 abiertos.
- La matriz de trazabilidad no contiene requisitos P0 sin test.
- Se completa la validacion fisica en Omarchy y macOS.

## 18. Seguimiento de progreso

Esta tabla debe actualizarse al completar cada fase.

| Fase | Estado | Evidencia | Fecha |
|---|---|---|---|
| T0 - Tests y baseline | Completado (baseline Mac verificado por el usuario; contratos y tests Linux en verde) | swift test + build-app.sh OK en Mac; 7 tests cargo + 5 tests node | 2026-08-25 |
| M0A - WebKit layer-shell | Completado | Daemon WebKitGTK corriendo bajo systemd --user en Omarchy/Hyprland; wallpaper DefaultWallpaper visible; CLI --pause/--resume/--fps/--status operativa via socket | 2026-08-25 |
| M0B - wgpu layer-shell | Completado | GPU renderer wgpu/Vulkan en vivo en Omarchy: connection Wayland dedicada + superficies zwlr-layer-shell (GTK 4.18 no puede compartir wl_surface con la WSI de NVIDIA por wp_fifo_v1); presenters persistentes (parked) en switches web-gpu | 2026-08-28 |
| M1 - Primer modulo compartido | Completado | particle.wgsl (particle-v1) + modulo parametric-waves + crate particlewall-render (uniforms 128B, referencia CPU, presenter wgpu, tests de paridad CPU-GPU en verde); wallpaper GPU aplicado y verificado visualmente en vivo | 2026-08-28 |
| M2 - Ocho fondos GPU | Pendiente | - | - |
| M3 - Daemon funcional | En curso adelantado (multi-output + CLI listos; faltan persistencia, importador y biblioteca) | linux/crates/particlewall-linux | 2026-08-25 |
| M4 - Energia y deep sleep | Pendiente | - | - |
| M5 - Empaquetado | Parcial (servicio systemd + install.sh listos) | linux/install.sh, linux/particlewall.service | 2026-08-25 |

## 19. Primer siguiente paso autorizado

El siguiente paso es exclusivamente la Fase T0:

1. Ejecutar y registrar el baseline actual en el Mac fisico.
2. Crear schemas y fixtures compartidos.
3. Crear los tests contractuales Swift y Rust antes de implementar sus modelos.
4. Extraer y probar los scripts JavaScript compartidos.
5. Preparar el harness visual y los criterios numericos.

No debe iniciarse el renderer Linux ni refactorizarse Metal hasta cumplir G-01.
