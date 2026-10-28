//! Game process creation. No shell command strings or connection mode are accepted.
use std::{path::Path, ffi::OsString};
pub fn arguments(nick: &str) -> Vec<OsString> {
    ["-fsltx", "fsgame_a.ltx", "-smap2048", "-coop_nick"].map(OsString::from).into_iter()
        .chain(std::iter::once(OsString::from(crate::settings::clean_nick(nick)))).collect()
}
pub fn game(root: &Path, nick: &str) -> Result<u32, String> {
    crate::settings::check_plain_ancestors(root)?;
    let exe = root.join("bin/AnomalyCoopDX11AVX.exe");
    if !exe.is_file() || !root.join("fsgame_a.ltx").is_file() { return Err("Prepare the game files before playing".into()); }
    if !crate::stand::running_processes(root)?.is_empty() { return Err("Close the running game before launching it again".into()); }
    let child = std::process::Command::new(&exe).args(arguments(nick)).current_dir(root).spawn().map_err(|err| format!("Cannot start the game: {err}"))?;
    Ok(child.id())
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn uses_legacy_working_directory_and_no_join_arguments() {
        assert_eq!(arguments("Alice"), ["-fsltx", "fsgame_a.ltx", "-smap2048", "-coop_nick", "Alice"].map(OsString::from));
    }
    #[test] fn nickname_cannot_inject_engine_options() {
        assert_eq!(arguments("Alice -coop_host").last(), Some(&OsString::from("Alice-coop_host")));
    }
    #[test] fn incomplete_stand_is_not_launched() { let dir = tempfile::tempdir().unwrap(); assert!(game(dir.path(), "Alice").is_err()); }
}
