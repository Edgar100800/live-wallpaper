//! xtask: WGSL validation and reproducible MSL generation (PRD 9.4).
//!
//! particle.wgsl is the single editable shader source. This tool:
//!   1. parses and validates it with a pinned Naga,
//!   2. generates particle.metal for the macOS Metal backend,
//!   3. records the source hash + toolchain version next to the artifact.
//!
//! `shadergen` regenerates and writes the artifacts; `shadergen --check`
//! regenerates in memory and fails on any drift (CI mode).

use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::ExitCode;

const WGSL_REL: &str = "shared/backgrounds/engines/particle-v1/particle.wgsl";
const GEN_DIR_REL: &str = "shared/backgrounds/engines/particle-v1/generated";
/// SPM bundles resources only from inside Sources/, so the artifact is
/// mirrored here for the macOS app (canonical copy is GEN_DIR_REL).
const SWIFT_RES_REL: &str = "Sources/ParticleWall/Resources/generated";
const MSL_NAME: &str = "particle.metal";
const META_NAME: &str = "particle.metal.meta";
const NAGA_VERSION: &str = "25.0.1";
const MSL_LANG_VERSION: (u8, u8) = (2, 0);

/// All engine entry points; each needs the binding map applied.
const ENTRY_POINTS: [&str; 7] = [
    "vsMain",
    "fsMain",
    "flowUpdate",
    "graphPositionUpdate",
    "graphConnectionUpdate",
    "vsLine",
    "fsLine",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Metal has a flat binding namespace (no bind groups). This maps every
/// (group, binding) of the WGSL to a unique Metal [[buffer(n)]] slot. Buffers
/// shared between read-only and read-write views (flow particles/history,
/// graph edges, uniforms) intentionally map to the SAME slot: Metal binds one
/// buffer per slot and the access mode lives in the entry point signature.
/// The Swift renderer binds exactly these slots.
fn flat_binding_map() -> naga::back::msl::BindingMap {
    use naga::back::msl::{BindTarget, BindingMap};
    use naga::ResourceBinding;
    let mut map = BindingMap::default();
    let mut slot = |group: u32, binding: u32, slot: u8| {
        map.insert(
            ResourceBinding { group, binding },
            BindTarget { buffer: Some(slot), texture: None, sampler: None, mutable: false },
        );
    };
    // Group 0: uniforms + read-only storage (render + graph position compute).
    slot(0, 0, 0); // uniforms
    slot(0, 1, 1); // flowParticles / primes (read)
    slot(0, 2, 2); // flowHistory (read)
    slot(0, 3, 3); // graphEdges (read, line pipeline)
    // Group 1: flowUpdate read-write views of the same buffers.
    slot(1, 0, 1); // flowParticles (rw)
    slot(1, 1, 2); // flowHistory (rw)
    slot(1, 2, 4); // flow step params (uniform)
    // Group 2: graph compute.
    slot(2, 0, 0); // gu == uniforms
    slot(2, 1, 5); // graphPositions (rw)
    slot(2, 2, 3); // graphEdges (rw)
    map
}

/// Validates the WGSL and returns (msl_source, wgsl_sha256).
fn generate(wgsl: &str) -> Result<(String, String), String> {
    let module = naga::front::wgsl::parse_str(wgsl)
        .map_err(|e| format!("WGSL parse error:\n{e}"))?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    );
    let info = validator
        .validate(&module)
        .map_err(|e| format!("WGSL validation error:\n{e}"))?;

    let options = naga::back::msl::Options {
        lang_version: MSL_LANG_VERSION,
        // Assign concrete [[buffer(n)]] slots; Metal compilation in Swift
        // expects fixed indices, not naga's patchable fake bindings.
        fake_missing_bindings: false,
        // Match the original Swift shader: no bounds-check plumbing and no
        // _mslBufferSizes side buffer.
        bounds_check_policies: naga::proc::BoundsCheckPolicies {
            index: naga::proc::BoundsCheckPolicy::Unchecked,
            buffer: naga::proc::BoundsCheckPolicy::Unchecked,
            image_load: naga::proc::BoundsCheckPolicy::Unchecked,
            ..naga::proc::BoundsCheckPolicies::default()
        },
        // Naga 25 carries the binding map per entry point; apply the same
        // flat map to every entry point of the engine.
        per_entry_point_map: ENTRY_POINTS
            .iter()
            .map(|ep| {
                (
                    (*ep).to_string(),
                    naga::back::msl::EntryPointResources {
                        resources: flat_binding_map(),
                        push_constant_buffer: None,
                        sizes_buffer: Some(6),
                    },
                )
            })
            .collect(),
        ..naga::back::msl::Options::default()
    };
    let pipeline_options = naga::back::msl::PipelineOptions::default();
    let mut msl = String::new();
    let mut writer = naga::back::msl::Writer::new(&mut msl);
    let translation = writer
        .write(&module, &info, &options, &pipeline_options)
        .map_err(|e| format!("MSL generation error: {e}"))?;
    // Naga skips entry points it cannot resolve and records the reason in
    // translation info instead of failing: surface every one of them.
    for ep in &translation.entry_point_names {
        if let Err(e) = ep {
            return Err(format!("MSL entry point skipped: {e}"));
        }
    }
    let emitted = translation
        .entry_point_names
        .iter()
        .filter(|ep| ep.is_ok())
        .count();
    if emitted != ENTRY_POINTS.len() {
        return Err(format!(
            "MSL emitted {emitted} of {} entry points",
            ENTRY_POINTS.len()
        ));
    }

    Ok((msl, sha256_hex(wgsl.as_bytes())))
}

fn meta_json(source_sha: &str) -> String {
    serde_json::json!({
        "source": "particle.wgsl",
        "sourceSha256": source_sha,
        "nagaVersion": NAGA_VERSION,
        "language": "metal",
        "langVersion": format!("{}.{}", MSL_LANG_VERSION.0, MSL_LANG_VERSION.1),
    })
    .to_string()
}

fn run(check: bool) -> Result<(), String> {
    let root = repo_root();
    let wgsl_path = root.join(WGSL_REL);
    let gen_dir = root.join(GEN_DIR_REL);
    let msl_path = gen_dir.join(MSL_NAME);
    let meta_path = gen_dir.join(META_NAME);

    let wgsl = std::fs::read_to_string(&wgsl_path)
        .map_err(|e| format!("cannot read {}: {e}", wgsl_path.display()))?;
    let (msl, source_sha) = generate(&wgsl)?;

    if check {
        let committed = std::fs::read_to_string(&msl_path)
            .map_err(|_| "drift: generated/particle.metal is missing; run `xtask shadergen`".to_string())?;
        if committed != msl {
            return Err(format!(
                "drift: {MSL_NAME} does not match {WGSL_REL}; run `xtask shadergen` and commit"
            ));
        }
        let committed_meta = std::fs::read_to_string(&meta_path)
            .map_err(|_| "drift: particle.metal.meta is missing".to_string())?;
        if committed_meta.trim() != meta_json(&source_sha) {
            return Err("drift: particle.metal.meta does not match the current WGSL".into());
        }
        let mirror = root.join(SWIFT_RES_REL).join(MSL_NAME);
        let mirrored = std::fs::read_to_string(&mirror).unwrap_or_default();
        if mirrored != msl {
            return Err(format!(
                "drift: {MSL_NAME} mirror at {} is stale; run `xtask shadergen`",
                mirror.display()
            ));
        }
        println!("shadergen --check OK (sha256 {source_sha})");
        return Ok(());
    }

    std::fs::create_dir_all(&gen_dir)
        .map_err(|e| format!("cannot create {}: {e}", gen_dir.display()))?;
    std::fs::write(&msl_path, &msl).map_err(|e| format!("write {}: {e}", msl_path.display()))?;
    std::fs::write(&meta_path, format!("{}\n", meta_json(&source_sha)))
        .map_err(|e| format!("write {}: {e}", meta_path.display()))?;
    let swift_dir = root.join(SWIFT_RES_REL);
    std::fs::create_dir_all(&swift_dir)
        .map_err(|e| format!("cannot create {}: {e}", swift_dir.display()))?;
    std::fs::write(swift_dir.join(MSL_NAME), &msl)
        .map_err(|e| format!("write mirror: {e}"))?;
    println!(
        "generated {} (+ {}) and {} (sha256 {source_sha}, {} bytes of MSL)",
        msl_path.display(),
        SWIFT_RES_REL,
        meta_path.display(),
        msl.len()
    );
    Ok(())
}

fn usage() -> ! {
    eprintln!("usage: xtask shadergen [--check]");
    std::process::exit(2);
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("shadergen") => {
            let check = match args.next().as_deref() {
                None => false,
                Some("--check") => true,
                Some(_) => usage(),
            };
            match run(check) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("xtask: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => usage(),
    }
}
