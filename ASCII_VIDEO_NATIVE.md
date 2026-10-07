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
