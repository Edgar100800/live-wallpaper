//! Deterministic playback policy.
//!
//! Mirrors PlaybackPolicy.resolve in PowerManager.swift:4-19:
//! paused  = userPaused || powerSave || systemUnavailable || batteryPause
//! deep    = powerSave || systemUnavailable || batteryPause
//! frame is always preserved on deep sleep; snapshot persisted only when
//! the system becomes unavailable (lock/screen sleep), not on battery.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackPolicy {
    pub paused: bool,
    pub deep_sleep: bool,
    pub preserving_frame: bool,
    pub persist_snapshot: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PolicyInputs {
    pub user_paused: bool,
    pub power_save: bool,
    pub system_unavailable: bool,
    pub battery_pause: bool,
}

pub fn resolve(inputs: PolicyInputs) -> PlaybackPolicy {
    let deep =
        inputs.power_save || inputs.system_unavailable || inputs.battery_pause;
    PlaybackPolicy {
        paused: inputs.user_paused || deep,
        deep_sleep: deep,
        preserving_frame: true,
        persist_snapshot: inputs.system_unavailable,
    }
}
