//! ParticleWall daemon for Hyprland/Wayland.
//!
//! Usage:
//!   particlewall              start the daemon
//!   particlewall --pause      pause rendering on all outputs
//!   particlewall --resume     resume rendering
//!   particlewall --toggle     toggle pause
//!   particlewall --fps <N>    set global FPS cap (0 = unlimited)
//!   particlewall --list       list available wallpapers
//!   particlewall --apply <id|nombre>   switch wallpaper live
//!   particlewall --set-color background=#0a0a1a --set-color particle=#7ee0c0
//!   particlewall --set-color size=2.5 --set-color brightness=3
//!   particlewall --profile-save "Nombre"     save current colors as profile
//!   particlewall --profile-apply "Nombre"    apply a saved profile
//!   particlewall --profile-delete "Nombre"
//!   particlewall --status     print daemon state as JSON
//!   particlewall --app        launcher entry: ensure the daemon runs and
//!                             open the settings window

#[cfg(feature = "web")]
mod layer;
#[cfg(feature = "web")]
mod web;
#[cfg(feature = "web")]
mod control;
#[cfg(all(feature = "web", feature = "power"))]
mod power;
#[cfg(feature = "web")]
mod settings;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    #[cfg(feature = "web")]
    if args.first().map(String::as_str) == Some("--reprocess") {
        let result = args.get(1).ok_or_else(|| "Falta el ID del fondo".to_string()).and_then(|id| {
            let size = args.get(2).ok_or("Falta el tamaño de celda (4, 6, 8, 12 o 16)")?.parse().map_err(|_| "Tamaño de celda inválido")?;
            web::importer::reprocess(id, size, &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)), &|event| println!("{event:?}"))
        });
        match result { Ok(id) => println!("Reprocesado: {id}"), Err(error) => { eprintln!("{error}"); std::process::exit(1); } }
        return;
    }

    #[cfg(feature = "web")]
    if args.first().map(String::as_str) == Some("--import-youtube") {
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let result = args.get(1).ok_or_else(|| "Falta la URL".to_string()).and_then(|url| {
            let video = web::importer::analyze(url, &cancel)?;
            println!("{} · {} s", video.title, video.duration);
            web::importer::import(url, &video, &web::importer::Options::default(), &cancel, &|event| println!("{event:?}"))
        });
        match result { Ok(id) => println!("Importado: {id}"), Err(error) => { eprintln!("{error}"); std::process::exit(1); } }
        return;
    }

    if args.first().map(String::as_str) == Some("--app") {
        std::process::exit(app_mode());
    }

    if let Some(cmd) = cli_command(&args) {
        match cmd {
            CliAction::Print(text) => println!("{}", text.trim_end()),
            CliAction::Send(line) => std::process::exit(cli::send_and_print(line)),
        }
        return;
    }

    #[cfg(feature = "web")]
    {
        web::run();
    }
    #[cfg(not(feature = "web"))]
    {
        eprintln!("particlewall was built without the `web` feature.");
        eprintln!("Rebuild with --features web, or use pw-layer-spike for the no-webkit spike.");
        std::process::exit(2);
    }
}

/// Desktop-launcher mode: make sure the daemon is running, then open the
/// settings window. Never starts a second daemon (the systemd service owns
/// the daemon lifecycle; a manual detached fallback keeps dev setups working).
fn app_mode() -> i32 {
    let open = || cli::send_and_print(r#"{"cmd":"open-settings"}"#.into());

    if cli::daemon_reachable() {
        return open();
    }

    // Preferred path: the installed user service (idempotent when active).
    let started = std::process::Command::new("systemctl")
        .args(["--user", "start", "particlewall.service"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !started {
        // Fallback for repos without the service installed: spawn a detached
        // daemon (own process group, output discarded).
        use std::os::unix::process::CommandExt;
        let exe = std::env::current_exe().expect("current exe");
        std::process::Command::new(exe)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .expect("spawn detached daemon");
    }

    // Wait for the control socket to appear.
    for _ in 0..50 {
        if cli::daemon_reachable() {
            return open();
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    eprintln!("particlewall: daemon did not come up");
    3
}

enum CliAction {
    /// Answer locally, no daemon needed.
    Print(String),
    /// Forward a JSON command line to the daemon socket.
    Send(String),
}

/// Builds a profile command requiring a name argument.
fn profile_cmd(kind: &str, args: &[String]) -> String {
    let name = args
        .get(1)
        .unwrap_or_else(|| {
            eprintln!("{kind} requires a profile name");
            std::process::exit(64);
        })
        .clone();
    serde_json::json!({"cmd": kind, "name": name}).to_string()
}

/// Parses "#RRGGBB", "RRGGBB" or a plain integer into the wire value the
/// wallpaper contract expects (packed RGB integer; float for size/brightness).
fn parse_color_value(value: &str, wire: &str) -> serde_json::Value {
    if wire == "particleSize" || wire == "brightness" {
        return serde_json::json!(value.parse::<f64>().unwrap_or_else(|_| {
            eprintln!("{wire} must be a number, got '{value}'");
            std::process::exit(64);
        }));
    }
    let hex = value.trim_start_matches('#');
    let packed = u32::from_str_radix(hex, 16)
        .or_else(|_| value.parse::<u32>())
        .unwrap_or_else(|_| {
            eprintln!("color must be #RRGGBB or an integer, got '{value}'");
            std::process::exit(64);
        });
    serde_json::json!(packed)
}

fn cli_command(args: &[String]) -> Option<CliAction> {
    match args.first().map(String::as_str) {
        Some("--pause") => Some(CliAction::Send(r#"{"cmd":"pause"}"#.into())),
        Some("--resume") => Some(CliAction::Send(r#"{"cmd":"resume"}"#.into())),
        Some("--toggle") => Some(CliAction::Send(r#"{"cmd":"toggle"}"#.into())),
        Some("--settings") => Some(CliAction::Send(r#"{"cmd":"open-settings"}"#.into())),
        Some("--status") => Some(CliAction::Send(r#"{"cmd":"status"}"#.into())),
        Some("--fps") => {
            let v = args.get(1).expect("--fps requires a value (0 = unlimited)");
            v.parse::<u32>().ok().map(|v| CliAction::Send(format!(r#"{{"cmd":"fps","value":{v}}}"#)))
                .or_else(|| {
                    eprintln!("--fps expects an integer (0 = unlimited), got '{v}'");
                    None
                })
        }
        Some("--apply") => {
            let id = args.get(1).unwrap_or_else(|| {
                eprintln!("--apply requires an id or name; see particlewall --list");
                std::process::exit(64);
            });
            let line = serde_json::to_string(&serde_json::json!({
                "cmd": "apply",
                "value": id,
            }))
            .expect("serialize command");
            Some(CliAction::Send(line))
        }
        Some("--set-color") => {
            // Repeatable: --set-color background=#0a0a1a --set-color particle=#7ee0c0
            //             --set-color size=1.5
            let mut colors = serde_json::Map::new();
            let mut rest: &[String] = args;
            while let Some(Some(flag)) = rest.first().map(String::as_str).map(Some) {
                if flag != "--set-color" {
                    break;
                }
                let spec = rest.get(1).unwrap_or_else(|| {
                    eprintln!("--set-color requires key=value");
                    std::process::exit(64);
                });
                let (key, value) = spec.split_once('=').unwrap_or_else(|| {
                    eprintln!("--set-color expects key=value (got '{spec}')");
                    std::process::exit(64);
                });
                let wire = match key.to_ascii_lowercase().as_str() {
                    "background" | "bg" => "backgroundColor",
                    "particle" | "fg" => "particleColor",
                    "size" | "particlesize" => "particleSize",
                    "brightness" | "intensity" => "brightness",
                    other => {
                        eprintln!(
                            "unknown color key '{other}' (background|particle|size|brightness)"
                        );
                        std::process::exit(64);
                    }
                };
                let parsed = parse_color_value(value, wire);
                colors.insert(wire.into(), parsed);
                rest = &rest[2..];
            }
            if colors.is_empty() {
                eprintln!("--set-color requires at least one key=value");
                std::process::exit(64);
            }
            Some(CliAction::Send(
                serde_json::json!({"cmd":"set-colors","colors":colors}).to_string(),
            ))
        }
        Some("--profiles") => Some(CliAction::Send(r#"{"cmd":"status"}"#.into())),
        Some("--profile-save") => Some(CliAction::Send(profile_cmd("profile-save", args))),
        Some("--profile-delete") => Some(CliAction::Send(profile_cmd("profile-delete", args))),
        Some("--profile-apply") | Some("--profile") => {
            Some(CliAction::Send(profile_cmd("profile-apply", args)))
        }
        Some("--list") => {
            #[cfg(feature = "web")]
            {
                let mut out = String::new();
                for wp in web::library::all() {
                    out.push_str(&format!("{}\t{}\n", wp.id, wp.name));
                }
                Some(CliAction::Print(out))
            }
            #[cfg(not(feature = "web"))]
            Some(CliAction::Print(String::new()))
        }
        _ => None,
    }
}

/// Client mode: connect to the daemon control socket, send one command,
/// print the JSON reply.
mod cli {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    pub fn send_and_print(command: String) -> i32 {
        let path = control_socket_path();
        let Ok(mut stream) = UnixStream::connect(&path) else {
            eprintln!("particlewall: daemon not reachable at {} — is it running?", path.display());
            return 3;
        };
        stream.set_read_timeout(Some(Duration::from_secs(3))).ok();

        if writeln!(stream, "{command}").is_err() {
            eprintln!("particlewall: failed to send command");
            return 4;
        }
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => {
                eprintln!("particlewall: no reply from daemon");
                return 5;
            }
            Ok(_) => {
                print!("{line}");
                0
            }
        }
    }

    /// True when the daemon control socket accepts connections.
    pub fn daemon_reachable() -> bool {
        UnixStream::connect(control_socket_path()).is_ok()
    }

    fn control_socket_path() -> std::path::PathBuf {
        let runtime =
            std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        std::path::Path::new(&runtime).join("particlewall.sock")
    }
}
