//! Local, cancellable YouTube import. Only completed directories enter the library.
use super::library;
use std::os::unix::process::CommandExt;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub duration: f64,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Options {
    pub height: u32,
    pub fps: u32,
    pub start: f64,
    pub end: Option<f64>,
    pub cell_size: u32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            height: 720,
            fps: 30,
            start: 0.0,
            end: None,
            cell_size: 8,
        }
    }
}
#[derive(Clone, Debug)]
pub enum Event {
    Progress(String, f64),
    Analyzed(Video),
    Imported(String),
    Error(String),
}
pub type Cancel = Arc<AtomicBool>;

pub fn youtube_url(value: &str) -> Result<String, String> {
    let uri = gtk4::glib::Uri::parse(value.trim(), gtk4::glib::UriFlags::NONE)
        .map_err(|_| "URL inválida")?;
    let host = uri.host().ok_or("Falta el dominio")?;
    if uri.scheme() != "https"
        || ![
            "youtube.com",
            "www.youtube.com",
            "m.youtube.com",
            "youtu.be",
        ]
        .contains(&host.as_str())
        || uri.userinfo().is_some()
    {
        return Err("Pega una URL HTTPS de YouTube o youtu.be.".into());
    }
    let path = uri.path();
    let id = if host == "youtu.be" {
        path.trim_start_matches('/').to_string()
    } else if path == "/watch" {
        uri.query()
            .unwrap_or_default()
            .split('&')
            .find_map(|p| p.strip_prefix("v="))
            .unwrap_or("")
            .to_string()
    } else {
        path.strip_prefix("/shorts/")
            .or_else(|| path.strip_prefix("/embed/"))
            .unwrap_or("")
            .to_string()
    };
    if id.len() != 11
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("La URL debe apuntar a un video individual.".into());
    }
    Ok(format!("https://www.youtube.com/watch?v={id}"))
}
fn run(command: &mut Command, dir: &Path, cancel: &Cancel, timeout: u64) -> Result<String, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Importación cancelada.".into());
    }
    let out = dir.join("stdout.log");
    let err = dir.join("stderr.log");
    let mut child = command
        .stdin(Stdio::null())
        .stdout(fs::File::create(&out).map_err(|e| e.to_string())?)
        .stderr(fs::File::create(&err).map_err(|e| e.to_string())?)
        .process_group(0)
        .spawn()
        .map_err(|e| format!("No se pudo iniciar la herramienta: {e}"))?;
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) || started.elapsed().as_secs() > timeout {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .status();
            let _ = child.kill();
            let _ = child.wait();
            return Err(if cancel.load(Ordering::Relaxed) {
                "Importación cancelada."
            } else {
                "La operación agotó el tiempo de espera. Reintenta."
            }
            .into());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                // Logs may be large; show only a bounded tail.
                use std::io::{Seek, SeekFrom};
                let mut file = fs::File::open(&err).map_err(|e| e.to_string())?;
                let length = file.metadata().map_err(|e| e.to_string())?.len();
                file.seek(SeekFrom::Start(length.saturating_sub(2048)))
                    .map_err(|e| e.to_string())?;
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                return Err(format!(
                    "La herramienta falló: {}",
                    String::from_utf8_lossy(&bytes).trim()
                ));
            }
            return fs::read_to_string(out).map_err(|e| e.to_string());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn temporary() -> Result<Temporary, String> {
    let base = library::user_wallpapers_dir();
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = base.join(format!(".import-{}-{nonce}", std::process::id()));
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    Ok(Temporary(dir))
}
pub fn analyze(url: &str, cancel: &Cancel) -> Result<Video, String> {
    let url = youtube_url(url)?;
    let temp = temporary()?;
    let text = run(
        Command::new("yt-dlp").args([
            "--ignore-config",
            "--no-playlist",
            "--skip-download",
            "--dump-single-json",
            "--socket-timeout",
            "15",
            "--retries",
            "1",
            "--",
            &url,
        ]),
        &temp.0,
        cancel,
        90,
    )?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if value["is_live"].as_bool() == Some(true) {
        return Err(
            "Selecciona un video terminado; las transmisiones en vivo no se admiten.".into(),
        );
    }
    let video = Video {
        id: value["id"].as_str().ok_or("Falta ID del video")?.into(),
        title: value["title"].as_str().unwrap_or("Video YouTube").into(),
        duration: value["duration"]
            .as_f64()
            .ok_or("No se pudo determinar la duración")?,
    };
    if !video.duration.is_finite() || video.duration <= 0.0 {
        return Err("Duración de video inválida".into());
    }
    if video.id != url.rsplit('=').next().unwrap_or("") {
        return Err("El ID del video no coincide con la URL".into());
    }
    Ok(video)
}
fn duration(video: &Video, options: &Options) -> Result<f64, String> {
    let end = options.end.unwrap_or(video.duration);
    if ![480, 720, 1080].contains(&options.height)
        || ![15, 30].contains(&options.fps)
        || ![4, 6, 8, 12, 16].contains(&options.cell_size)
        || !options.start.is_finite()
        || !end.is_finite()
        || options.start < 0.0
        || end > video.duration + 0.1
        || end <= options.start
    {
        return Err("Revisa la calidad, la densidad y el intervalo de segundos.".into());
    }
    let length = end - options.start;
    if length > 300.0 {
        return Err("Selecciona un fragmento de hasta 300 segundos.".into());
    }
    Ok(length)
}
pub fn estimate(video: &Video, options: &Options) -> Result<u64, String> {
    Ok(
        (duration(video, options)? * options.fps as f64).ceil() as u64
            * (options.height as u64 / options.cell_size as u64)
            * (options.height as u64 * 2 / options.cell_size as u64)
            * 2
            + 48,
    )
}
pub fn import(
    url: &str,
    video: &Video,
    options: &Options,
    cancel: &Cancel,
    notify: &dyn Fn(Event),
) -> Result<String, String> {
    let url = youtube_url(url)?;
    if video.id != url.rsplit('=').next().unwrap_or("") {
        return Err("Analiza nuevamente la URL antes de convertir.".into());
    }
    let length = duration(video, options)?;
    let bytes = estimate(video, options)?;
    if bytes > 512 * 1024 * 1024 {
        return Err("El ASCII superaría 512 MiB. Recorta el video o reduce la calidad.".into());
    }
    let id = variant_id(video, options)?;
    let destination = library::user_wallpapers_dir().join(&id);
    if destination.exists() {
        return Err("Ese video y fragmento ya están en tu biblioteca.".into());
    }
    let temp = temporary()?;
    let converter = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("particlewall-ascii-converter");
    if !converter.is_file() {
        return Err(
            "Falta particlewall-ascii-converter. Ejecuta linux/install.sh para instalarlo.".into(),
        );
    }
    // Require room for the ASCII, normalized clip and bounded download.
    let free = run(
        Command::new("df").args(["-Pk"]).arg(&temp.0),
        &temp.0,
        cancel,
        5,
    )?;
    let available = free
        .lines()
        .last()
        .and_then(|l| l.split_whitespace().nth(3))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
        * 1024;
    if available < bytes * 2 + 1024 * 1024 * 1024 {
        return Err("Falta espacio: libera al menos 1 GiB más el tamaño del ASCII.".into());
    }
    let cached = source_path(video, options.height)?;
    let source = if cached.is_file() {
        notify(Event::Progress(
            "Usando el video original guardado…".into(),
            0.10,
        ));
        cached.clone()
    } else {
        notify(Event::Progress("Descargando video…".into(), 0.10));
        run(
            Command::new("yt-dlp")
                .args([
                    "--ignore-config",
                    "--no-playlist",
                    "--socket-timeout",
                    "15",
                    "--retries",
                    "1",
                    "--max-filesize",
                    "512M",
                    "--no-progress",
                    "-f",
                    &format!(
                        "bestvideo[height<={}]/best[height<={}]",
                        options.height, options.height
                    ),
                    "-o",
                ])
                .arg(temp.0.join("source.%(ext)s"))
                .args(["--", &url]),
            &temp.0,
            cancel,
            600,
        )?;
        let source = fs::read_dir(&temp.0)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("source.")
                    && p.extension().is_some_and(|e| e != "part" && e != "ytdl")
            })
            .ok_or("No se encontró el video descargado")?;
        source
    };
    notify(Event::Progress(
        "Preparando resolución y fragmento…".into(),
        0.35,
    ));
    let normalized = temp.0.join("normalized.mp4");
    run(Command::new("ffmpeg").args(["-nostdin", "-v", "error", "-y", "-ss", &options.start.to_string(), "-i"]).arg(&source).args(["-t", &length.to_string(), "-an", "-vf", &format!("scale=w='min(iw,{})':h='min(ih,{})':force_original_aspect_ratio=decrease:force_divisible_by=2,fps={}", options.height * 2, options.height, options.fps), "-c:v", "libx264", "-preset", "veryfast", "-crf", "18"]).arg(&normalized), &temp.0, cancel, 600)?;
    notify(Event::Progress(
        "Convirtiendo fotogramas a ASCII…".into(),
        0.55,
    ));
    run(
        Command::new(converter)
            .arg("--input")
            .arg(&normalized)
            .arg("--output")
            .arg(temp.0.join("clip.asciivideo"))
            .args([
                "--fps",
                &options.fps.to_string(),
                "--cell-size",
                &options.cell_size.to_string(),
            ]),
        &temp.0,
        cancel,
        1800,
    )?;
    run(
        Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-y", "-i"])
            .arg(&normalized)
            .args(["-frames:v", "1", "-vf", "scale=320:-1"])
            .arg(temp.0.join("thumbnail.png")),
        &temp.0,
        cancel,
        30,
    )?;
    notify(Event::Progress(
        "Preparando reproducción por bloques…".into(),
        0.90,
    ));
    split_clip(&temp.0, cancel)?;
    let template = library::resources_dir().join("SpiderManASCIIWallpaper");
    fs::copy(template.join("index.html"), temp.0.join("index.html")).map_err(|e| e.to_string())?;
    for atlas in ["fillASCII.png", "edgesASCII.png"] {
        fs::copy(template.join(atlas), temp.0.join(atlas)).map_err(|e| e.to_string())?;
    }
    fs::write(
        temp.0.join("metadata.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"id":video.id,"title":video.title,"duration":video.duration,"options":options})).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    for path in [
        &normalized,
        &temp.0.join("stdout.log"),
        &temp.0.join("stderr.log"),
    ] {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Importación cancelada.".into());
    }
    if source != cached {
        fs::create_dir_all(cached.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::rename(&source, &cached).map_err(|e| e.to_string())?;
    }
    fs::rename(&temp.0, destination).map_err(|e| e.to_string())?;
    Ok(id)
}
/// Source cache is shared by density variants at the same source quality.
fn source_path(video: &Video, height: u32) -> Result<PathBuf, String> {
    youtube_url(&format!("https://www.youtube.com/watch?v={}", video.id))?;
    Ok(library::user_wallpapers_dir()
        .parent()
        .unwrap()
        .join("sources")
        .join(format!("{}-{height}.video", video.id)))
}
fn variant_id(video: &Video, options: &Options) -> Result<String, String> {
    let length = duration(video, options)?;
    let base = format!(
        "youtube-{}-{}-{}-{}-{}",
        video.id,
        (options.start * 1000.0) as u64,
        (length * 1000.0) as u64,
        options.height,
        options.fps
    );
    Ok(if options.cell_size == 8 {
        base
    } else {
        format!("{base}-c{}", options.cell_size)
    })
}
/// Old imports stored only Video; infer their original conversion from the stable ID.
pub fn processing(id: &str) -> Result<(Video, Options, bool), String> {
    let wp = library::find(id).ok_or("No se encontró el fondo")?;
    if !wp.id.starts_with("youtube-") {
        return Err("El reprocesamiento está disponible para videos importados de YouTube.".into());
    }
    let text =
        fs::read_to_string(wp.index.with_file_name("metadata.json")).map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let video: Video = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    let options = if let Some(options) = value.get("options") {
        serde_json::from_value(options.clone()).map_err(|e| e.to_string())?
    } else {
        legacy_options(&wp.id, &video)?
    };
    duration(&video, &options)?;
    let cached = source_path(&video, options.height)?.is_file();
    Ok((video, options, cached))
}
fn legacy_options(id: &str, video: &Video) -> Result<Options, String> {
    let prefix = format!("youtube-{}-", video.id);
    let pieces: Vec<&str> = id
        .strip_prefix(&prefix)
        .ok_or("ID de video inválido")?
        .split('-')
        .collect();
    if pieces.len() != 4 {
        return Err("Datos de conversión antiguos inválidos".into());
    }
    let number = |index: usize| {
        pieces[index]
            .parse::<u32>()
            .map_err(|_| "Datos de conversión inválidos".to_string())
    };
    let start = number(0)? as f64 / 1000.0;
    Ok(Options {
        start,
        end: Some(start + number(1)? as f64 / 1000.0),
        height: number(2)?,
        fps: number(3)?,
        cell_size: 8,
    })
}
pub fn reprocess(
    id: &str,
    cell_size: u32,
    cancel: &Cancel,
    notify: &dyn Fn(Event),
) -> Result<String, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Importación cancelada.".into());
    }
    let (video, mut options, _) = processing(id)?;
    options.cell_size = cell_size;
    let target = variant_id(&video, &options)?;
    if library::find(&target).is_some() {
        notify(Event::Progress(
            "Usando la variante ya procesada…".into(),
            1.0,
        ));
        return Ok(target);
    }
    import(
        &format!("https://www.youtube.com/watch?v={}", video.id),
        &video,
        &options,
        cancel,
        notify,
    )
}

/// 60-frame chunks keep player memory bounded regardless of clip duration.
fn split_clip(dir: &Path, cancel: &Cancel) -> Result<(), String> {
    let mut file = fs::File::open(dir.join("clip.asciivideo")).map_err(|e| e.to_string())?;
    let mut header = [0u8; 48];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    let n = |offset| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap()) as usize;
    let (columns, rows, count, numerator, denominator) = (n(24), n(28), n(32), n(36), n(40));
    if &header[..8] != b"PWASCII1"
        || u16::from_le_bytes(header[8..10].try_into().unwrap()) != 1
        || u16::from_le_bytes(header[10..12].try_into().unwrap()) != 48
        || columns == 0
        || rows == 0
        || count == 0
        || numerator == 0
        || denominator == 0
    {
        return Err("Archivo ASCII inválido".into());
    }
    let frame_bytes = columns
        .checked_mul(rows)
        .and_then(|v| v.checked_mul(2))
        .ok_or("Dimensiones inválidas")?;
    if frame_bytes > 1024 * 1024
        || file.metadata().map_err(|e| e.to_string())?.len() != 48 + (frame_bytes * count) as u64
    {
        return Err("Tamaño ASCII inválido".into());
    }
    fs::create_dir(dir.join("frames")).map_err(|e| e.to_string())?;
    let mut buffer = vec![0; frame_bytes * 60];
    for (chunk, first) in (0..count).step_by(60).enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Importación cancelada.".into());
        }
        let size = frame_bytes * (count - first).min(60);
        file.read_exact(&mut buffer[..size])
            .map_err(|e| e.to_string())?;
        let mut out =
            fs::File::create(dir.join(format!("frames/{chunk}.bin"))).map_err(|e| e.to_string())?;
        out.write_all(&buffer[..size]).map_err(|e| e.to_string())?;
    }
    fs::write(dir.join("stream.json"), serde_json::to_vec(&serde_json::json!({"columns":columns,"rows":rows,"frameCount":count,"fps":numerator as f64 / denominator as f64,"chunkFrames":60})).unwrap()).map_err(|e| e.to_string())?;
    // Chunks are the runtime representation; don't keep a second full copy.
    drop(file);
    fs::remove_file(dir.join("clip.asciivideo")).map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn urls_are_single_youtube_videos() {
        assert_eq!(
            youtube_url("https://youtu.be/gU4vSEZwiyE?t=5").unwrap(),
            "https://www.youtube.com/watch?v=gU4vSEZwiyE"
        );
        for url in [
            "http://youtube.com/watch?v=gU4vSEZwiyE",
            "https://youtube.com.evil/watch?v=gU4vSEZwiyE",
            "https://youtube.com/playlist?list=x",
            "https://youtube.com/watch?v=../../x",
        ] {
            assert!(youtube_url(url).is_err());
        }
    }
    #[test]
    fn chunks_cover_final_frame_and_cancel_cleans_staging() {
        let temp = temporary().unwrap();
        let path = temp.0.clone();
        let mut header = [0u8; 48];
        header[..8].copy_from_slice(b"PWASCII1");
        header[8..10].copy_from_slice(&1u16.to_le_bytes());
        header[10..12].copy_from_slice(&48u16.to_le_bytes());
        for (offset, value) in [(24, 2u32), (28, 1), (32, 61), (36, 30), (40, 1)] {
            header[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        let mut bytes = header.to_vec();
        bytes.extend((0..244).map(|n| n as u8));
        fs::write(path.join("clip.asciivideo"), bytes).unwrap();
        split_clip(&path, &Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(fs::metadata(path.join("frames/0.bin")).unwrap().len(), 240);
        assert_eq!(
            fs::read(path.join("frames/1.bin")).unwrap(),
            vec![240, 241, 242, 243]
        );
        assert!(!path.join("clip.asciivideo").exists());
        drop(temp);
        assert!(!path.exists());
    }
    #[test]
    fn cancellation_stops_process_group() {
        let temp = temporary().unwrap();
        let token = Arc::new(AtomicBool::new(false));
        let other = token.clone();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            other.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        assert!(run(Command::new("sleep").arg("10"), &temp.0, &token, 20)
            .unwrap_err()
            .contains("cancelada"));
        worker.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn legacy_conversion_and_density_variants_remain_compatible() {
        let video = Video {
            id: "gU4vSEZwiyE".into(),
            title: "test".into(),
            duration: 21.0,
        };
        let options = legacy_options("youtube-gU4vSEZwiyE-0-21000-720-30", &video).unwrap();
        assert_eq!(options.cell_size, 8);
        assert_eq!(options.end, Some(21.0));
        assert_eq!(
            variant_id(&video, &options).unwrap(),
            "youtube-gU4vSEZwiyE-0-21000-720-30"
        );
        let dense = Options {
            cell_size: 4,
            ..options.clone()
        };
        assert!(variant_id(&video, &dense).unwrap().ends_with("-c4"));
        assert_eq!(
            estimate(&video, &dense).unwrap() - 48,
            (estimate(&video, &options).unwrap() - 48) * 4
        );
        assert!(duration(
            &video,
            &Options {
                cell_size: 0,
                ..options.clone()
            }
        )
        .is_err());
        let old: Options =
            serde_json::from_str(r#"{"height":720,"fps":30,"start":0,"end":21}"#).unwrap();
        assert_eq!(old, options);
        assert!(source_path(
            &Video {
                id: "../../unsafe".into(),
                ..video
            },
            720
        )
        .is_err());
    }
    #[test]
    fn intervals_and_storage_are_bounded() {
        let video = Video {
            id: "gU4vSEZwiyE".into(),
            title: "test".into(),
            duration: 600.0,
        };
        assert!(estimate(&video, &Options::default()).is_err());
        let options = Options {
            end: Some(21.0),
            ..Options::default()
        };
        assert_eq!(estimate(&video, &options).unwrap(), 20_412_048);
        assert!(duration(
            &video,
            &Options {
                start: f64::NAN,
                ..options
            }
        )
        .is_err());
    }
}
