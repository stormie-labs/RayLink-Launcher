//! Small native smoke-test interface to the launcher core.
use std::{env, ffi::OsString, fs, path::Path, process::ExitCode, sync::atomic::AtomicBool};
use rl_core::{channel, dto::Settings, install, launch, relpath, settings, stand};

const HELP: &str = "Usage: rl-tool <channel <owner/repo> | install <zip> <home> <tag> | verify <buildDir> | prepare <settingsJson> <home> | launch-args <nick>>";

#[derive(Debug, PartialEq)]
enum Command {
    Channel(String), Install(String, String, String), Verify(String),
    Prepare(String, String), LaunchArgs(String),
}

fn parse(args: Vec<String>) -> Result<Command, ()> {
    match args.as_slice() {
        [cmd, repo] if cmd == "channel" => Ok(Command::Channel(repo.clone())),
        [cmd, zip, home, tag] if cmd == "install" => Ok(Command::Install(zip.clone(), home.clone(), tag.clone())),
        [cmd, build] if cmd == "verify" => Ok(Command::Verify(build.clone())),
        [cmd, file, home] if cmd == "prepare" => Ok(Command::Prepare(file.clone(), home.clone())),
        [cmd, nick] if cmd == "launch-args" => Ok(Command::LaunchArgs(nick.clone())),
        _ => Err(()),
    }
}

fn run(cmd: Command) -> Result<serde_json::Value, String> {
    let cancel = AtomicBool::new(false);
    // Do not emit arbitrary messages: they can include paths or user-supplied data.
    let progress = |p: rl_core::dto::Progress| eprintln!("{}: {}/{}", p.phase, p.current, p.total);
    match cmd {
        Command::Channel(repo) => serde_json::to_value(channel::latest(&repo)?).map_err(|_| "Cannot serialize release".into()),
        Command::Install(zip, home, tag) => {
            let path = install::install_archive(Path::new(&zip), &Path::new(&home).join("builds"), &tag, &cancel, &progress)?;
            serde_json::to_value(path).map_err(|_| "Cannot serialize build path".into())
        }
        Command::Verify(build) => { install::verify_build(Path::new(&build))?; Ok(serde_json::Value::Bool(true)) }
        Command::Prepare(file, home) => {
            let bytes = fs::read(file).map_err(|_| "Cannot read settings file")?;
            let config: Settings = serde_json::from_slice(&bytes).map_err(|_| "Invalid settings JSON")?;
            settings::validate(&config)?;
            relpath::validate_tag(&config.installed)?;
            let build = rl_core::selection::build_directory(Path::new(&home), &config);
            serde_json::to_value(stand::materialize(&config, &build, Path::new(&home), &cancel, &progress)?).map_err(|_| "Cannot serialize stand summary".into())
        }
        Command::LaunchArgs(nick) => {
            let args: Vec<String> = launch::arguments(&nick).into_iter().map(|s: OsString| s.to_string_lossy().into_owned()).collect();
            serde_json::to_value(args).map_err(|_| "Cannot serialize launch arguments".into())
        }
    }
}

fn main() -> ExitCode {
    let args: Result<Vec<_>, _> = env::args_os().skip(1).map(OsString::into_string).collect();
    let command = match args.ok().and_then(|args| parse(args).ok()) {
        Some(command) => command,
        None => { eprintln!("{HELP}"); return ExitCode::from(2); }
    };
    match run(command) {
        Ok(value) => { println!("{value}"); ExitCode::SUCCESS }
        Err(err) => { eprintln!("{}", rl_core::redact::redact(&err)); ExitCode::from(1) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_accepts_exact_arities() {
        assert_eq!(parse(vec!["verify".into(), "build".into()]), Ok(Command::Verify("build".into())));
        assert!(parse(vec![]).is_err());
        assert!(parse(vec!["verify".into()]).is_err());
        assert!(parse(vec!["verify".into(), "build".into(), "extra".into()]).is_err());
        assert!(parse(vec!["--help".into()]).is_err());
    }
}
