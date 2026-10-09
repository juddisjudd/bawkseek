use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO_URL: &str = "https://github.com/juddisjudd/bawkseek";
pub const RELEASES_URL: &str = "https://github.com/juddisjudd/bawkseek/releases";
pub const ISSUES_URL: &str = "https://github.com/juddisjudd/bawkseek/issues";
const LATEST_API: &str = "https://api.github.com/repos/juddisjudd/bawkseek/releases/latest";
const ZIP_SUFFIX: &str = "-windows-x64.zip";

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// A published release newer than this build, with the download that matches this platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub page: String,
    pub size: u64,
    zip_url: String,
    sum_url: String,
}

pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.split(['-', '+']).next()?.parse().ok()?;
    Some((major, minor, patch))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent(format!("bawkseek/{VERSION}"))
        .build()
        .into()
}

fn pick(release: ApiRelease) -> Result<Release, String> {
    let zip = release
        .assets
        .iter()
        .find(|asset| asset.name.ends_with(ZIP_SUFFIX))
        .ok_or("the newest release has no Windows download yet")?;
    let sum_name = format!("{}.sha256", zip.name);
    let sum = release
        .assets
        .iter()
        .find(|asset| asset.name == sum_name)
        .ok_or("the newest release has no checksum to verify it with")?;
    Ok(Release {
        version: release.tag_name.trim_start_matches('v').to_string(),
        page: release.html_url,
        size: zip.size,
        zip_url: zip.browser_download_url.clone(),
        sum_url: sum.browser_download_url.clone(),
    })
}

/// The newest release on GitHub, when it is newer than this build.
pub fn check() -> Result<Option<Release>, String> {
    let release: ApiRelease = agent()
        .get(LATEST_API)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|err| format!("could not reach GitHub: {err}"))?
        .body_mut()
        .read_json()
        .map_err(|err| format!("GitHub sent something unexpected: {err}"))?;
    if !is_newer(&release.tag_name, VERSION) {
        return Ok(None);
    }
    pick(release).map(Some)
}

/// The exe to replace, unless this copy was built from source, which updates through git instead.
pub fn install_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let from_source = exe
        .parent()
        .and_then(|dir| Some((dir.file_name()?, dir.parent()?.file_name()?)))
        .is_some_and(|(profile, target)| {
            target == "target" && (profile == "debug" || profile == "release")
        });
    if from_source {
        return Err("this copy was built from source, so update it with git and cargo".into());
    }
    Ok(exe)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sibling(exe: &Path, tag: &str) -> PathBuf {
    let stem = exe.file_stem().unwrap_or_default().to_string_lossy();
    exe.with_file_name(format!("{stem}.{tag}.exe"))
}

/// Downloads and checks the release, then puts the new exe where the running one was. It runs after a restart.
pub fn install(release: &Release, progress: impl Fn(u64)) -> Result<PathBuf, String> {
    let exe = install_path()?;
    let agent = agent();
    let sums = agent
        .get(&release.sum_url)
        .call()
        .map_err(|err| format!("could not download the checksum: {err}"))?
        .body_mut()
        .read_to_string()
        .map_err(|err| err.to_string())?;
    let expected = sums
        .split_whitespace()
        .next()
        .ok_or("the checksum file is empty")?
        .to_lowercase();

    let zip_path = std::env::temp_dir().join(format!("bawkseek-{}.zip", release.version));
    let mut response = agent
        .get(&release.zip_url)
        .call()
        .map_err(|err| format!("could not download the update: {err}"))?;
    let mut reader = response.body_mut().as_reader();
    let mut file = File::create(&zip_path).map_err(|err| err.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let read = reader
            .read(&mut buf)
            .map_err(|err| format!("the download broke off: {err}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        file.write_all(&buf[..read])
            .map_err(|err| err.to_string())?;
        done += read as u64;
        progress(done);
    }
    drop(file);
    if hex(&hasher.finalize()) != expected {
        let _ = fs::remove_file(&zip_path);
        return Err("the download does not match its checksum, so it was not installed".into());
    }

    let fresh = sibling(&exe, "new");
    let result = extract_exe(&zip_path, &fresh).and_then(|()| swap(&exe, &fresh));
    let _ = fs::remove_file(&zip_path);
    let _ = fs::remove_file(&fresh);
    result.map(|()| exe)
}

fn extract_exe(zip_path: &Path, out: &Path) -> Result<(), String> {
    let file = File::open(zip_path).map_err(|err| err.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|err| err.to_string())?;
    let name = (0..archive.len())
        .filter_map(|ix| archive.name_for_index(ix).map(str::to_string))
        .find(|name| name.ends_with("bawkseek.exe"))
        .ok_or("the download has no bawkseek.exe in it")?;
    let mut entry = archive.by_name(&name).map_err(|err| err.to_string())?;
    let mut target = File::create(out).map_err(|err| {
        format!("cannot write next to bawkseek.exe ({err}). download the new version by hand.")
    })?;
    std::io::copy(&mut entry, &mut target).map_err(|err| err.to_string())?;
    Ok(())
}

/// Windows lets a running exe be renamed but not overwritten, so the old one steps aside first.
fn swap(exe: &Path, fresh: &Path) -> Result<(), String> {
    let old = sibling(exe, "old");
    let _ = fs::remove_file(&old);
    fs::rename(exe, &old).map_err(|err| format!("cannot replace bawkseek.exe: {err}"))?;
    if let Err(err) = fs::rename(fresh, exe) {
        let _ = fs::rename(&old, exe);
        return Err(format!("cannot replace bawkseek.exe: {err}"));
    }
    Ok(())
}

/// Starts the freshly installed exe; the caller quits this one.
pub fn relaunch(exe: &Path) -> Result<(), String> {
    Command::new(exe)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("could not start the new version: {err}"))
}

/// Removes the exe an earlier update set aside, waiting for that copy to finish quitting.
pub fn clean_up() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let old = sibling(&exe, "old");
    if !old.exists() {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..30 {
            if fs::remove_file(&old).is_ok() || !old.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> ApiAsset {
        ApiAsset {
            name: name.into(),
            browser_download_url: format!("https://example.com/{name}"),
            size: 10,
        }
    }

    #[test]
    fn compares_versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.10.0-beta.1"), Some((0, 10, 0)));
        assert_eq!(parse_version("nonsense"), None);
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("v0.10.0", "0.9.0"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn picks_the_windows_zip_and_its_checksum() {
        let release = ApiRelease {
            tag_name: "v0.2.0".into(),
            html_url: "https://example.com/v0.2.0".into(),
            assets: vec![
                asset("notes.txt"),
                asset("bawkseek-v0.2.0-windows-x64.zip.sha256"),
                asset("bawkseek-v0.2.0-windows-x64.zip"),
            ],
        };
        let picked = pick(release).unwrap();
        assert_eq!(picked.version, "0.2.0");
        assert!(picked.zip_url.ends_with(".zip"));
        assert!(picked.sum_url.ends_with(".zip.sha256"));
        let bare = ApiRelease {
            tag_name: "v0.2.0".into(),
            html_url: String::new(),
            assets: vec![asset("bawkseek-v0.2.0-windows-x64.zip")],
        };
        assert!(pick(bare).is_err());
    }

    #[test]
    #[ignore = "talks to GitHub"]
    fn checks_github() {
        assert_eq!(check(), Ok(None));
    }

    #[test]
    fn swaps_files_and_names_siblings() {
        let dir = std::env::temp_dir().join(format!("bawkseek-swap-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("bawkseek.exe");
        let fresh = sibling(&exe, "new");
        assert_eq!(fresh.file_name().unwrap(), "bawkseek.new.exe");
        fs::write(&exe, b"old").unwrap();
        fs::write(&fresh, b"new").unwrap();
        swap(&exe, &fresh).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new");
        assert_eq!(fs::read(sibling(&exe, "old")).unwrap(), b"old");
        fs::remove_dir_all(&dir).unwrap();
    }
}
