use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const MAGIC: &[u8; 8] = b"PWASCII1";
const HEADER_BYTES: u16 = 48;
const DEFAULT_CELL_SIZE: usize = 8;
const DEFAULT_EDGE_THRESHOLD: f32 = 0.075;

#[derive(Debug, Clone, Copy)]
struct VideoInfo {
    width: usize,
    height: usize,
    fps_num: u32,
    fps_den: u32,
}

#[derive(Debug, Clone, Copy)]
struct Options {
    cell_size: usize,
    edge_threshold: f32,
}

fn usage() -> &'static str {
    "usage: particlewall-ascii-converter --input VIDEO --output FILE.asciivideo [--cell-size N] [--fps N|NUM/DEN] [--edge-threshold N] [--preview FILE.png]"
}

fn parse_args() -> Result<(PathBuf, PathBuf, Options, Option<PathBuf>, Option<String>), String> {
    let mut input = None;
    let mut output = None;
    let mut preview = None;
    let mut cell_size = DEFAULT_CELL_SIZE;
    let mut edge_threshold = DEFAULT_EDGE_THRESHOLD;
    let mut fps_override = None;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0;

    while index < args.len() {
        let value = |name: &str, index: &mut usize| -> Result<String, String> {
            *index += 1;
            args.get(*index)
                .cloned()
                .ok_or_else(|| format!("missing value for {name}"))
        };
        match args[index].as_str() {
            "--input" => input = Some(PathBuf::from(value("--input", &mut index)?)),
            "--output" => output = Some(PathBuf::from(value("--output", &mut index)?)),
            "--preview" => preview = Some(PathBuf::from(value("--preview", &mut index)?)),
            "--cell-size" => {
                cell_size = value("--cell-size", &mut index)?
                    .parse()
                    .map_err(|_| "cell size must be a positive integer".to_string())?;
            }
            "--edge-threshold" => {
                edge_threshold = value("--edge-threshold", &mut index)?
                    .parse()
                    .map_err(|_| "edge threshold must be a number".to_string())?;
            }
            "--fps" => fps_override = Some(value("--fps", &mut index)?),
            "--help" | "-h" => {
                println!("{}", usage());
                std::process::exit(0);
            }
            unknown => return Err(format!("unknown argument {unknown}\n{}", usage())),
        }
        index += 1;
    }

    let input = input.ok_or_else(|| format!("missing --input\n{}", usage()))?;
    let output = output.ok_or_else(|| format!("missing --output\n{}", usage()))?;
    if !input.is_file() {
        return Err(format!("input does not exist: {}", input.display()));
    }
    if !(1..=64).contains(&cell_size) {
        return Err("cell size must be between 1 and 64".into());
    }
    if !(0.0..=1.0).contains(&edge_threshold) {
        return Err("edge threshold must be between 0 and 1".into());
    }
    Ok((
        input,
        output,
        Options {
            cell_size,
            edge_threshold,
        },
        preview,
        fps_override,
    ))
}

fn parse_rate(value: &str) -> Result<(u32, u32), String> {
    if let Some((numerator, denominator)) = value.split_once('/') {
        let numerator = numerator
            .parse()
            .map_err(|_| format!("invalid frame rate: {value}"))?;
        let denominator = denominator
            .parse()
            .map_err(|_| format!("invalid frame rate: {value}"))?;
        if numerator == 0 || denominator == 0 {
            return Err(format!("invalid frame rate: {value}"));
        }
        return Ok((numerator, denominator));
    }
    let whole: u32 = value
        .parse()
        .map_err(|_| format!("invalid frame rate: {value}"))?;
    if whole == 0 {
        return Err(format!("invalid frame rate: {value}"));
    }
    Ok((whole, 1))
}

fn probe(input: &Path, fps_override: Option<&str>) -> Result<VideoInfo, String> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate",
            "-of",
            "csv=p=0",
        ])
        .arg(input)
        .output()
        .map_err(|error| format!("could not execute ffprobe: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let line = String::from_utf8_lossy(&output.stdout);
    let fields: Vec<&str> = line.trim().split(',').collect();
    if fields.len() < 3 {
        return Err(format!("unexpected ffprobe output: {}", line.trim()));
    }
    let width = fields[0]
        .parse()
        .map_err(|_| format!("invalid video width: {}", fields[0]))?;
    let height = fields[1]
        .parse()
        .map_err(|_| format!("invalid video height: {}", fields[1]))?;
    let (fps_num, fps_den) = parse_rate(fps_override.unwrap_or(fields[2]))?;
    Ok(VideoInfo {
        width,
        height,
        fps_num,
        fps_den,
    })
}

fn write_u16(file: &mut File, value: u16) -> io::Result<()> {
    file.write_all(&value.to_le_bytes())
}

fn write_u32(file: &mut File, value: u32) -> io::Result<()> {
    file.write_all(&value.to_le_bytes())
}

fn write_header(file: &mut File, info: VideoInfo, cell_size: usize, frame_count: u32) -> io::Result<()> {
    let columns = info.width / cell_size;
    let rows = info.height / cell_size;
    file.write_all(MAGIC)?;
    write_u16(file, 1)?;
    write_u16(file, HEADER_BYTES)?;
    write_u16(file, cell_size as u16)?;
    write_u16(file, 0)?;
    write_u32(file, info.width as u32)?;
    write_u32(file, info.height as u32)?;
    write_u32(file, columns as u32)?;
    write_u32(file, rows as u32)?;
    write_u32(file, frame_count)?;
    write_u32(file, info.fps_num)?;
    write_u32(file, info.fps_den)?;
    write_u32(file, 0)?;
    Ok(())
}

fn rgb332(red: u32, green: u32, blue: u32) -> u8 {
    (((red >> 5) << 5) | ((green >> 5) << 2) | (blue >> 6)) as u8
}

fn luminance(red: u32, green: u32, blue: u32) -> f32 {
    (0.2127 * red as f32 + 0.7152 * green as f32 + 0.0722 * blue as f32) / 255.0
}

fn direction(dx: f32, dy: f32) -> u8 {
    let horizontal = dx.abs();
    let vertical = dy.abs();
    if horizontal > vertical * 2.0 {
        0
    } else if vertical > horizontal * 2.0 {
        1
    } else if dx * dy >= 0.0 {
        2
    } else {
        3
    }
}

fn encode_frame(frame: &[u8], info: VideoInfo, options: Options) -> Vec<u8> {
    let columns = info.width / options.cell_size;
    let rows = info.height / options.cell_size;
    let cell_count = columns * rows;
    let mut luma = vec![0.0f32; cell_count];
    let mut colors = vec![0u8; cell_count];

    for row in 0..rows {
        for column in 0..columns {
            let mut red = 0u32;
            let mut green = 0u32;
            let mut blue = 0u32;
            let mut count = 0u32;
            for y in 0..options.cell_size {
                for x in 0..options.cell_size {
                    let px = column * options.cell_size + x;
                    let py = row * options.cell_size + y;
                    let offset = (py * info.width + px) * 3;
                    red += frame[offset] as u32;
                    green += frame[offset + 1] as u32;
                    blue += frame[offset + 2] as u32;
                    count += 1;
                }
            }
            red /= count;
            green /= count;
            blue /= count;
            let index = row * columns + column;
            luma[index] = luminance(red, green, blue);
            colors[index] = rgb332(red, green, blue);
        }
    }

    let mut encoded = Vec::with_capacity(cell_count * 2);
    for row in 0..rows {
        for column in 0..columns {
            let index = row * columns + column;
            let center = luma[index];
            let left = luma[row * columns + column.saturating_sub(1)];
            let right = luma[row * columns + (column + 1).min(columns - 1)];
            let top = luma[row.saturating_sub(1) * columns + column];
            let bottom = luma[(row + 1).min(rows - 1) * columns + column];
            let dx = (right - left) * 0.5;
            let dy = (bottom - top) * 0.5;
            let neighbor_average = (left + right + top + bottom) * 0.25;
            let contrast = (center - neighbor_average).abs();
            let edge = (dx * dx + dy * dy).sqrt() > options.edge_threshold
                || contrast > options.edge_threshold;
            let glyph = if edge {
                10 + direction(dx, dy)
            } else {
                (center * 10.0).floor().min(9.0) as u8
            };
            encoded.push(glyph);
            encoded.push(colors[index]);
        }
    }
    encoded
}

fn render_preview(input: &Path, preview: &Path) -> Result<(), String> {
    if let Some(parent) = preview.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(input)
        .args(["-frames:v", "1", "-vf", "scale=640:-1"])
        .arg(preview)
        .status()
        .map_err(|error| format!("could not execute ffmpeg for preview: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("ffmpeg preview generation failed".into())
    }
}

fn convert(
    input: &Path,
    output: &Path,
    options: Options,
    preview: Option<&Path>,
    fps_override: Option<&str>,
) -> Result<(VideoInfo, u32), String> {
    let info = probe(input, fps_override)?;
    if info.width < options.cell_size || info.height < options.cell_size {
        return Err("video is smaller than cell size".into());
    }
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut file = File::create(output).map_err(|error| error.to_string())?;
    write_header(&mut file, info, options.cell_size, 0).map_err(|error| error.to_string())?;

    let filter = format!("fps={}/{}", info.fps_num, info.fps_den);
    let mut decoder = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(input)
        .args(["-an", "-vf"])
        .arg(filter)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|error| format!("could not execute ffmpeg: {error}"))?;
    let mut stdout = decoder.stdout.take().ok_or("ffmpeg stdout unavailable")?;
    let frame_bytes = info.width * info.height * 3;
    let mut frame = vec![0u8; frame_bytes];
    let mut frame_count = 0u32;
    loop {
        let first = stdout.read(&mut frame).map_err(|error| error.to_string())?;
        if first == 0 {
            break;
        }
        if first < frame_bytes {
            stdout
                .read_exact(&mut frame[first..])
                .map_err(|error| format!("incomplete decoded frame: {error}"))?;
        }
        file.write_all(&encode_frame(&frame, info, options))
            .map_err(|error| error.to_string())?;
        frame_count = frame_count.saturating_add(1);
    }
    let status = decoder.wait().map_err(|error| error.to_string())?;
    if !status.success() {
        return Err("ffmpeg frame decode failed".into());
    }

    file.seek(SeekFrom::Start(32)).map_err(|error| error.to_string())?;
    write_u32(&mut file, frame_count).map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())?;
    drop(file);
    if let Some(preview) = preview {
        render_preview(input, preview)?;
    }
    Ok((info, frame_count))
}

fn main() {
    let (input, output, options, preview, fps_override) = match parse_args() {
        Ok(value) => value,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    };
    match convert(
        &input,
        &output,
        options,
        preview.as_deref(),
        fps_override.as_deref(),
    ) {
        Ok((info, frame_count)) => println!(
            "created {}: {}x{} cells, {} frames, {}:{}, cell {}",
            output.display(),
            info.width / options.cell_size,
            info.height / options.cell_size,
            frame_count,
            info.fps_num,
            info.fps_den,
            options.cell_size
        ),
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{luminance, parse_rate, rgb332};

    #[test]
    fn parses_integer_and_fractional_rates() {
        assert_eq!(parse_rate("30").unwrap(), (30, 1));
        assert_eq!(parse_rate("30000/1001").unwrap(), (30000, 1001));
    }

    #[test]
    fn converts_rgb332_and_luminance() {
        assert_eq!(rgb332(255, 0, 0), 0b1110_0000);
        assert_eq!(rgb332(0, 255, 255), 0b0001_1111);
        assert!((luminance(255, 255, 255) - 1.0).abs() < 0.001);
    }
}
