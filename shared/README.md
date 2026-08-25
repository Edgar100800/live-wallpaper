# shared/

Assets y contratos consumidos por ambas plataformas. Nada aqui depende de
Swift, Rust ni del sistema operativo.

```
scripts/    JS compartido, fuente unica (macOS: WKWebView / Linux: WebKitGTK)
  raf-patch.js    parche de requestAnimationFrame (pausa, FPS cap, calidad adaptativa)
  dpr-clamp.js    clamp de devicePixelRatio; reemplazar __PW_CAP__ al inyectar
  harden.js       endurecimiento de contexto (document-end)
  test/           pruebas deterministas con reloj falso: node --test scripts/test/
contracts/
  schemas/        esquemas de formatos versionados
  fixtures/       casos neutral-formato leidos por cargo test y XCTest
    playback/policy.json               16 combinaciones de politica energetica
    navigation/path-containment.json   contencion de rutas local-only
    navigation/schemes.json            politica de esquemas URL
    manifest/migration-cases.json      manifest v1 -> id de modulo compartido
backgrounds/   modulos GPU compartidos (Fase M1+)
```

Reglas:

1. Los fixtures son la verdad: una implementacion que no los pasa esta rota.
2. Los cambios de contrato se hacen aqui primero, con fixture nuevo o
   modificado, y luego se propagan a Swift y Rust.
3. Los scripts JS deben mantenerse byte-identicos entre plataformas.
4. Prohibido acceso a red en cualquier prueba (NFR-01).
