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
//!   particlewall --profile-save "Nombre"     save current colors as profile
//!   particlewall --profile-apply "Nombre"    apply a saved profile
//!   particlewall --profile-delete "Nombre"
//!   particlewall --status     print daemon state as JSON

#[cfg(feature = "web")]
mod layer;
#[cfg(feature = "web")]
mod web;
#[cfg(feature = "web")]
mod control;
#[cfg(all(feature = "web", feature = "power"))]
mod power;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

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
/// wallpaper contract expects (packed RGB integer; float for size).
fn parse_color_value(value: &str, wire: &str) -> serde_json::Value {
    if wire == "particleSize" {
        return serde_json::json!(value.parse::<f64>().unwrap_or_else(|_| {
            eprintln!("size must be a number, got '{value}'");
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
                    other => {
                        eprintln!("unknown color key '{other}' (background|particle|size)");
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
                for wp in web::library::bundled() {
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

    fn control_socket_path() -> std::path::PathBuf {
        let runtime =
            std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        std::path::Path::new(&runtime).join("particlewall.sock")
    }
}
