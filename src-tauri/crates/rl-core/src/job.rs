//! Progress and cancellation for blocking backend workers.
use std::sync::atomic::{AtomicBool, Ordering};
use crate::dto::Progress;
pub type Reporter<'a> = dyn Fn(Progress) + Send + Sync + 'a;
pub fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) { Err("Operation cancelled".into()) } else { Ok(()) }
}
pub fn report(progress: &Reporter, phase: &str, message: &str, current: u64, total: u64, cancellable: bool) {
    progress(Progress { phase: phase.into(), message: message.into(), current, total, cancellable });
}
