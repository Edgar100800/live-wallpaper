# ParticleWall: investigación, optimización y migración nativa

Actualizado: 2026-07-27

El inventario vigente de funciones terminadas, pruebas y pendientes está en
[`PROJECT_PROGRESS.md`](PROJECT_PROGRESS.md). Este archivo conserva el análisis
de rendimiento y la planificación técnica.

## Objetivo

Reducir CPU, GPU, memoria y consumo energético sin perder la compatibilidad con
los wallpapers HTML/JavaScript que ya puede importar ParticleWall.

La aplicación ya usa Swift, SwiftUI y AppKit. El costo principal no está en la
interfaz, sino en mantener un `WKWebView` con JavaScript, Three.js y WebGL por
pantalla. Por ello, la dirección técnica es una arquitectura híbrida:

```text
WallpaperWindowController
└── WallpaperRenderer
    ├── MetalParticleRenderer  (partículas nativas)
    ├── VideoRenderer          (AVFoundation)
    └── WebRenderer            (compatibilidad HTML/Three.js)
```

WebKit seguirá siendo necesario para contenido web arbitrario, pero dejará de
ser el renderer obligatorio para todos los formatos.

## Hallazgos de la auditoría inicial

Los puntos siguientes describen el baseline encontrado al comenzar la
investigación. La mayoría ya está corregida y su estado se registra en las
fases y en `PROJECT_PROGRESS.md`.

1. El arranque restaura cada wallpaper durante `WallpaperManager.start()` y
   vuelve a restaurarlo desde `AppDelegate`, duplicando carga, parseo y creación
   del contexto WebGL.
2. La pausa normal conserva WebKit y sus recursos. Cada loop interceptado sigue
   despertando periódicamente para comprobar el estado.
3. `enterDeepSleep()` sustituye el webview destruido por otro `WKWebView` vacío,
   por lo que el controlador nunca queda realmente libre de WebKit.
4. El wallpaper incluido actualiza 6,000 partículas y vuelve a subir su buffer
   desde JavaScript en cada frame.
5. El template de snippets crea otras 5,000 partículas antes de insertar el
   código del usuario, incluso si el snippet monta su propio renderer.
6. Un preview de galería puede continuar vivo si la ventana se oculta sin que
   SwiftUI destruya su jerarquía.
7. La generación de thumbnails crea webviews adicionales, aunque su cola ya es
   serial y limita correctamente la concurrencia.
8. No existe todavía un target de pruebas ni una referencia automatizada de
   consumo.

## Principios de implementación

- No crear un motor de render hasta que exista contenido que mostrar.
- Al bloquear, dormir o abandonar la sesión, liberar el renderer, no solo
  detener su animación.
- Conservar una imagen congelada o thumbnail mientras un renderer se reconstruye.
- En Metal, compartir device, pipelines y buffers inmutables entre pantallas.
- Mantener una ruta web explícita para formatos que no puedan convertirse.
- Medir siempre procesos auxiliares de WebKit, no solo el proceso principal.

## Reposicionamiento y parámetros de los modelos

Sí es posible editar un modelo después de importarlo, pero hay dos niveles de
compatibilidad:

1. **Controles universales detectados automáticamente.** En módulos ES que
   exponen `.mesh`, `.group`, `.root` o `.model`, ParticleWall ofrece posición
   X/Y/Z, rotación X/Y/Z y escala. Si existen las propiedades conocidas,
   también expone velocidad (`speedMult`), campo de visión de cámara y los
   parámetros de bloom (intensidad, radio y umbral).
2. **Controles propios del modelo.** Un módulo puede declarar parámetros
   numéricos como densidad, amplitud, tamaño o color interpolado mediante el
   contrato `particleWallControls` y aplicar cada cambio en
   `setParticleWallParameter(id, value)`.

Contrato para un módulo importado:

```js
export default class MyWallpaper {
  particleWallControls = [
    {
      id: 'amplitude',
      label: 'Amplitud',
      category: 'Modelo',
      min: 0,
      max: 3,
      step: 0.05,
      defaultValue: 1
    }
  ];

  setParticleWallParameter(id, value) {
    if (id === 'amplitude') {
      this.amplitude = value;
    }
  }
}
```

También se acepta `getParticleWallControls()` en lugar de la propiedad. En este
primer contrato todos los valores son numéricos y deben tener un identificador
estable. Los controles que cambian geometría de forma costosa deben reutilizar
buffers o aplicar el cambio al terminar el arrastre; no deben reconstruir toda
la escena en cada frame.

La configuración se guarda fuera del `manifest.json`, con una clave por
wallpaper y UUID de pantalla. “Todas las pantallas” mantiene además un valor
comodín para monitores conectados en el futuro. Esto conserva compatibilidad con
los manifests existentes y permite posiciones diferentes por monitor.

Estado de implementación:

- [x] Puente Swift ↔ JavaScript para consultar y aplicar controles.
- [x] Detección automática de transformación, cámara, velocidad y bloom.
- [x] Contrato extensible para parámetros específicos del modelo.
- [x] Persistencia por wallpaper y pantalla con fallback global.
- [x] Editor SwiftUI con vista previa en vivo y restablecimiento.
- [x] Guardado explícito por fondo y restauración real de valores predeterminados.
- [x] Regeneración idempotente de módulos ES ya importados.
- [x] Extracción automática de `PARAMS`/`addControl(...)` en módulos generados.
- [x] Copia derivada `pw-user-module.js` sin modificar el módulo original.
- [x] Controles invariantes movidos fuera del loop principal de partículas.
- [x] Controles de color con selector nativo y persistencia RGB.
- [x] Tipo booleano para toggles nativos y del puente de controles.
- [ ] Tipos adicionales: listas y vectores agrupados.
- [ ] Validación/versionado explícito del contrato en `manifest v2`.
- [ ] Soporte equivalente para HTML arbitrario mediante una API opt-in.

## Plan por fases

### Fase 0: referencia y observabilidad

- [x] Inventario estructural con CodeGraph.
- [x] Verificación de compilación release.
- [ ] Captura inicial con Instruments en un equipo ejecutando ParticleWall.
- [x] Registrar CPU, RSS y FPS efectivos mediante comandos reproducibles.
- [ ] Registrar wakeups y tiempo GPU con Instruments.

Matriz mínima de medición:

| Escenario | Pantallas | FPS | Estado |
|---|---:|---:|---|
| Demo incluido | 1 | 30 | visible |
| Demo incluido | 1 | 15 | batería |
| Demo incluido | 2 | 30 | visible |
| HTML con bloom | 1 | 30 | visible |
| Cualquier wallpaper | 1 | — | bloqueado/reposo |
| Preview de galería | 1 | 30 | galería visible/oculta |

Cada captura debe calentarse durante 60 segundos y medirse durante al menos 5
minutos. Herramientas: Time Profiler, Allocations, Energy Log y, para el futuro
renderer Metal, Metal System Trace.

Comandos de verificación:

```bash
swift test -c release
./build-app.sh
~/Applications/ParticleWall.app/Contents/MacOS/ParticleWall \
  --diag --renderer-smoke-test
Scripts/measure-runtime.sh 300 /tmp/particlewall-runtime.csv
```

El script de medición incluye la app y la instancia más reciente de cada helper
WebKit. Para comparar resultados, cerrar antes Safari y otras apps WebKit.

### Fase 1: optimizaciones del renderer web

- [x] Diseñar la eliminación de la doble restauración de arranque.
- [x] Eliminar la segunda restauración del arranque.
- [x] Crear `WKWebView` de forma lazy y permitir que el controlador no tenga uno.
- [x] Liberar WebKit en Power Save, lock, sleep, sesión inactiva y pausa por batería.
- [x] Evitar que un preview continúe renderizando con la galería oculta.
- [x] Añadir teardown explícito de previews.
- [x] Mover el demo de partículas de un loop CPU a un shader.
- [x] Extraer controles invariantes del loop de exports generados.
- [x] Calidad adaptativa con degradación 60/30/24/20/15 FPS e histéresis.
- [ ] Revisar el template para no crear un sistema de partículas duplicado.

### Fase 2: abstracción y video nativo

- [x] Introducir `WallpaperRenderer` con operaciones de carga, pausa, FPS y
  liberación de recursos.
- [x] Implementar `WebWallpaperRenderer` usando el código existente.
- [ ] Aceptar `.mp4` y `.mov` en el manifest/importador.
- [ ] Implementar `VideoRenderer` con `AVQueuePlayer`, `AVPlayerLooper` y
  `AVPlayerLayer`, sin audio y sin impedir el reposo del display.

### Fase 3: partículas Metal

- [x] Crear `MetalParticleRenderer` sobre `MTKView`.
- [x] Generar posiciones, fases y colores proceduralmente en GPU.
- [x] Calcular animación desde shaders usando un uniforme de tiempo.
- [x] Controlar FPS con `preferredFramesPerSecond` y pausa con `isPaused`.
- [x] Liberar la vista y su delegate durante reposo.
- [x] Portar `Ondas Paramétricas` y conservar el Canvas 2D como fallback.
- [x] Portar `Vórtice Gemelo`, `Flor Orbital` y `Roseta Hexagonal`.
- [x] Portar `Lluvia de Ruido` con estado persistente en un compute shader y
  estelas almacenadas en GPU.
- [x] Portar `Espiral Prima` almacenando solo los 78.498 primos del millón de
  entradas original.
- [x] Portar `Órbita Toroidal` como una nube de 37.376 puntos en lugar de pedir
  la teselación `400×` de cada esfera y toro del sketch original.
- [x] Añadir ajuste al tamaño de pantalla y límites horizontal/vertical al
  editor nativo.
- [x] Sustituir el feedback de seis imágenes de la roseta por simetría generada
  directamente en el vertex shader.
- [x] Exponer transformación y velocidad del demo Metal al editor.
- [x] Portar `Anillos Cromáticos`: 6.225 puntos base y ocho muestras
  temporales sin aplicar blur al framebuffer completo.
- [x] Añadir grafo opcional por proximidad sobre 768 nodos representativos,
  con hasta tres vecinos y cero costo de búsqueda al desactivarlo.
- [x] Separar líneas y puntos en dos render passes para evitar una caída del
  driver Metal detectada por el smoke test.
- [x] Intensidad de puntos y líneas configurable hasta 10.

### Fase 4: formato declarativo y conversores

- [x] Campo opcional retrocompatible para `web` y las ocho variantes Metal.
- [ ] Manifest v2 formal con familias versionadas `web | video | metal`.
- [ ] Definir un `particles.json` versionado y validable.
- [ ] Convertir exports conocidos al formato nativo cuando sea seguro.
- [ ] Mostrar claramente en la galería qué motor usa cada wallpaper.

## Criterios de aceptación

Los objetivos se comparan contra el baseline del mismo equipo y wallpaper:

- Al menos 60% menos RSS para el demo nativo frente al demo web.
- Al menos 70% menos CPU para el demo nativo.
- Cero frames de render y cero timers de animación al estar en deep sleep.
- Una sola carga del wallpaper durante el arranque.
- Ningún preview activo después de ocultar/cerrar la galería.
- Tiempo GPU por frame por debajo del 50% de su presupuesto.
- Reconexión de monitor, cambio de escala y unlock sin pantalla permanentemente
  negra ni procesos WebKit huérfanos.

## Riesgos y decisiones

- JavaScript arbitrario no puede traducirse automáticamente a Metal. Los
  wallpapers no convertibles seguirán usando WebKit.
- Un loop de video reduce flexibilidad y puede aumentar almacenamiento; es una
  alternativa, no el reemplazo universal.
- La API pública de macOS configura imágenes de escritorio, no un renderer
  animado arbitrario. La ventana AppKit a nivel del escritorio se conserva.
- Las cifras finales deben venir de Instruments. No se asumirán mejoras solo por
  reducir líneas de código o cambiar de framework.

## Registro de avances

### 2026-07-22

- CodeGraph: 22 archivos, 226 símbolos y 428 relaciones indexadas.
- `swift build -c release`: compilación correcta antes de los cambios.
- Se confirmó que ParticleWall no estaba ejecutándose durante la auditoría, por
  lo que aún no existe un baseline de runtime representativo.
- Inicio de Fase 1: ciclo de vida lazy de WebKit, reposo profundo por estado del
  sistema y teardown de previews.
- Se eliminó `restoreAllAssignments()`: cada monitor restaura su contenido una
  sola vez y recibe la política energética antes de cargarlo.
- `WallpaperWindowController.webView` ahora es opcional. `clear()` y deep sleep
  detienen la carga, desconectan el delegate y liberan la instancia.
- Lock, screen sleep y sesión inactiva liberan WebKit inmediatamente; Power Save
  y pausa por batería intentan conservar el último frame, con timeout de 750 ms
  y fallback al thumbnail.
- Ocultar o cerrar la galería desmonta el preview y cancela su carga.
- `Demo Particles` calcula la respiración en el vertex shader: por frame solo se
  actualiza un uniforme de tiempo, no 6,000 posiciones ni el buffer completo.
- Los demos instalados se actualizan de forma idempotente sin reescribir ningún
  wallpaper importado por el usuario.
- Se añadió el target `ParticleWallTests` con cuatro pruebas de la política de
  reproducción y energía.
- Se añadió “Personalizar…” al menú contextual de cada wallpaper. Los módulos
  ES compatibles permiten editar transformación, cámara, velocidad, bloom y
  parámetros propios, con valores independientes por pantalla.
- La vista previa de los sliders se envía al renderer en vivo; UserDefaults se
  escribe al terminar el arrastre o cerrar el editor, evitando escrituras por
  cada evento del mouse.
- Se corrigió la detección tardía en módulos pesados: la consulta espera hasta
  dos segundos a que termine el módulo ES y acepta directamente los objetos de
  WebKit. Si la serialización falla, los módulos ES conservan un fallback de
  posición, rotación y escala en vez de mostrarse como incompatibles.
- Diagnóstico real sobre `tesseract2_0it (1)`: 12 controles detectados
  (transformación, velocidad, cámara y bloom), animación a 29 FPS efectivos y
  ningún error JavaScript.
- Verificaciones posteriores: `swift test -c release` (8/8), sintaxis JavaScript
  del demo y `git diff --check`, todas correctas.

### 2026-07-26

- Se incorporaron `WallpaperRenderer`, `WebWallpaperRenderer` y
  `MetalParticleRenderer`. El demo incluido selecciona Metal; los imports
  arbitrarios conservan WebKit.
- El primer demo Metal generaba 6.000 partículas en el vertex shader y enviaba
  únicamente uniforms. La prueba aislada dibujó 62 frames en dos segundos.
- El manifest acepta un campo `renderer` opcional. Los manifests anteriores
  siguen decodificando y los demos `bundled` migran a Metal automáticamente.
- `user-module.js` permanece intacto. La copia derivada `pw-user-module.js`
  enlaza `PARAMS` al editor y saca las lecturas `addControl` invariantes del
  loop principal.
- Diagnóstico de `tesseract2_0it (1)`: 61 frames/2 s, 3,43 ms de trabajo
  síncrono medio por callback, cero errores JS y 22 controles (12 universales
  más 10 propios).
- Se agregó calidad adaptativa configurable y
  `Scripts/measure-runtime.sh` para CPU/RSS. Una muestra corta del modelo web
  activo registró aproximadamente 23,5% CPU y 183 MB RSS combinados; todavía no
  sustituye el baseline de cinco minutos con Instruments.
- Verificaciones: `swift test -c release` (10/10), smoke test Metal,
  diagnóstico WebKit y `git diff --check`.
- Se interpretó y expandió una fórmula compacta p5.js de 10.000 puntos. Ahora
  `Ondas Paramétricas` la evalúa enteramente en el vertex shader Metal, mantiene
  un fallback Canvas 2D visualmente equivalente y es el wallpaper aplicado por
  defecto en todas las pantallas.
- El reloj Metal acumula únicamente tiempo dibujado para evitar saltos después
  de pausa o deep sleep. Smoke test de la nueva fórmula: 60 frames/2 s; renderer
  activo: 168 frames y 26,1 FPS medios durante el diagnóstico.
- El editor distingue vista previa y persistencia: cada UUID de wallpaper
  conserva su configuración por pantalla, Cmd-S guarda la versión actual y
  restaurar elimina los overrides globales o fija defaults explícitos en una
  pantalla para impedir que herede un perfil global personalizado.
- `Ondas Paramétricas` incorpora ColorPicker para fondo y partículas, además de
  tamaño y brillo. Los defaults ahora usan fondo `#030609`, partículas
  `#E8FFFF`, tamaño 1,6× y brillo 1,5× para mejorar contraste.
- Los wallpapers con `source: bundled` están protegidos tanto en el menú como
  en `LibraryManager.delete`. Si el fondo incluido falta por una versión
  anterior, el arranque lo reinstala sin sobrescribir imports ni cambiar una
  asignación activa válida.
- Se añadieron tres fórmulas procedurales sin reemplazar `Ondas Paramétricas`:
  `Vórtice Gemelo` (30.000 puntos), `Flor Orbital` (30.000 puntos) y
  `Roseta Hexagonal` (119.958 vértices para seis ramas).
- El catálogo instala y repara cada fondo incluido por su identificador de
  renderer. Los cuatro están protegidos y mantienen configuración independiente.
- El menú de tarjeta ya no depende del clic derecho: al hacer hover aparece
  `•••` en la esquina inferior derecha con aplicar, personalizar, FPS, renombrar,
  regenerar miniatura y mostrar en Finder.
- Smoke test simultáneo: las cuatro variantes completaron 60 frames en dos
  segundos a 30 FPS objetivo. Los tres fallbacks Canvas generaron miniaturas
  sin errores.
- `Lluvia de Ruido` añade 6.480 partículas con posición, gravedad y tamaño
  persistentes. Metal conserva doce muestras por partícula para sustituir el
  feedback `filter(BLUR)` sin procesar el framebuffer completo.
- Smoke test de `metal-noise-rain`: 61 frames en dos segundos (~29 FPS),
  77.760 vértices de estela. El fallback generó 150 frames sin errores.
- `Espiral Prima` evita mantener el arreglo Processing de 999.999 enteros:
  precalcula sus 78.498 primos una vez y conserva únicamente esos valores en
  un buffer Metal.
- `Órbita Toroidal` representa 64 toros con 36.864 puntos y la esfera de luz
  con 512, conservando el movimiento 3D sin la teselación extrema original.
- El editor incorpora el tipo booleano y una sección “Pantalla”: “Adaptar al
  tamaño de pantalla”, “Límite horizontal” y “Límite vertical”.
- Smoke test: ambos fondos completaron 60 frames en dos segundos (~28,5 FPS);
  sus fallbacks Canvas generaron 149/153 frames sin errores.

### 2026-07-27

- Se añadió `Anillos Cromáticos`, traducción nativa de once anillos HSB con
  ruido y deformación temporal. Usa 6.225 puntos base y ocho muestras de
  estela: 49.800 vértices sin filtro de framebuffer.
- Los ocho fondos distribuidos son protegidos y se reparan automáticamente.
- El editor guarda perfiles personales con ambos colores, nombre opcional,
  aplicación reutilizable entre fondos y eliminación independiente.
- Se añadió el modo “Grafo”: 768 nodos representativos buscan en GPU hasta tres
  vecinos dentro de una distancia visual configurable.
- El grafo se dibuja antes que los puntos en un render pass separado. Esta
  decisión corrige un `EXC_BAD_ACCESS` del driver al alternar topologías de
  línea y punto dentro del mismo encoder.
- La intensidad de puntos y líneas llega hasta 10. El alpha de las líneas se
  limita a un rango válido antes del blending.
- Verificación final: 20/20 pruebas; smoke test de los ocho renderers con
  grafo e intensidades `10/10`, 62 frames en dos segundos y aproximadamente
  29,1–29,4 FPS bajo un límite de 30.
