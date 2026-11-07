//! Updating Shepherd from its GitHub releases (Collie's updater): the latest release from GitHub's API, its archive
//! for this machine downloaded, checked, put in place of the running app, then a relaunch. Checked at launch and every
//! few hours; the sidebar offers what it finds (Zeron's update strip), Settings → About checks by hand.
//!
//! On macOS the archive is `Shepherd-<V>-universal.zip`, and the app in it must be signed by the running app's team.
//! On Linux and Windows it is the tar.gz or zip with the bare binary, and its SHA-256 must carry CI's Ed25519
//! signature ([`UPDATE_KEY`], `scripts/sign-update.sh`); the binary is swapped in only where Shepherd may write it. A
//! release build that can't (a .deb or .rpm in `/usr/bin`, a copy in Program Files) only tells of a newer release, and
//! a click opens its page.
use anyhow::{Context as _, bail};
use futures::StreamExt;
use gpui::{App, AppContext as _, Global};
use sha2::{Digest as _, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

const LATEST: &str = "https://api.github.com/repos/getshepherd/shepherd/releases/latest";
/// What a release's page must start with before a click opens it.
const PAGES: &str = "https://github.com/getshepherd/shepherd/releases/";
const EVERY: Duration = Duration::from_secs(6 * 3600);
/// The Ed25519 public key whose private half (the `SHEPHERD_UPDATE_KEY` secret) signs the Linux and Windows archives.
const UPDATE_KEY: [u8; 32] = [
    0x06, 0xa1, 0xb4, 0x9b, 0xbe, 0x8b, 0x6f, 0x5c, 0x52, 0x12, 0x0a, 0xff, 0x23, 0x5f, 0xb7, 0xcd, 0x3f, 0xbe, 0x1f, 0xd8,
    0xce, 0x4f, 0x8e, 0x12, 0x14, 0x64, 0xee, 0x16, 0x0c, 0x1a, 0x1c, 0x09,
];

/// A release's version and its archive for this machine, with the archive's signature off macOS; when the build only
/// [`notifies`], its page and no size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub size: u64,
    pub sig: Option<String>,
}

/// Where the app is with its updates.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Updates {
    #[default]
    Unknown,
    Checking,
    UpToDate,
    Available(Release),
    Downloading { version: String, percent: u64 },
    Installing,
    /// Why the check or the update failed; with the release when it was the update, to try again.
    Failed(String, Option<Release>),
}

impl Global for Updates {}

impl Updates {
    pub fn of(cx: &App) -> &Updates {
        cx.global::<Updates>()
    }

    /// The line under "Shepherd <version>" in Settings → About.
    pub fn line(&self) -> String {
        match self {
            Updates::Unknown => "A herdr client for macOS".into(),
            Updates::Checking => "Checking for updates…".into(),
            Updates::UpToDate => "Up to date".into(),
            Updates::Available(r) => format!("Shepherd {} is available", r.version),
            Updates::Downloading { percent, .. } => format!("Downloading · {percent}%"),
            Updates::Installing => "Installing, Shepherd restarts in a moment…".into(),
            Updates::Failed(why, _) => format!("Update failed: {why}"),
        }
    }

    pub fn busy(&self) -> bool {
        matches!(self, Updates::Checking | Updates::Downloading { .. } | Updates::Installing)
    }

    /// The sidebar strip: its label and the release a click installs. `None` while there is nothing to offer.
    pub fn strip(&self) -> Option<(String, Option<Release>)> {
        match self {
            Updates::Available(r) if notifies() => Some((format!("Shepherd v{} is out", r.version), Some(r.clone()))),
            Updates::Available(r) => Some((format!("Update to v{}", r.version), Some(r.clone()))),
            Updates::Downloading { version, percent } => Some((format!("Downloading v{version}… {percent}%"), None)),
            Updates::Installing => Some(("Installing, Shepherd restarts…".into(), None)),
            Updates::Failed(why, Some(r)) => Some((format!("Update failed: {why}"), Some(r.clone()))),
            _ => None,
        }
    }
}

/// The `.app` this release build runs from; none for debug builds (`cargo run` runs from a bundle too, which
/// must not be swapped for a release).
pub fn app_bundle() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return None;
    }
    bundle_of(&std::env::current_exe().ok()?)
}

/// `…/Shepherd.app/Contents/MacOS/shepherd` → `…/Shepherd.app`.
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent().filter(|d| d.ends_with("Contents/MacOS"))?;
    let app = macos.parent()?.parent()?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

/// Off macOS, a release build from CI, which sets `SHEPHERD_UPDATE_CHECK`. Builds from source never ask GitHub.
fn released_off_mac() -> bool {
    !cfg!(target_os = "macos") && !cfg!(debug_assertions) && option_env!("SHEPHERD_UPDATE_CHECK").is_some()
}

/// Off macOS, the binary this release build swaps for a newer one: the running one, when Shepherd may replace it.
/// A package's (`/usr/bin`) is the package manager's, even to root.
fn binary() -> Option<PathBuf> {
    static BINARY: OnceLock<Option<PathBuf>> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            let exe = std::env::current_exe().ok().filter(|_| released_off_mac())?;
            (!exe.starts_with("/usr/bin") && writable(exe.parent()?)).then_some(exe)
        })
        .clone()
}

/// A file can be made in `dir`, so the binary in it can be renamed aside and another moved in.
fn writable(dir: &Path) -> bool {
    let probe = dir.join(".shepherd-update-probe");
    let made = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    made
}

/// What the updater replaces: the app bundle on macOS, the [`binary`] elsewhere.
fn target() -> Option<PathBuf> {
    app_bundle().or_else(binary)
}

/// This build only tells of new releases: one from CI off macOS whose binary it may not replace.
pub fn notifies() -> bool {
    released_off_mac() && binary().is_none()
}

/// This build checks for updates: one that installs them, or one that [`notifies`].
pub fn checks() -> bool {
    target().is_some() || notifies()
}

/// Sets up the state and, when it [`checks`], checks now and every few hours.
pub fn start(cx: &mut App) {
    cx.set_global(Updates::default());
    // Windows can't delete a running binary, so the one the last update moved aside goes now.
    if let Some(exe) = binary() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
    }
    if !checks() {
        return;
    }
    cx.spawn(async move |cx| {
        loop {
            cx.update(|cx| check(cx, true));
            cx.background_executor().timer(EVERY).await;
        }
    })
    .detach();
}

/// Asks GitHub for the latest release. A `quiet` check (the timer's) keeps what was shown when it fails and
/// leaves an offered update alone.
pub fn check(cx: &mut App, quiet: bool) {
    let before = Updates::of(cx).clone();
    if before.busy() || quiet && matches!(before, Updates::Available(_) | Updates::Failed(_, Some(_))) {
        return;
    }
    set(cx, Updates::Checking);
    cx.spawn(async move |cx| {
        let found = cx.background_spawn(async { latest() }).await;
        cx.update(|cx| {
            set(
                cx,
                match found {
                    Ok(r) if is_newer(&r.version, env!("CARGO_PKG_VERSION")) => Updates::Available(r),
                    Ok(_) => Updates::UpToDate,
                    Err(_) if quiet => before,
                    Err(e) => Updates::Failed(format!("{e:#}"), None),
                },
            )
        });
    })
    .detach();
}

/// Downloads `release`, swaps the app for it and relaunches; in a build that only [`notifies`], opens its page.
pub fn install(release: Release, cx: &mut App) {
    if notifies() {
        return crate::ui::open_outside(&release.url, cx);
    }
    let Some(app) = target() else { return };
    if Updates::of(cx).busy() {
        return;
    }
    set(cx, Updates::Downloading { version: release.version.clone(), percent: 0 });
    // The percent comes from the download's thread; the stream ends when the download drops its sender.
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<u64>();
    let version = release.version.clone();
    let progress = cx.spawn(async move |cx| {
        while let Some(percent) = rx.next().await {
            cx.update(|cx| set(cx, Updates::Downloading { version: version.clone(), percent }));
        }
    });
    cx.spawn(async move |cx| {
        let r = release.clone();
        let zip = cx
            .background_spawn(async move { download(&r, &std::env::temp_dir().join("shepherd-update"), |p| drop(tx.unbounded_send(p))) })
            .await;
        progress.await;
        let done = match zip {
            Ok((zip, digest)) => {
                cx.update(|cx| set(cx, Updates::Installing));
                let target = app.clone();
                let r = release.clone();
                cx.background_spawn(async move {
                    if cfg!(target_os = "macos") {
                        replace(&zip, &target)
                    } else {
                        let checked = signature(&r).and_then(|sig| verify_signature(&digest, &sig, &UPDATE_KEY));
                        if checked.is_err() {
                            let _ = std::fs::remove_file(&zip);
                        }
                        checked.and_then(|()| replace_binary(&zip, &target))
                    }
                })
                .await
                .and_then(|()| relaunch(&app))
            }
            Err(e) => Err(e),
        };
        cx.update(|cx| match done {
            Ok(()) => cx.quit(),
            Err(e) => set(cx, Updates::Failed(format!("{e:#}"), Some(release))),
        });
    })
    .detach();
}

fn set(cx: &mut App, updates: Updates) {
    cx.set_global(updates);
    cx.refresh_windows();
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .build()
        .into()
}

/// The latest release and its archive for this machine.
fn latest() -> anyhow::Result<Release> {
    let body = agent()
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("cannot reach GitHub")?
        .body_mut()
        .read_to_string()?;
    let json: serde_json::Value = serde_json::from_str(&body)?;
    if notifies() {
        return page_of(&json).context("the latest release has no page on GitHub");
    }
    let names = archives("<V>").join(" or ");
    release_of(&json).with_context(|| format!("the latest release has no {names} signed for the updater"))
}

/// The archives of `version` that update this machine, the one to prefer first: on macOS the universal zip (or, from
/// before universal builds, this arch's), on Linux and Windows this arch's, named as `scripts/linux-dist.sh` and
/// release.yml name them.
fn archives(version: &str) -> Vec<String> {
    let arch = std::env::consts::ARCH;
    if cfg!(target_os = "macos") {
        let arch = if arch == "aarch64" { "arm64" } else { arch };
        vec![format!("Shepherd-{version}-universal.zip"), format!("Shepherd-{version}-{arch}.zip")]
    } else if cfg!(windows) {
        vec![format!("Shepherd-{version}-windows-{arch}.zip")]
    } else {
        vec![format!("Shepherd-{version}-linux-{arch}.tar.gz")]
    }
}

/// A release from GitHub's JSON: `v0.2.0` with one of [`archives`] among its assets and, off macOS, its `.sig`.
fn release_of(json: &serde_json::Value) -> Option<Release> {
    let version = json["tag_name"].as_str()?.strip_prefix('v')?.to_string();
    let assets = json["assets"].as_array()?;
    let named = |name: &str| assets.iter().find(|a| a["name"] == name);
    let (name, asset) = archives(&version).into_iter().find_map(|name| named(&name).map(|a| (name, a)))?;
    let sig = match cfg!(target_os = "macos") {
        true => None,
        false => Some(named(&format!("{name}.sig"))?["browser_download_url"].as_str()?.to_string()),
    };
    Some(Release { url: asset["browser_download_url"].as_str()?.into(), size: asset["size"].as_u64()?, version, sig })
}

/// A release from GitHub's JSON as a page to open: `v0.2.0` and its `html_url`, which must be one of [`PAGES`].
fn page_of(json: &serde_json::Value) -> Option<Release> {
    let version = json["tag_name"].as_str()?.strip_prefix('v')?.to_string();
    let url = json["html_url"].as_str().filter(|u| u.starts_with(PAGES))?.to_string();
    Some(Release { version, url, size: 0, sig: None })
}

/// The release's signature, as `scripts/sign-update.sh` makes it: 64 bytes.
fn signature(release: &Release) -> anyhow::Result<Vec<u8>> {
    let url = release.sig.as_deref().context("the release has no signature")?;
    let sig = agent().get(url).call().context("cannot download the signature")?.body_mut().with_config().limit(1024).read_to_vec()?;
    Ok(sig)
}

/// `sig` is `key`'s Ed25519 signature of the archive's SHA-256.
fn verify_signature(digest: &[u8], sig: &[u8], key: &[u8]) -> anyhow::Result<()> {
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(digest, sig)
        .map_err(|_| anyhow::anyhow!("the download isn't signed by Shepherd's release key"))
}

/// `latest` is a later `x.y.z` than `current`; anything unreadable is not newer.
fn is_newer(latest: &str, current: &str) -> bool {
    let parts = |v: &str| v.split('.').map(|p| p.parse::<u64>().ok()).collect::<Option<Vec<_>>>();
    matches!((parts(latest), parts(current)), (Some(l), Some(c)) if l > c)
}

/// The release's archive into `dir`, with its percent as it comes; with the archive's SHA-256.
fn download(release: &Release, dir: &Path, mut progress: impl FnMut(u64)) -> anyhow::Result<(PathBuf, Vec<u8>)> {
    std::fs::create_dir_all(dir)?;
    let name = release.url.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("Shepherd-update");
    let zip = dir.join(name);
    let response = agent().get(&release.url).call().with_context(|| format!("cannot download {}", release.url))?;
    let mut body = response.into_body().into_with_config().limit(release.size + 1).reader();
    let mut file = std::fs::File::create(&zip).with_context(|| zip.display().to_string())?;
    let (mut buf, mut done, mut shown) = (vec![0; 1 << 16], 0u64, 0);
    let mut digest = Sha256::new();
    loop {
        let n = body.read(&mut buf).context("the download broke off")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        digest.update(&buf[..n]);
        done += n as u64;
        let percent = done * 100 / release.size.max(1);
        if percent != shown {
            shown = percent;
            progress(percent);
        }
    }
    if done != release.size {
        bail!("the download stopped at {done} of {} bytes", release.size);
    }
    Ok((zip, digest.finalize().to_vec()))
}

/// Unpacks `archive` (the tar.gz's `bin/shepherd`, the zip's `shepherd.exe`) beside `exe` and puts the binary in its
/// place: the old one is renamed aside first and comes back if the new one cannot move in. A running binary may be
/// renamed on Windows too, though not deleted; Linux deletes it at once, Windows at the next launch ([`start`]).
/// ponytail: only the binary; the tar.gz's .desktop file and icon stay as they were installed.
fn replace_binary(archive: &Path, exe: &Path) -> anyhow::Result<()> {
    let stage = exe.with_extension("update");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).with_context(|| stage.display().to_string())?;
    // bsdtar on Windows 10 and later unpacks a zip too; GNU tar tells gzip by itself.
    let out = Command::new("tar").arg("-xf").arg(archive).arg("-C").arg(&stage).output().context("spawn tar")?;
    let result = (|| {
        if !out.status.success() {
            bail!("cannot unpack {}: {}", archive.display(), String::from_utf8_lossy(&out.stderr).trim());
        }
        let name = format!("shepherd{}", std::env::consts::EXE_SUFFIX);
        let new = [stage.join("bin").join(&name), stage.join(&name)]
            .into_iter()
            .find(|p| p.is_file())
            .with_context(|| format!("{} holds no {name}", archive.display()))?;
        let old = exe.with_extension("old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(exe, &old).with_context(|| format!("move {} aside", exe.display()))?;
        if let Err(e) = std::fs::rename(&new, exe) {
            let _ = std::fs::rename(&old, exe);
            return Err(e).with_context(|| format!("move the new binary to {}", exe.display()));
        }
        if cfg!(unix) {
            let _ = std::fs::remove_file(&old);
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&stage);
    let _ = std::fs::remove_file(archive);
    result
}

/// Unpacks `zip` beside `app` and puts its `.app` in its place: the old one is renamed aside first and comes back
/// if the new one cannot move in. The running process keeps working from the renamed bundle.
fn replace(zip: &Path, app: &Path) -> anyhow::Result<()> {
    let stage = app.with_extension("update");
    let _ = std::fs::remove_dir_all(&stage);
    let out = Command::new("ditto").args(["-x", "-k"]).arg(zip).arg(&stage).output().context("spawn ditto")?;
    if !out.status.success() {
        bail!("cannot unpack {}: {}", zip.display(), String::from_utf8_lossy(&out.stderr).trim());
    }
    let result = (|| {
        let unpacked = std::fs::read_dir(&stage)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "app") && p.join("Contents/MacOS").is_dir())
            .with_context(|| format!("{} holds no app", zip.display()))?;
        verify(&unpacked, app)?;
        let old = app.with_extension("old.app");
        let _ = std::fs::remove_dir_all(&old);
        std::fs::rename(app, &old).with_context(|| format!("move {} aside", app.display()))?;
        if let Err(e) = std::fs::rename(&unpacked, app) {
            let _ = std::fs::rename(&old, app);
            return Err(e).with_context(|| format!("move the new app to {}", app.display()));
        }
        let _ = std::fs::remove_dir_all(&old);
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&stage);
    let _ = std::fs::remove_file(zip);
    result
}

/// The new app's signature holds and, when the running app is signed by a team, it is that team's.
fn verify(new: &Path, old: &Path) -> anyhow::Result<()> {
    let out = Command::new("codesign").args(["--verify", "--deep", "--strict"]).arg(new).output().context("spawn codesign")?;
    if !out.status.success() {
        bail!("the new app's signature is broken: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    // ponytail: an ad hoc signed app (released without the signing secrets) takes any intact signature.
    if let Some(want) = team(old)
        && team(new).as_deref() != Some(want.as_str())
    {
        bail!("the new app is not signed by team {want}");
    }
    Ok(())
}

/// The signing team of `app`; none when ad hoc signed.
fn team(app: &Path) -> Option<String> {
    let out = Command::new("codesign").arg("-dv").arg(app).output().ok()?;
    let info = String::from_utf8_lossy(&out.stderr);
    info.lines().find_map(|l| l.strip_prefix("TeamIdentifier=")).filter(|t| *t != "not set").map(str::to_string)
}

/// Starts `app` (the bundle on macOS, the binary elsewhere) once this process has quit; the caller quits right after.
fn relaunch(app: &Path) -> anyhow::Result<()> {
    let pid = std::process::id();
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt as _;
        let app = app.display().to_string().replace('\'', "''");
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(format!("Wait-Process -Id {pid} -ErrorAction SilentlyContinue; Start-Process -FilePath '{app}'"));
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashes up
        cmd
    };
    #[cfg(not(windows))]
    let mut cmd = {
        use std::os::unix::process::CommandExt as _;
        let start = if cfg!(target_os = "macos") { r#"exec open "$2""# } else { r#"exec "$2""# };
        let mut cmd = Command::new("sh");
        cmd.args(["-c", &format!(r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; {start}"#), "_"])
            .arg(pid.to_string())
            .arg(app);
        // A group of its own, so a terminal's hangup on Shepherd's group doesn't take it along.
        cmd.process_group(0);
        cmd
    };
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawn the relauncher")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_for_this_machine() {
        let name = archives("0.2.0").pop().unwrap(); // on macOS, this arch's zip
        let json = serde_json::json!({
            "tag_name": "v0.2.0",
            "assets": [
                { "name": "Shepherd-0.2.0-other.zip", "size": 1, "browser_download_url": "https://x/other" },
                { "name": name, "size": 7, "browser_download_url": "https://x/zip" },
                { "name": format!("{name}.sig"), "size": 64, "browser_download_url": "https://x/sig" },
            ],
        });
        let sig = (!cfg!(target_os = "macos")).then(|| "https://x/sig".to_string());
        assert_eq!(release_of(&json), Some(Release { version: "0.2.0".into(), url: "https://x/zip".into(), size: 7, sig }));
        assert_eq!(release_of(&serde_json::json!({ "tag_name": "v0.2.0", "assets": [] })), None);
        // Off macOS an archive without its signature can't be installed.
        let unsigned = serde_json::json!({
            "tag_name": "v0.2.0",
            "assets": [{ "name": name, "size": 7, "browser_download_url": "https://x/zip" }],
        });
        assert_eq!(release_of(&unsigned).is_some(), cfg!(target_os = "macos"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn a_universal_zip_wins_over_an_arch() {
        let [universal, arch] = <[String; 2]>::try_from(archives("0.3.0")).unwrap();
        let json = serde_json::json!({
            "tag_name": "v0.3.0",
            "assets": [
                { "name": arch, "size": 7, "browser_download_url": "https://x/arch" },
                { "name": universal, "size": 9, "browser_download_url": "https://x/universal" },
                { "name": "Shepherd-0.3.0-universal.dmg", "size": 9, "browser_download_url": "https://x/dmg" },
            ],
        });
        let want = Release { version: "0.3.0".into(), url: "https://x/universal".into(), size: 9, sig: None };
        assert_eq!(release_of(&json), Some(want));
    }

    /// What `scripts/sign-update.sh` writes is what the updater checks: a key made here signs a file through the
    /// script, and only that file and that key pass.
    #[test]
    #[cfg(unix)] // sh and openssl
    fn the_ci_signature_checks_out() {
        let tmp = tempfile::tempdir().unwrap();
        let (key, file) = (tmp.path().join("key.pem"), tmp.path().join("Shepherd.tar.gz"));
        assert!(Command::new("openssl").args(["genpkey", "-algorithm", "ed25519", "-out"]).arg(&key).status().unwrap().success());
        let der = Command::new("openssl").args(["pkey", "-pubout", "-outform", "DER", "-in"]).arg(&key).output().unwrap().stdout;
        let public = &der[der.len() - 32..];
        std::fs::write(&file, b"a release").unwrap();
        let signed = Command::new("sh")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/sign-update.sh"))
            .arg(&file)
            .env("SHEPHERD_UPDATE_KEY", std::fs::read_to_string(&key).unwrap())
            .status()
            .unwrap();
        assert!(signed.success());
        let sig = std::fs::read(tmp.path().join("Shepherd.tar.gz.sig")).unwrap();
        let digest = Sha256::digest(b"a release");
        verify_signature(&digest, &sig, public).unwrap();
        assert!(verify_signature(&Sha256::digest(b"another release"), &sig, public).is_err());
        assert!(verify_signature(&digest, &sig, &UPDATE_KEY).is_err(), "a key of one's own isn't Shepherd's");
    }

    #[test]
    #[cfg(unix)] // tar to pack the archives; the swap itself is the same on Windows
    fn the_new_binary_replaces_the_old_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, build) = (tmp.path().join("bin"), tmp.path().join("build"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(build.join("bin")).unwrap();
        let exe = dir.join("shepherd");
        std::fs::write(&exe, "old").unwrap();
        std::fs::write(build.join("bin/shepherd"), "new").unwrap();
        let archive = tmp.path().join("Shepherd-0.2.0-linux.tar.gz");
        let pack = |from: &Path| Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(from).arg(".").status().unwrap();
        assert!(pack(&build).success());
        replace_binary(&archive, &exe).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no stage or old binary left behind");

        // An archive without the binary leaves the installed one alone.
        std::fs::remove_file(build.join("bin/shepherd")).unwrap();
        std::fs::write(build.join("README"), "nothing").unwrap();
        assert!(pack(&build).success());
        let e = replace_binary(&archive, &exe).unwrap_err().to_string();
        assert!(e.contains("holds no shepherd"), "{e}");
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn the_release_page() {
        let page = |url: &str| page_of(&serde_json::json!({ "tag_name": "v0.2.0", "html_url": url, "assets": [] }));
        let url = "https://github.com/getshepherd/shepherd/releases/tag/v0.2.0";
        assert_eq!(page(url), Some(Release { version: "0.2.0".into(), url: url.into(), size: 0, sig: None }));
        assert_eq!(page("https://github.com/getshepherd/shepherd-evil/releases/tag/v0.2.0"), None);
        assert_eq!(page("http://github.com/getshepherd/shepherd/releases/tag/v0.2.0"), None);
        assert_eq!(page_of(&serde_json::json!({ "tag_name": "v0.2.0" })), None);
    }

    #[test]
    fn newer_versions() {
        assert!(is_newer("0.2.0", "0.1.1"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("0.2.0-beta", "0.1.1"));
    }

    #[test]
    fn the_app_bundle_of_the_executable() {
        assert_eq!(bundle_of(Path::new("/Applications/Shepherd.app/Contents/MacOS/shepherd")), Some("/Applications/Shepherd.app".into()));
        assert_eq!(bundle_of(Path::new("/r/target/debug/shepherd")), None);
    }

    /// An ad hoc signed bundle whose `Contents/Resources/which` says which one it is.
    #[cfg(target_os = "macos")]
    fn fake_app(app: &Path, which: &str) {
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        std::fs::copy("/usr/bin/true", app.join("Contents/MacOS/shepherd")).unwrap();
        std::fs::write(app.join("Contents/Resources/which"), which).unwrap();
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>shepherd</string><key>CFBundleIdentifier</key><string>test.shepherd</string></dict></plist>"#;
        std::fs::write(app.join("Contents/Info.plist"), plist).unwrap();
        assert!(Command::new("codesign").args(["-s", "-", "--force"]).arg(app).status().unwrap().success());
    }

    #[test]
    #[cfg(target_os = "macos")] // ditto and codesign
    fn the_new_app_replaces_the_old_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (apps, build) = (tmp.path().join("Applications"), tmp.path().join("build"));
        let app = apps.join("Shepherd.app");
        fake_app(&app, "old");
        fake_app(&build.join("Shepherd.app"), "new");
        let zip = tmp.path().join("Shepherd-0.2.0.zip");
        let zipped = Command::new("ditto").args(["-c", "-k", "--keepParent"]).arg(build.join("Shepherd.app")).arg(&zip).status().unwrap();
        assert!(zipped.success());
        replace(&zip, &app).unwrap();
        assert_eq!(std::fs::read_to_string(app.join("Contents/Resources/which")).unwrap(), "new");
        assert_eq!(std::fs::read_dir(&apps).unwrap().count(), 1, "no stage or old app left behind");

        // A tampered app leaves the installed one alone.
        std::fs::write(build.join("Shepherd.app/Contents/Resources/which"), "evil").unwrap();
        let zipped = Command::new("ditto").args(["-c", "-k", "--keepParent"]).arg(build.join("Shepherd.app")).arg(&zip).status().unwrap();
        assert!(zipped.success());
        let e = replace(&zip, &app).unwrap_err().to_string();
        assert!(e.contains("signature"), "{e}");
        assert_eq!(std::fs::read_to_string(app.join("Contents/Resources/which")).unwrap(), "new");
    }

    /// The latest release from GitHub over the app at `SHEPHERD_UPDATE_APP` (network; run by hand).
    #[test]
    #[ignore]
    fn updates_an_app_from_github() {
        let app = PathBuf::from(std::env::var("SHEPHERD_UPDATE_APP").expect("SHEPHERD_UPDATE_APP"));
        let release = latest().unwrap();
        let mut seen = Vec::new();
        let (zip, _) = download(&release, &std::env::temp_dir().join("shepherd-update-test"), |p| seen.push(p)).unwrap();
        assert_eq!(seen.last(), Some(&100));
        replace(&zip, &app).unwrap();
    }
}
