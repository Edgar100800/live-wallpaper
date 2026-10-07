# Revisión de estructura macOS y apertura en el escritorio actual

Fecha: 7 de octubre de 2026. Revisión del workspace actual con Codegraph y lectura de implementaciones. No se ejecutó la app macOS: el entorno de trabajo es Linux, sin Swift/Xcode.

## Cambio implementado en ventanas

`AppDelegate.swift` reutilizaba galería y Ajustes sin especificar comportamiento de Spaces. La aplicación se activaba antes de ordenar la ventana solicitada. Además, `toggleGallery` solo comprobaba `isVisible`: una galería visible en otro escritorio se ocultaba en lugar de abrirse en el actual.

Corrección aplicada:

- Galería y Ajustes usan `.moveToActiveSpace` y `.fullScreenAuxiliary`.
- Clic en barra superior oculta la galería solo si está visible, sin minimizar y en el Space activo. Si está en otro Space, solicita mostrarla.
- Las ventanas minimizadas se restauran antes de mostrarse.
- Se ordena la ventana solicitada antes de activar la aplicación, usando `NSApp.activate()` disponible desde macOS 14, versión mínima del paquete.
- Se reutilizan las ventanas y su estado de navegación. Los fondos de escritorio conservan su comportamiento independiente.

Apple describe `.moveToActiveSpace` como mover la ventana al Space activo cuando se activa, en vez de cambiar de Space: [documentación](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/movetoactivespace). La decisión de ocultar usa [isOnActiveSpace](https://developer.apple.com/documentation/appkit/nswindow/isonactivespace). `.fullScreenAuxiliary` permite participar en un Space de pantalla completa; su interacción con otras apps y Stage Manager requiere prueba en Mac.

## Estructura actual

| Capa | Archivos principales | Responsabilidad |
| --- | --- | --- |
| Arranque y barra superior | `main.swift`, `AppDelegate.swift` | Lifecycle, menús, galería, Ajustes y acciones CLI |
| Interfaz SwiftUI | `GalleryView.swift`, `SettingsView.swift`, `WallpaperCustomizationView.swift` | Biblioteca visual, preferencias y edición |
| Política de reproducción | `PowerManager.swift` | Pausa manual, batería, bloqueo y Power Save |
| Coordinación por pantalla | `WallpaperManager.swift` | Asignaciones, pantallas conectadas, capturas y escritorio del sistema |
| Lifecycle del renderer | `WallpaperWindow.swift` | Ventana de fondo, oclusión, carga y deep sleep |
| Renderizado | `WallpaperRenderer.swift`, `ASCIIWallpaperRenderer.swift`, `WebViewFactory.swift` | Metal, ASCII, WKWebView y contrato JavaScript |
| Biblioteca y entrada | `LibraryManager.swift`, `ImportPipeline.swift`, `Models.swift` | Importación, manifiestos, actualización y almacenamiento |
| Estado visual persistido | `LastFrameStore.swift`, `ThumbnailGenerator.swift`, `WallpaperControls.swift` | Fotogramas, miniaturas y controles guardados |
| Código común | `shared/backgrounds`, `shared/contracts`, `shared/scripts` | Shaders, módulos, fixtures y scripts para macOS/Linux |
| Verificación | `Tests/ParticleWallTests`, `shared/scripts/test` | Reproducción, snapshots, caché, controles y ASCII |

La división general ya es útil. No hace falta reescribir la aplicación. Los puntos de mayor mantenimiento son responsabilidades acumuladas en `WallpaperRenderer.swift`, `WallpaperManager.swift` y `WallpaperWindow.swift`, y duplicación de scripts entre `shared/scripts` y `WebViewFactory.swift`.

## Mejoras pendientes, por prioridad

### Alta: sincronización de capturas ASCII

`ASCIIWallpaperRenderer.captureSnapshot` dibuja y llama inmediatamente a `currentImage`. `draw` hace `commandBuffer.commit()`; `currentImage` lee `metalView.currentDrawable` y usa `texture.getBytes` sin esperar la finalización GPU.

Hay un mecanismo de lectura prematura: `commit()` programa el trabajo, y Apple indica que [getBytes no sincroniza operaciones GPU](https://developer.apple.com/documentation/metal/mtltexture/getbytes(_:bytesperrow:from:mipmaplevel:)). También conviene capturar la textura del drawable concreto durante el dibujo, evitando pedir de nuevo `currentDrawable` al terminar.

Recomendación: copiar a buffer de lectura mediante blit y entregar imagen en completion del command buffer, como hace el renderer de partículas. Verificar snapshots ASCII repetidos, resolución y colores con Metal API Validation. El fallo visual concreto no fue reproducido aquí.

### Alta: tamaños y desbordamientos al importar ASCII

`ASCIIFrameStore` calcula `columns * rows * 2` y `frameCount * bytesPerFrame` a partir de campos UInt32 del archivo, antes de comprobar longitud. Un archivo inválido puede desbordar Int y terminar el proceso, en lugar de producir el error de importación esperado.

Recomendación: usar operaciones con detección de overflow y fijar límites de celdas, dimensiones y memoria compatibles con el dispositivo. Añadir fixtures con cabeceras máximas, archivos truncados y tamaños fuera de presupuesto. Esta revisión no modifica el importador.

### Media: cola de thumbnails con recuperación

`ThumbnailGenerator` mantiene cola sin límite ni deduplicación. Tras llamar a `takeSnapshot`, depende de su callback para ejecutar `finish` y liberar `running`; el retraso de 2.5 s previo no es un timeout del snapshot.

Si una captura no completa, la cola puede quedar detenida y retener trabajos posteriores. Recomendación: token por trabajo, timeout, finalización idempotente y deduplicación por wallpaper. Verificar que un callback tardío de trabajo vencido no finalice otro trabajo.

### Media: coste de reconstruir pipelines Metal

Cada nuevo `MetalParticleRenderer` compila shader y crea pipelines de partículas, flujo y grafo, incluso si parte no se utiliza para ese fondo. Cambiar modelos o salir de deep sleep puede repetir trabajo.

Recomendación: medir coste de creación y compartir recursos inmutables por dispositivo si el perfil lo justifica. Los buffers de simulación deben seguir perteneciendo a cada renderer/pantalla.

### Media: garantías de CPU/GPU en buffers ASCII

ASCII rota tres buffers al subir celdas, sin seguimiento explícito de cuándo termina la GPU de leer cada uno. Revisar esta relación con el límite de drawables y las capturas adicionales antes de afirmar que existe corrupción.

Recomendación: validar bajo carga GPU y regular reutilización mediante completions o límite de trabajo en vuelo. Apple explica el patrón en [Synchronizing CPU and GPU work](https://developer.apple.com/documentation/metal/synchronizing-cpu-and-gpu-work).

### Mantenimiento: separar responsabilidades y automatizar pruebas Mac

- Extraer `MetalParticleRenderer` de `WallpaperRenderer.swift`, conservando protocolo y ownership. Evitar una reestructuración grande sin pruebas.
- Compartir implementación de scripts web mediante recursos generados o copiados al bundle, para que macOS y Linux no diverjan.
- Separar herramientas CLI y smoke tests del lifecycle de `AppDelegate` cuando crezcan.
- Añadir comprobación macOS en CI con build y pruebas Swift. No se encontró directorio `.github/workflows` en este workspace.
- Mantener prueba manual de Spaces, pantalla completa y Stage Manager: las pruebas de política no sustituyen el comportamiento del Window Server.

## Verificación del cambio de escritorio

Comprobación local: `git diff --check` y revisión de llamadas con Codegraph. No se afirma validación visual ni compilación macOS.

Prueba requerida en Mac:

1. Abrir galería en escritorio 1. Cambiar a escritorio 2 dejando galería abierta. Clic izquierdo en icono de barra: galería debe mostrarse en escritorio 2 sin volver al 1.
2. Repetir después de ocultar galería y después de minimizarla.
3. Segundo clic en el mismo escritorio debe ocultarla; siguiente clic debe mostrarla.
4. Repetir con «Abrir galería» del menú contextual y con Ajustes.
5. Repetir desde Space con otra app en pantalla completa, con Stage Manager y con dos monitores. Confirmar que no aparecen ventanas duplicadas.
6. Comprobar que búsqueda, selección y sheet de personalización siguen presentes al traer galería; el fondo continúa asignado a su pantalla.

Los hallazgos de mejora quedan como recomendaciones; el código modificado en esta revisión corresponde a apertura de ventanas en el escritorio actual.
