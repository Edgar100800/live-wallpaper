# ParticleWall: estado y registro de progreso

Actualizado: 2026-07-27

Este documento es la fuente de verdad del estado actual del proyecto. El
[`README.md`](README.md) contiene la guía rápida de uso,
[`PERFORMANCE_ROADMAP.md`](PERFORMANCE_ROADMAP.md) conserva la investigación y
el trabajo técnico pendiente, y [`Animated Wallpaper for Mac.md`](Animated%20Wallpaper%20for%20Mac.md)
es el plan histórico con el que comenzó la aplicación.

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
