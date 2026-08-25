//! Shared ParticleWall contracts implemented for Linux.
//!
//! Every function here mirrors Swift behavior in Sources/ParticleWall and is
//! validated against the neutral fixtures in shared/contracts/fixtures.

pub mod manifest;
pub mod navigation;
pub mod paths;
pub mod playback;

/// Fixture root relative to this crate (used by tests and tooling).
pub fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../shared/contracts/fixtures")
}
