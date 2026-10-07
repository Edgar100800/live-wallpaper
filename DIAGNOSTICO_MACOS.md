# Diagnóstico macOS: memoria y movimiento durante pausa

Fecha: 7 de octubre de 2026. Revisión del código actual del workspace, incluidos cambios locales. Herramientas: Codegraph CLI para símbolos y llamadas; Caveman para comunicación; análisis de código y pruebas JavaScript.

## Estado posterior: correcciones implementadas

Los hallazgos de este documento describen el estado previo. Se implementaron estas correcciones:

- Metal y ASCII no avanzan tiempo durante dibujos explícitos si MTKView está pausado. Noise Rain permite inicializar su primer fotograma, pero congela las actualizaciones posteriores de simulación.
- La captura periódica omite controladores pausados u ocultos. Las capturas explícitas siguen disponibles para guardar el último fotograma.
- La caché conserva una imagen por pantalla y elimina imágenes en memoria de pantallas desconectadas. La recuperación desde archivos persistidos sigue disponible.
- Los scripts web de macOS y compartido conservan hasta 100 errores, con máximo de 2048 caracteres por entrada.
- Se corrigieron asignaciones de propiedades ocultas por variables locales en los constructores Metal (`self.bufferSizes`) y ASCII (`self.pipeline`).
- Se agregaron pruebas macOS para capturas durante pausa, reanudación de Metal/ASCII, expulsión de imágenes al cambiar fondo y limpieza de pantallas desconectadas. Pendientes de ejecución en macOS.
- Pruebas JavaScript compartidas y contra script macOS extraído: 6/6 por variante. Comprobación de whitespace con `git diff --check`.

Las previews de galería siguen siendo independientes de la pausa del escritorio. Pausar CSS, video u otros mecanismos de un wallpaper web importado requiere ampliar su contrato de reproducción; no se cambió ese comportamiento en esta corrección.

## Alcance y límites

La inspección se realizó en Linux, sin Swift disponible. No se ejecutó la aplicación macOS ni Instruments. Los hallazgos siguientes identifican mecanismos verificables en código; no son mediciones de RAM, GPU ni confirmación de lo que ocurre en un binario instalado.

## 1. Movimiento de modelos nativos durante pausa

**Causa identificada, prioridad alta.** La pausa detiene el bucle automático de MTKView, pero las capturas periódicas siguen forzando dibujos que actualizan el tiempo de animación.

Cadena de llamadas verificada con Codegraph:

1. `WallpaperManager.startSnapshotTimer`, línea 484: captura cada 5 segundos y captura inicial al segundo.
2. `WallpaperManager.refreshSnapshotCache`, línea 497: excluye deep sleep y capturas pendientes, pero no pausa global ni oclusión.
3. `WallpaperWindowController.captureSnapshot`, línea 209: tampoco excluye pausa.
4. `MetalParticleRenderer.captureSnapshot`, `WallpaperRenderer.swift:496`: llama explícitamente a `metalView.draw()`.
5. `MetalParticleRenderer.draw`, línea 545: actualiza `animationTime` usando `min(0.1, now - lastDrawTime) * speed`, sin comprobar pausa.

`setPlayback`, línea 456, establece `metalView.isPaused` y reinicia `lastDrawTime`. La primera captura posterior fija ese tiempo; las siguientes pueden avanzar aproximadamente 0.1 segundos de animación por captura con `speed=1`. Eso explica pequeños saltos aproximadamente cada 5 segundos. Cambios de resolución durante la captura también pueden alterar la apariencia del fotograma.

El renderer ASCII repite el mecanismo: `ASCIIWallpaperRenderer.swift:180` fuerza `draw()` y línea 198 avanza tiempo sin comprobar pausa.

Apple documenta que, con `isPaused=true` y `enableSetNeedsDisplay=false`, `draw()` sigue permitiendo dibujo explícito: https://developer.apple.com/documentation/metalkit/mtkview/

**Corrección propuesta:** conservar estado explícito de pausa y no avanzar tiempo ni simulación mientras esté activo, aunque se necesite renderizar una captura. Evitar capturas periódicas redundantes durante pausa. Solo excluir capturas no cubre otras llamadas, como resincronización del escritorio.

## 2. Memoria retenida por imágenes antiguas

**Problema identificado, prioridad alta.** `LastFrameStore.swift:23` mantiene un diccionario fuerte `[Key: NSImage]`, cuya clave incluye pantalla y wallpaper. `cache`, línea 30, sustituye la imagen para la misma clave, pero acumula imágenes al cambiar a wallpapers diferentes.

No existe límite, presupuesto de bytes ni expulsión al cambiar wallpaper o desconectar pantalla. `removeStaleFiles`, línea 88, elimina archivos antiguos del disco, no entradas de memoria. La eliminación de memoria ocurre con `remove(wallpaperID:)`, línea 76, cuando se elimina el wallpaper de biblioteca.

Por tanto, no se acumula una imagen nueva cada 5 segundos para un mismo wallpaper: el crecimiento depende principalmente de parejas distintas pantalla/wallpaper visitadas. Es retención por caché sin límite, no evidencia de un ciclo ARC.

Ejemplo de tamaño bruto BGRA: 3840 × 2160 × 4 = **31.64 MiB por imagen**; 20 imágenes distintas equivalen a **632.81 MiB** de datos de píxeles. Son estimaciones, no RSS medido; representaciones de NSImage y recursos gráficos pueden cambiar el consumo real.

**Corrección propuesta:** conservar solo la imagen vigente por pantalla o implementar caché con presupuesto de bytes y expulsión. Limpiar pantallas desconectadas; mantener recuperación desde disco cuando corresponda.

## 3. Coste transitorio de capturas, incluso pausado

**Mecanismo identificado; impacto real pendiente de perfilado.** La captura Metal cambia temporalmente `drawableSize` a resolución física de pantalla (`WallpaperRenderer.swift:496`). La lectura crea buffer CPU y copia a `Data` (`encodeSnapshot`, línea 625), además del drawable y NSImage.

Esto genera asignaciones y trabajo GPU cada 5 segundos, incluso si la pausa detuvo renderizado continuo. No demuestra por sí solo fuga: los buffers de lectura quedan vinculados a finalización del command buffer. Con varios monitores, las capturas multiplican el coste. Alternar resolución de captura y resolución habitual puede añadir presión de asignación.

**Corrección propuesta:** reutilizar último fotograma pausado; evitar cambios innecesarios de tamaño y considerar capturas de menor resolución para caché.

## 4. Si el movimiento es continuo o aparece en galería

Las previews son independientes de los controladores del escritorio. `GalleryView.swift:349` crea WKWebView y línea 378 crea renderer ASCII, sin recibir la pausa global. Pueden seguir animadas mientras exista la preview; tienen desmontaje al retirarlas.

Para wallpapers web, `WebViewFactory.swift:145` bloquea callbacks de `requestAnimationFrame`, pero no pausa CSS animations, video, `setInterval`, workers ni eventos independientes. Este límite puede explicar movimiento continuo en contenido importado. Los fondos web incluidos revisados comprueban `__pwPaused` en sus bucles; no hay evidencia de fallo continuo de ese gate.

## 5. Otros riesgos de memoria

`WebViewFactory.swift:84` mantiene `__pwErrors` y añade errores/rechazos sin límite. Un wallpaper que emita errores repetidamente puede aumentar memoria durante toda la vida del documento. Corrección: buffer circular con máximo de entradas.

La pausa manual conserva renderer y recursos: `PlaybackPolicy.resolve`, `PowerManager.swift:11`, activa `paused` pero no `deepSleep` por `userPaused`. Por eso no debe esperarse una gran liberación de RAM al pulsar pausa. Power Save sí desmonta renderer mediante `WallpaperWindowController.enterDeepSleep`.

## Validación realizada

- Codegraph: índice informado actualizado; consultas de `setPlayback`, `captureSnapshot`, `cache` y llamadas a capturas periódicas.
- Pruebas existentes del script compartido: **5/5 pasan**.
- Mismas pruebas ejecutadas contra el JavaScript extraído de `WebViewFactory.swift`: **5/5 pasan**, incluida pausa sin ejecutar callback.
- Simulación de la fórmula temporal nativa: seis capturas a intervalos de 5 segundos, tras reiniciar `lastDrawTime`, avanzan 0.5 segundos de animación con `speed=1`. No ejecuta Metal ni reproduce visualmente macOS.
- No se modificó código de aplicación ni se ejecutaron pruebas AppKit/Metal.

## Confirmación pendiente en macOS

1. Cerrar previews y elegir modelo Metal. Pausar al menos 30 segundos; comprobar si cambia aproximadamente cada 5 segundos. Repetir con ASCII.
2. Cambiar entre 20 wallpapers distintos, esperar una captura por cada uno y observar memoria con Instruments Allocations / Memory Graph. Volver al inicial para distinguir reemplazo de acumulación por claves nuevas.
3. Comparar pausa manual y Power Save; observar también procesos WebContent/GPU cuando se use WebKit.
4. Tras corregir: exigir tiempo/simulación constante en pausa aun al capturar, y memoria de imágenes acotada al recorrer wallpapers.
