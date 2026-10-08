# ASCII Video Native

`ascii-video-v1` stores precomputed ASCII cells, not a decoded video. Runtime uses a native renderer to draw those cells with Metal on macOS and wgpu on Linux.

## Why this engine exists

`AcerolaFX_ASCII.fx` is a ReShade post-process. It receives a game backbuffer, computes luminance, Difference of Gaussians, Sobel direction and glyph selection every frame. A normal video has no depth buffer or normal buffer, so only color/luminance-derived edges apply.

Running that whole graph for every wallpaper frame wastes GPU work. `ascii-video-v1` moves expensive work to import time:

```text
source video -> ffmpeg decode -> cell analysis -> .asciivideo -> native glyph renderer
```

Runtime does not decode video, run blur, run Sobel or inspect full-resolution pixels. It uploads a small cell buffer and draws glyph quads from two atlas textures.

## Binary format

File extension: `.asciivideo`

Header is 48 bytes, little endian:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 8 | Magic `PWASCII1` |
| 8 | 2 | Format version, currently `1` |
| 10 | 2 | Header size, currently `48` |
| 12 | 2 | Source cell size, normally `8` |
| 14 | 2 | Reserved |
| 16 | 4 | Source width |
| 20 | 4 | Source height |
| 24 | 4 | Cell columns |
| 28 | 4 | Cell rows |
| 32 | 4 | Frame count |
| 36 | 4 | FPS numerator |
| 40 | 4 | FPS denominator |
| 44 | 4 | Flags, currently `0` |

Frames follow consecutively. Every cell uses two bytes:

| Byte | Meaning |
|---:|---|
| 0 | Glyph ID: `0..9` fill glyph, `10..13` edge glyph |
| 1 | RGB332 color (`RRRGGGBB`) |

Cell order is row-major. RGB332 keeps runtime bandwidth low. Renderer expands it to linear RGB in the fragment shader.

## Offline converter

Build and test:

```bash
cargo test --manifest-path tools/ascii-converter/Cargo.toml
cargo run --manifest-path tools/ascii-converter/Cargo.toml -- \
  --input input.mp4 \
  --output output.asciivideo \
  --preview preview.png
```

Converter requirements:

- `ffprobe` for source dimensions and rate.
- `ffmpeg` for decoded RGB frames.
- CPU analysis only during import.

Options:

- `--cell-size 8`: pixels per ASCII cell.
- `--fps 30` or `--fps 30000/1001`: output sampling rate.
- `--edge-threshold 0.075`: cell-space edge threshold.
- `--preview path.png`: writes source preview for gallery fallback.

Current converter uses average cell color, cell luminance, neighbor contrast and gradient direction. This is the video-safe equivalent of the AcerolaFX graph. Depth and normal passes are intentionally absent.

## Native runtime

Native renderer contract:

- Load `.asciivideo` and validate magic, version, dimensions and payload length.
- Load `fillASCII.png` and `edgesASCII.png` with point filtering.
- Keep decoded cell data in a mapped or shared GPU buffer.
- Advance frame from monotonic elapsed time and fixed file FPS.
- Pause by stopping frame advancement, not by destroying renderer.
- Use triple buffering when cell data is uploaded while GPU draws.
- Render one quad per cell. No full-resolution post-process target required.

For a 1920x1080 source and 8px cells:

```text
240 x 135 = 32,400 cells per frame
64,800 bytes per frame in file representation
~1.94 MB/s at 30 FPS before GPU alignment
```

Runtime cost is bounded by cell count, not source pixel count.

## Wallpaper layout

An imported native ASCII wallpaper contains:

```text
index.html                 required legacy library marker
clip.asciivideo            native frame data
thumbnail.png              optional gallery thumbnail
```

Manifest renderer value is `ascii-video`. Existing manifest v1 remains readable. Future manifest v2 should model this as `engine: "ascii-video-v1"`, not as a `particle-v1` model.

## Atlas mapping

`fillASCII.png` is 80x8: ten 8x8 fill glyphs.

`edgesASCII.png` is 40x8: four edge directions after the empty atlas slot. Runtime maps glyph IDs `10..13` to atlas columns `1..4`.

Assets originate from AcerolaFX, which is MIT licensed. Keep the AcerolaFX copyright and license notice when redistributing those assets.

## Linux wgpu renderer

`particlewall-render::ascii` plays ascii-video-v1 clips without WebKit.
`shared/backgrounds/engines/ascii-video-v1/ascii.wgsl` is a direct port of the
WebGL2 shader in `SpiderManASCIIWallpaper/index.html`, including its atlas
sampling (nearest texel at `local * 8 + 0.5` after `UNPACK_FLIP_Y`). One
full-screen triangle per frame; the cell grid stretches over the output like
the WebGL canvas.

- `AsciiClip` reads both layouts: bundled `clip.asciivideo` and imported
  `stream.json` + `frames/<n>.bin`, with the same validation limits as the
  WebGL player. It reads only the current frame (`pread`), so memory does not
  grow with clip length.
- `AsciiPlayer` advances clip time with wall time (delta clamped to 0.1 s, no
  speed multiplier), reads and uploads cells only when the clip frame changes,
  and keeps the last good frame on read errors.
- The daemon prefers this renderer over WebKit for every wallpaper with a native
  clip; the WebKit page stays as the fallback when the GPU path is unavailable.
  Pause stops clip time; palette, ASCII settings and wallpaper changes repaint a
  paused frame once.

Parity: the `ascii` tests compare every pixel of synthetic and bundled frames
against a CPU transcription of the WebGL shader in all four color/cleanup
combinations. Checked once against the real page in Chromium (WebGL2 on
SwiftShader, canvas exported with `toDataURL`) at 2560x1440: original colors
100% identical, Omarchy palette with cleanup 99.86% identical and the rest within
1/255. Resolutions with an integer number of pixels per cell (1920x1080 for a
240x135 grid) put every pixel center on an atlas texel boundary, where WebGL and
wgpu may round to different texels; avoid them for parity checks.

## Omarchy palette and visual separation

On Linux the daemon reads
`$XDG_STATE_HOME/omarchy/current/theme/colors.toml` (default
`~/.local/state/omarchy/current/theme/colors.toml`), with the older config-directory
location as fallback. Palette fields are `background`, `ink` and `highlight`,
each an RGB triplet in `0..1`. Ink prefers `cyan`, then `accent`, then
`foreground`; highlights prefer `bright_cyan`, then `foreground`. The native
renderer receives them as uniforms; the WebKit fallback receives
`window.__pwSystemPalette`.

The daemon checks for palette changes every two seconds and updates the
renderer only when colors change. The wallpaper repaints even when
paused, preserving its current frame. No desktop theme files or saved manual
ParticleWall colors are changed. Without a valid system palette, original RGB332
rendering remains available, including on macOS.

This clip has no depth or segmentation channel. Its bright sky is suppressed with
`1 - smoothstep(cutoff - 0.30, cutoff, luminance)`, where the separation slider
sets `cutoff` between `0.78` and `0.38` (default `0.58`); local four-neighbor contrast attenuates
flat regions. Removed glyphs become the solid theme background. Dark buildings
can still remain, and bright parts of the subject can be attenuated: this is a
clip-specific visual approximation, not physical depth detection.

For accurate separation on other clips, generate a temporally stable subject
mask or depth map from the original video during conversion, store one mask value
per cell per frame in a versioned format or sidecar, then use that value to gate
glyph visibility. Keep inference offline so wallpaper playback stays inexpensive.

## Resource policy

Import cost is acceptable because it happens once. Runtime must not invoke ffmpeg, AVFoundation decode or full-resolution image analysis. Renderer may cache current and next frame only; loading all frames into memory is unnecessary.

4K sources should use either a larger cell size or a lower analysis resolution. Native rendering can scale the cell grid to monitor bounds without changing frame data.

## Verification

Required checks:

1. Converter rejects truncated input and writes zero-frame files only on explicit empty decode.
2. Reader rejects bad magic, unsupported version and payload length mismatch.
3. First frame thumbnail matches native renderer orientation.
4. Pause freezes current frame and uses zero analysis work.
5. Deep sleep tears down native view while preserving last-frame snapshot.
6. Same `.asciivideo` cell sequence renders on Metal and wgpu within documented color tolerance.

## Unified Linux settings

The resizable GTK editor starts at 900x760 (tiling compositors may allocate a
smaller or larger area). It exposes wallpaper selection, pause, FPS, ASCII color
mode, independent background cleanup and separation strength, particle appearance
and saved color profiles. Inapplicable controls are disabled for ASCII wallpapers.

Changes send `configure` commands with `save: false` for live preview. Only
**Guardar cambios** writes the full configuration, using a temporary file and an
atomic rename; write failures appear in the editor. **Cancelar** or closing the
window restores the last saved preview baseline. **Restaurar valores** previews
default appearance, 30 FPS and the active wallpaper's default ASCII settings;
other wallpaper settings and saved profiles remain intact.

`config.json` adds `fpsCap` and `asciiSettings`, a map keyed by wallpaper ID.
Each ASCII entry stores `colorMode` (`original` or `omarchy`), `cleanBackground`
and `separation` (`0..1`). Older configurations default to 30 FPS and the existing
Omarchy treatment. The color mode and cleanup are independent. Runtime changes
repaint a paused frame without decoding or reloading the clip.

GUI regression: `PARTICLEWALL_GUI_TEST=1 cargo test --manifest-path linux/Cargo.toml
-p particlewall-linux --test web-lifecycle`. This checks one-window reuse, preview,
explicit save, cancellation and independent ASCII controls on GTK's main thread.

## YouTube import on Linux

The **Importar video** settings tab includes **Agregar video de YouTube**: analyze a single HTTPS URL, choose
480/720/1080p, 15/30 FPS and 4/6/8/12/16px cells, select a start/end interval, then convert. Imports run
on a worker thread while the current wallpaper continues playing. Closing the
settings window or cancelling kills the active subprocess group and removes the
staging directory. A completed import is added to the picker as a preview;
**Guardar cambios** makes it the persisted wallpaper.

Requirements: `yt-dlp`, `ffmpeg`, `ffprobe`, and `particlewall-ascii-converter`
beside the daemon executable. `linux/install.sh` builds and places the converter.
YouTube errors are surfaced in the import panel; no browser credentials are read.

Limits: individual completed videos, no audio, a maximum 300-second selected
interval, at most 512 MiB estimated ASCII output, and at most 512 MiB source
video. The importer checks available disk space before downloading. Download,
normalization and conversion have bounded timeouts. Progress describes stages,
not a measured percentage of downloaded bytes.

Completed imports live in `$XDG_DATA_HOME/particlewall/wallpapers` (default
`~/.local/share/particlewall/wallpapers`). Hidden staging directories never enter
the library. Existing built-in wallpapers and configuration remain compatible.

Imported clips contain `metadata.json`, `thumbnail.png`, `index.html`, the two
atlases, `stream.json` and `frames/N.bin`. The converter first writes v1 data;
the importer validates and splits it into 60-frame blocks, then removes the full
copy. The downloaded source is retained in the shared source cache. The player caches the current and following blocks,
pausing advancement if a block is not yet available. At 720p, 30 FPS and 8px
cells, typical 16:9 cache payload is about 3.5 MB rather than growing with video
length. Legacy bundled `.asciivideo` files retain their original loading path.

For command-line import (does not change the active wallpaper):

```sh
linux/target/release/particlewall --import-youtube 'https://www.youtube.com/watch?v=gU4vSEZwiyE'
```

The CLI uses default 720p/30 FPS and the full video, so videos over 300 seconds
must be imported through settings with an explicit interval.

Validated on this Omarchy PC with `gU4vSEZwiyE`: real metadata lookup from GTK,
download and conversion to 160x90 cells, 627 frames at 30 FPS, original/Omarchy
color modes and pause (zero changed wallpaper pixels). Eight daemon/WebKit
processes remained present during a 50-second sample; aggregate PSS was
542.8–547.7 MiB. This includes WebKit and GPU resources and is a short playback
check, not proof against long-term leaks. Unit checks cover URL rejection,
interval/storage limits, final partial chunks and subprocess cancellation.


## Density and reprocessing on Linux

Settings groups controls into **Fondo**, **Video ASCII**, **Importar video**, and
**Partículas** tabs, with preview/save/cancel in a shared footer. **Video ASCII**
contains live appearance on the left and density conversion on the right.
Density uses the converter cell size: 4px (very high), 6px (high), 8px (medium,
legacy default), 12px (low), or 16px (very low). Changing this selector alone does
not alter the playing file: click **Reprocesar video** to create a new variant,
then save settings to persist its selection. The previous variant remains usable.

A shared source cache lives beside the wallpaper library, at
`$XDG_DATA_HOME/particlewall/sources/<youtube-id>-<height>.video`. New successful
imports retain the downloaded video here. Density variants share that source;
normalization remains temporary. Reprocessing keeps the same resolution, FPS and
start/end interval. If a previous density variant exists, it is selected without
repeating conversion. The picker lists imported variants with their cell size.

Older imports stored only video metadata and deleted their source. Their original
conversion settings are inferred from the existing stable ID, keeping 8px IDs
compatible. The UI explicitly offers **Recuperar original y reprocesar** when the
source is absent. This downloads it once; later density changes use the local
cache. Bundled ASCII clips have no original-video metadata and cannot be
reprocessed through this feature.

New `metadata.json` files retain title/ID/duration plus `options` (height, FPS,
start/end, `cell_size`). Variants use a `-c<N>` suffix except legacy 8px. Work is
staged and published by directory rename; cancellation never changes the playing
variant or removes a previously cached source. Disk estimates and the 512 MiB
ASCII output limit scale with density.

CLI: `particlewall --reprocess <wallpaper-id> <cell-size>` creates or reuses a
variant without switching the active wallpaper. Validation on this PC recovered
Gargantua's original for 4px (320x180 cells), then generated 16px (80x45 cells)
while a deliberately failing `yt-dlp` executable was on PATH: the download tool
was never invoked. Both conversions retain the 627-frame, 30 FPS sequence.
