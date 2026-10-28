//! `rl-core` — all launcher logic without Tauri (SPEC §1.5): manifest, channel, stand, GAMMA, install, host probe,
//! launch args. Module declarations — T1; module bodies — their owners (PLAN §1).

pub mod dto; // T1: mirror of src/lib/api/contract.ts, pinned by contracts/fixtures
pub mod job; // T1: Progress, Cancel, Clock shared by T6–T8 (PLAN §2.4)

pub mod manifest; // T6
pub mod redact; // T6
pub mod relpath; // T6
pub mod secret; // T6

pub mod channel; // T6

pub mod gamma; // T7
pub mod stand; // T7

pub mod install; // T8

pub mod launch; // T9
pub mod probe; // T9
pub mod settings;
pub mod lifecycle;
pub mod selection;
