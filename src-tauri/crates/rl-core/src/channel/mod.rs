//! Public GitHub release metadata and bounded, verified downloads.
pub use crate::dto::Release;
use crate::{
    job::{Reporter, check_cancel, report},
    manifest, relpath,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::atomic::AtomicBool,
    time::Duration,
};
const LAUNCHER_REPO: &str = "stormie-labs/RayLink-Launcher";
const MAX_JSON: u64 = 4 * 1024 * 1024;
const MAX_DOWNLOAD: u64 = 2 * 1024 * 1024 * 1024;

pub fn valid_repo(repo: &str) -> bool {
    let Some((owner, name)) = repo.split_once('/') else {
        return false;
    };
    !owner.is_empty()
        && owner.len() <= 39
        && !owner.starts_with('-')
        && !owner.ends_with('-')
        && owner
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.')
        && owner != "."
        && owner != ".."
        && !owner.contains("..")
        && !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}

fn github_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "Invalid GitHub URL")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || url.fragment().is_some()
        || !matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "api.github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
                    | "github-releases.githubusercontent.com"
            )
        )
    {
        return Err("Only HTTPS GitHub release URLs are allowed".into());
    }
    Ok(url)
}

/// Metadata must name this repository/tag/asset. CDN URLs are allowed only as
/// HTTPS redirects from this canonical GitHub release URL.
fn release_url(value: &str, repo: &str, tag: &str, asset: Option<&str>) -> Result<(), String> {
    let actual = github_url(value)?;
    let mut expected = reqwest::Url::parse(&format!("https://github.com/{repo}/releases/"))
        .map_err(|_| "Invalid release repository")?;
    {
        let mut parts = expected
            .path_segments_mut()
            .map_err(|_| "Invalid release URL")?;
        parts
            .pop_if_empty()
            .push(if asset.is_some() { "download" } else { "tag" })
            .push(tag);
        if let Some(name) = asset {
            parts.push(name);
        }
    }
    let left: Vec<_> = actual.path().split('/').collect();
    let right: Vec<_> = expected.path().split('/').collect();
    if actual.host_str() != Some("github.com")
        || actual.query().is_some()
        || left.len() != right.len()
        || left.iter().zip(right).enumerate().any(|(index, (a, b))| {
            if index <= 2 {
                !a.eq_ignore_ascii_case(b)
            } else {
                *a != b
            }
        })
    {
        return Err("URL does not belong to the requested GitHub release".into());
    }
    Ok(())
}

fn client() -> Result<reqwest::blocking::Client, String> {
    // Blocking timeout bounds each connect/read/write operation (see reqwest's
    // blocking::Response::read). Async read_timeout requires a Tokio reactor
    // during body polling and must not be injected into the blocking builder.
    reqwest::blocking::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || github_url(attempt.url().as_str()).is_err() {
                attempt.error("Disallowed GitHub release redirect")
            } else {
                attempt.follow()
            }
        }))
        .user_agent("RayLink-Launcher/0.1")
        .build()
        .map_err(|_| "Cannot initialize HTTPS client".into())
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    #[serde(default)]
    published_at: String,
    html_url: String,
    assets: Vec<ApiAsset>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}
#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

fn digest(value: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let hash = value
        .strip_prefix("sha256:")
        .ok_or("Unsupported release asset digest")?;
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid SHA-256 release asset digest".into());
    }
    Ok(Some(hash.to_ascii_lowercase()))
}

fn build_asset(name: &str) -> bool {
    name.starts_with("anomaly-coop-") && name.ends_with(".zip") && !name.contains(['/', '\\'])
}

fn parse_release(repo: &str, json: &[u8]) -> Result<Release, String> {
    if !valid_repo(repo) {
        return Err("Invalid GitHub owner/repository".into());
    }
    let api: ApiRelease =
        serde_json::from_slice(json).map_err(|_| "Invalid UTF-8 GitHub release response")?;
    if api.draft || api.prerelease {
        return Err("A stable public release is required".into());
    }
    relpath::validate_tag(&api.tag_name)?;
    release_url(&api.html_url, repo, &api.tag_name, None)?;
    let executable = repo.eq_ignore_ascii_case(LAUNCHER_REPO);
    let mut candidates = api.assets.into_iter().filter(|a| {
        if executable {
            a.name.eq_ignore_ascii_case("RayLinkSetup.exe")
        } else {
            build_asset(&a.name)
        }
    });
    let asset = candidates
        .next()
        .ok_or("Release does not contain the required build archive or RayLinkSetup.exe")?;
    if candidates.next().is_some() {
        return Err("Release contains ambiguous matching assets".into());
    }
    relpath::validate(&asset.name)?;
    release_url(
        &asset.browser_download_url,
        repo,
        &api.tag_name,
        Some(&asset.name),
    )?;
    if asset.size == 0 || asset.size > MAX_DOWNLOAD {
        return Err("Release asset size is outside allowed limits".into());
    }
    let sha256 = digest(asset.digest.as_deref())?;
    if executable && sha256.is_none() {
        return Err("Launcher update requires a GitHub SHA-256 digest".into());
    }
    Ok(Release {
        tag: api.tag_name.clone(),
        name: api.name.unwrap_or(api.tag_name),
        body: api.body.unwrap_or_default(),
        published_at: api.published_at,
        page_url: api.html_url,
        asset_name: asset.name,
        asset_url: asset.browser_download_url,
        asset_size: asset.size,
        sha256,
    })
}

pub fn latest(repo: &str) -> Result<Release, String> {
    if !valid_repo(repo) {
        return Err("Invalid GitHub owner/repository".into());
    }
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let response = client()?
        .get(url)
        .timeout(Duration::from_secs(25))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .map_err(|_| "GitHub release request failed")?;
    if !response.status().is_success() {
        return Err(format!(
            "GitHub release request returned HTTP {}",
            response.status().as_u16()
        ));
    }
    read_release(repo, response)
}

fn read_release(repo: &str, response: impl Read) -> Result<Release, String> {
    let mut bytes = Vec::new();
    let mut input = response.take(MAX_JSON + 1);
    let mut buffer = [0u8; 16 * 1024];
    let started = std::time::Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(30) {
            return Err("GitHub metadata read timed out".into());
        }
        let count = input
            .read(&mut buffer)
            .map_err(|_| "Cannot read GitHub release response")?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() as u64 > MAX_JSON {
            return Err("GitHub release response exceeds size limit".into());
        }
    }
    parse_release(repo, &bytes)
}

pub fn download(
    release: &Release,
    destination: &Path,
    cancel: &AtomicBool,
    progress: &Reporter,
) -> Result<(), String> {
    check_cancel(cancel)?;
    validate_download(release)?;
    let url = github_url(&release.asset_url)?;
    let response = client()?
        .get(url)
        .header("Accept", "application/octet-stream")
        .send()
        .map_err(|_| "GitHub asset download failed")?;
    check_cancel(cancel)?;
    if response.status().as_u16() != 200 {
        return Err(format!(
            "GitHub asset request returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let length = response.content_length();
    receive(release, destination, response, length, cancel, progress)
}

fn validate_download(release: &Release) -> Result<(), String> {
    relpath::validate_tag(&release.tag)?;
    relpath::validate(&release.asset_name)?;
    if !build_asset(&release.asset_name)
        && !release.asset_name.eq_ignore_ascii_case("RayLinkSetup.exe")
    {
        return Err("Unsupported release asset name".into());
    }
    if release.asset_size == 0 || release.asset_size > MAX_DOWNLOAD {
        return Err("Release asset size exceeds limit".into());
    }
    if let Some(hash) = &release.sha256 {
        if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Invalid expected SHA-256 digest".into());
        }
    } else if release.asset_name.eq_ignore_ascii_case("RayLinkSetup.exe") {
        return Err("Executable download requires a GitHub SHA-256 digest".into());
    }
    let url = github_url(&release.asset_url)?;
    let segments: Vec<_> = url.path().split('/').collect();
    if segments.len() < 3 {
        return Err("Invalid GitHub release asset URL".into());
    }
    let repo = format!("{}/{}", segments[1], segments[2]);
    if !valid_repo(&repo)
        || (release.asset_name.eq_ignore_ascii_case("RayLinkSetup.exe")
            && !repo.eq_ignore_ascii_case(LAUNCHER_REPO))
    {
        return Err("Invalid GitHub release repository".into());
    }
    release_url(&release.page_url, &repo, &release.tag, None)?;
    release_url(
        &release.asset_url,
        &repo,
        &release.tag,
        Some(&release.asset_name),
    )?;
    Ok(())
}

fn receive(
    release: &Release,
    destination: &Path,
    mut input: impl Read,
    length: Option<u64>,
    cancel: &AtomicBool,
    progress: &Reporter,
) -> Result<(), String> {
    check_cancel(cancel)?;
    validate_download(release)?;
    if length.is_some_and(|size| size != release.asset_size) {
        return Err("Asset Content-Length does not match the release size".into());
    }
    relpath::no_reparse_ancestors(destination)?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|_| "Cannot create download directory")?;
    relpath::no_reparse_ancestors(destination)?;
    let name = destination
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid download destination")?;
    let part = parent.join(format!("{name}.{}.part", manifest::unique_id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)
        .map_err(|_| "Cannot create partial download")?;
    let result = (|| {
        let mut hash = Sha256::new();
        let mut downloaded = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        let started = std::time::Instant::now();
        report(
            progress,
            "download",
            "Downloading release asset",
            0,
            release.asset_size,
            true,
        );
        loop {
            check_cancel(cancel)?;
            if started.elapsed() > Duration::from_secs(30 * 60) {
                return Err("Asset download exceeded time limit".into());
            }
            let count = input
                .read(&mut buffer)
                .map_err(|_| "Cannot read release asset data")?;
            check_cancel(cancel)?;
            if count == 0 {
                break;
            }
            downloaded += count as u64;
            if downloaded > release.asset_size {
                return Err("Downloaded asset exceeds expected size".into());
            }
            file.write_all(&buffer[..count])
                .map_err(|_| "Cannot write release asset data")?;
            hash.update(&buffer[..count]);
            report(
                progress,
                "download",
                "Downloading release asset",
                downloaded,
                release.asset_size,
                true,
            );
        }
        if downloaded != release.asset_size {
            return Err("Downloaded asset is truncated".into());
        }
        let actual = manifest::hex(&hash.finalize());
        if release
            .sha256
            .as_ref()
            .is_some_and(|expected| !actual.eq_ignore_ascii_case(expected))
        {
            return Err("Downloaded asset SHA-256 does not match GitHub".into());
        }
        file.sync_all()
            .map_err(|_| "Cannot flush downloaded asset")?;
        check_cancel(cancel)?;
        Ok(actual)
    })();
    drop(file);
    let result = result.and_then(|actual| {
        check_cancel(cancel)?;
        relpath::no_reparse_ancestors(destination)?;
        // Publish only validated bytes; preserve existing cache unless it is
        // byte-identical to this fresh download (also for archives without a digest).
        if destination.exists() {
            return cached_matches(release, destination, &actual, cancel);
        }
        fs::hard_link(&part, destination).map_err(|_| "Cannot publish downloaded asset".to_string())
    });
    if relpath::no_reparse_ancestors(&part).is_ok() {
        let _ = fs::remove_file(&part);
    }
    result
}

fn cached_matches(
    release: &Release,
    destination: &Path,
    expected: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let meta = fs::symlink_metadata(destination).map_err(|_| "Cannot inspect cached asset")?;
    if !meta.is_file() || relpath::is_reparse(&meta) || meta.len() != release.asset_size {
        return Err("Existing cached asset does not match release".into());
    }
    let mut file = fs::File::open(destination).map_err(|_| "Cannot read cached asset")?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        check_cancel(cancel)?;
        let count = file
            .read(&mut buffer)
            .map_err(|_| "Cannot hash cached asset")?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > release.asset_size {
            return Err("Existing cache changed during verification".into());
        }
        hash.update(&buffer[..count]);
    }
    if size != release.asset_size {
        return Err("Existing cache changed during verification".into());
    }
    if !manifest::hex(&hash.finalize()).eq_ignore_ascii_case(expected) {
        return Err("Existing cached asset SHA-256 mismatch".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_owner_repository_and_https_allowlist() {
        for good in ["stormie-labs/RayLink-Coop", "some.owner/repo_name"] {
            assert!(valid_repo(good));
        }
        for bad in [
            "",
            "owner/repo/extra",
            "../repo",
            "-owner/repo",
            "owner/..",
            "own er/repo",
            "https://github.com/a/b",
            "owner/repo?x",
            "owner/.git",
        ] {
            assert!(!valid_repo(bad), "{bad}");
        }
        for good in [
            "https://github.com/a/b/releases/download/v/a.zip",
            "https://release-assets.githubusercontent.com/a?signature=x",
        ] {
            assert!(github_url(good).is_ok());
        }
        for bad in [
            "http://github.com/a",
            "https://github.com.evil.test/a",
            "https://evil@github.com/a",
            "https://github.com:444/a",
            "https://127.0.0.1/a",
            "https://raw.githubusercontent.com/a",
            "https://github.com/a#frag",
        ] {
            assert!(github_url(bad).is_err(), "{bad}");
        }
    }

    fn json(asset: &str, digest: &str) -> Vec<u8> {
        let repo = if asset == "RayLinkSetup.exe" {
            LAUNCHER_REPO
        } else {
            "stormie-labs/RayLink-Coop"
        };
        format!(r#"{{"tag_name":"b26","name":"Сборка 26","body":"Исправления ёжика","published_at":"2026-10-28T00:00:00Z","html_url":"https://github.com/{repo}/releases/tag/b26","assets":[{{"name":"{asset}","browser_download_url":"https://github.com/{repo}/releases/download/b26/{asset}","size":6,"digest":{digest}}}]}}"#).into_bytes()
    }

    #[test]
    fn parses_utf8_and_selects_only_the_build_asset() {
        let release = parse_release(
            "stormie-labs/RayLink-Coop",
            &json("anomaly-coop-26.zip", "null"),
        )
        .unwrap();
        assert_eq!(release.name, "Сборка 26");
        assert_eq!(release.body, "Исправления ёжика");
        assert_eq!(release.asset_name, "anomaly-coop-26.zip");
        assert!(parse_release("stormie-labs/RayLink-Coop", &json("other.zip", "null")).is_err());
        assert!(
            parse_release(
                "stormie-labs/RayLink-Coop",
                &json("../anomaly-coop-26.zip", "null")
            )
            .is_err()
        );
    }

    #[test]
    fn executable_update_requires_github_sha256() {
        assert!(parse_release(LAUNCHER_REPO, &json("RayLinkSetup.exe", "null")).is_err());
        let digest = format!("\"sha256:{}\"", "a".repeat(64));
        let release = parse_release(LAUNCHER_REPO, &json("RayLinkSetup.exe", &digest)).unwrap();
        assert_eq!(release.sha256, Some("a".repeat(64)));
    }

    #[test]
    fn metadata_urls_must_belong_to_the_requested_release() {
        let original = json("anomaly-coop-26.zip", "null");
        for replacement in [
            "https://github.com/attacker/project/releases/download/b26/anomaly-coop-26.zip",
            "https://github.com/stormie-labs/RayLink-Coop/releases/download/old/anomaly-coop-26.zip",
            "https://github.com/stormie-labs/RayLink-Coop/releases/download/b26/different.zip",
            "https://release-assets.githubusercontent.com/arbitrary",
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
            value["assets"][0]["browser_download_url"] = replacement.into();
            assert!(
                parse_release(
                    "stormie-labs/RayLink-Coop",
                    &serde_json::to_vec(&value).unwrap()
                )
                .is_err(),
                "accepted {replacement}"
            );
        }
        let mut release = tiny_release();
        release.asset_url = "https://github.com/stormie-labs/RayLink-Coop/releases/download/old/anomaly-coop-26.zip".into();
        assert!(validate_download(&release).is_err());
    }

    #[test]
    fn selected_asset_uses_its_own_digest_and_rejects_ambiguity() {
        let mut value: serde_json::Value = serde_json::from_slice(&json(
            "anomaly-coop-26.zip",
            &format!("\"sha256:{}\"", "b".repeat(64)),
        ))
        .unwrap();
        let selected = value["assets"][0].clone();
        let mut unrelated = selected.clone();
        unrelated["name"] = "source.zip".into();
        unrelated["digest"] = format!("sha256:{}", "a".repeat(64)).into();
        value["assets"] = serde_json::json!([unrelated, selected.clone()]);
        let parse = |v: &serde_json::Value| {
            parse_release("stormie-labs/RayLink-Coop", &serde_json::to_vec(v).unwrap())
        };
        assert_eq!(parse(&value).unwrap().sha256, Some("b".repeat(64)));
        value["assets"][1]["digest"] = "sha256:invalid".into();
        assert!(parse(&value).is_err());
        value["assets"] = serde_json::json!([selected.clone(), selected]);
        assert!(parse(&value).is_err());
    }

    fn temp() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|p| p.file_name().is_some_and(|n| n == "raylink-launcher"))
            .unwrap()
            .join("temp");
        fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(root).unwrap()
    }

    fn tiny_release() -> Release {
        let mut release = parse_release(
            "stormie-labs/RayLink-Coop",
            &json("anomaly-coop-26.zip", "null"),
        )
        .unwrap();
        release.asset_size = 3;
        release.sha256 =
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into());
        release
    }

    #[test]
    fn download_requires_complete_length_and_digest_and_keeps_existing_cache() {
        let t = temp();
        let dest = t.path().join("build.zip");
        let release = tiny_release();
        for (body, length) in [
            (b"ab".as_slice(), Some(3)),
            (b"abcd".as_slice(), None),
            (b"bad".as_slice(), Some(3)),
            (b"abc".as_slice(), Some(4)),
        ] {
            assert!(
                receive(
                    &release,
                    &dest,
                    body,
                    length,
                    &AtomicBool::new(false),
                    &|_| {}
                )
                .is_err()
            );
            assert!(!dest.exists());
            assert_eq!(fs::read_dir(t.path()).unwrap().count(), 0);
        }
        receive(
            &release,
            &dest,
            b"abc".as_slice(),
            Some(3),
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"abc");
        assert!(
            receive(
                &release,
                &dest,
                b"bad".as_slice(),
                None,
                &AtomicBool::new(false),
                &|_| {}
            )
            .is_err()
        );
        assert_eq!(fs::read(&dest).unwrap(), b"abc");
        assert_eq!(fs::read_dir(t.path()).unwrap().count(), 1);
    }

    #[test]
    fn archive_without_remote_digest_reuses_only_identical_downloaded_bytes() {
        let t = temp();
        let dest = t.path().join("build.zip");
        let mut release = tiny_release();
        release.sha256 = None;
        let cancel = AtomicBool::new(false);
        receive(&release, &dest, b"abc".as_slice(), None, &cancel, &|_| {}).unwrap();
        receive(&release, &dest, b"abc".as_slice(), None, &cancel, &|_| {}).unwrap();
        assert!(receive(&release, &dest, b"xyz".as_slice(), None, &cancel, &|_| {}).is_err());
        assert_eq!(fs::read(dest).unwrap(), b"abc");
    }

    #[test]
    fn cancellation_during_download_removes_only_its_partial_file() {
        let t = temp();
        let dest = t.path().join("build.zip");
        let release = tiny_release();
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        fs::write(t.path().join("unowned.part"), b"keep").unwrap();
        assert!(
            receive(
                &release,
                &dest,
                b"abc".as_slice(),
                None,
                &cancel,
                &move |_| {
                    signal.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            )
            .is_err()
        );
        assert!(!dest.exists());
        assert_eq!(fs::read_dir(t.path()).unwrap().count(), 1);
        assert_eq!(fs::read(t.path().join("unowned.part")).unwrap(), b"keep");
    }

    #[test]
    fn utf8_metadata_over_real_http_fixture() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = json("anomaly-coop-26.zip", "null");
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 8192];
            assert!(socket.read(&mut request).unwrap() > 0);
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            socket.write_all(&body).unwrap();
        });
        let response = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap()
            .get(format!("http://{address}/release"))
            .send()
            .unwrap();
        let release = read_release("stormie-labs/RayLink-Coop", response).unwrap();
        assert_eq!(release.name, "Сборка 26");
        assert_eq!(release.body, "Исправления ёжика");
        server.join().unwrap();
    }

    #[test]
    #[ignore = "Explicit live GitHub download smoke; writes only project temp"]
    fn live_public_build_download_and_install() {
        let t = temp();
        let base = t.path().to_path_buf();
        if std::env::var_os("RAYLINK_DIAG_KEEP_STAGE").is_some() { let _ = t.keep(); }
        let release = latest("stormie-labs/RayLink-Coop").unwrap();
        let dest = base.as_path().join("build.zip");
        let cancel = AtomicBool::new(false);
        download(&release, &dest, &cancel, &|_| {}).unwrap();
        let build = crate::install::install_archive(
            &dest,
            &base.as_path().join("builds"),
            &release.tag,
            &cancel,
            &|_| {},
        )
        .unwrap();
        crate::install::verify_build(&build).unwrap();
        eprintln!(
            "Verified public release {} ({} bytes), remote SHA256 present: {}",
            release.tag,
            release.asset_size,
            release.sha256.is_some()
        );
    }
}
