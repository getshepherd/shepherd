//! Extensions: paddock binaries (getshepherd/extensions), each a local page with an API and a CLI for agents, in
//! `<config dir>/extensions/<name>/`. Shepherd lists them in Settings → Extensions and in the sidebar, runs the ones
//! that start with it, installs them from a GitHub release and dresses their pages in its theme. The contract is
//! paddock's README, "For Shepherd": a folder with `state.json` and the binary `<name>`; Shepherd knows nothing else.
use crate::settings::WebEntry;
use crate::theme::{Appearance, Theme};
use gpui::{App, AppContext, AsyncApp, Global, Hsla};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// An extension's sidebar entry id starts with this; a page added in Settings has a hex id.
pub const PAGE_PREFIX: &str = "ext:";
/// How often the folder is scanned, the servers pinged and the `serve`s looked after.
const EVERY: Duration = Duration::from_secs(2);
/// A `serve` that exits before running this long counts as a crash in a row.
const STEADY: Duration = Duration::from_secs(30);
/// After this many crashes in a row it is left alone until its binary changes.
const GIVE_UP: u32 = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct Extension {
    /// Its folder's name, and its binary's.
    pub name: String,
    pub description: String,
    pub port: u16,
    pub token: String,
    /// `login` (the system runs it) or `shepherd` (Shepherd does, while it is open).
    pub start: String,
    pub binary: PathBuf,
    /// Its server answered the last ping.
    pub alive: bool,
    /// The GitHub release it was installed from, when Shepherd installed it: what it is updated from.
    pub source: Option<Source>,
}

/// `source.json` in an extension's folder, Shepherd's own beside paddock's `state.json`: the repo and the release tag
/// it installed. An extension installed by hand (`go run . install`) has none and isn't offered updates.
#[derive(Debug, Clone, PartialEq, serde::Serialize, Deserialize)]
pub struct Source {
    pub repo: String,
    pub tag: String,
}

const SOURCE: &str = "source.json";

/// The extensions installed, as last scanned.
#[derive(Default)]
pub struct Extensions(pub Vec<Extension>);

impl Global for Extensions {}

#[derive(Deserialize)]
struct State {
    #[serde(default)]
    description: String,
    port: u16,
    token: String,
    #[serde(default)]
    start: String,
}

pub fn root() -> PathBuf {
    crate::settings::config_dir().join("extensions")
}

/// The folders in `root` that hold a `state.json` and the binary named after the folder, by name.
pub fn scan(root: &Path) -> Vec<Extension> {
    let Ok(dirs) = std::fs::read_dir(root) else { return Vec::new() };
    let mut found: Vec<Extension> = dirs
        .flatten()
        .filter_map(|d| {
            let (dir, name) = (d.path(), d.file_name().to_str()?.to_string());
            let state: State = serde_json::from_slice(&std::fs::read(dir.join("state.json")).ok()?).ok()?;
            let binary = dir.join(if cfg!(windows) { format!("{name}.exe") } else { name.clone() });
            // paddock's default when it is unset
            let start = if state.start.is_empty() { "login".into() } else { state.start };
            let source = std::fs::read(dir.join(SOURCE)).ok().and_then(|b| serde_json::from_slice(&b).ok());
            binary.is_file().then_some(Extension { name, description: state.description, port: state.port, token: state.token, start, binary, alive: false, source })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

impl Extension {
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn url(&self) -> String {
        format!("{}/?token={}", self.origin(), self.token)
    }

    pub fn runs_with_shepherd(&self) -> bool {
        self.start == "shepherd"
    }

    /// Its page as a sidebar entry, listed everywhere and opened here.
    pub fn entry(&self) -> WebEntry {
        WebEntry { id: format!("{PAGE_PREFIX}{}", self.name), name: self.name.clone(), url: self.url(), ..Default::default() }
    }
}

/// The extensions' sidebar entries.
pub fn entries(cx: &App) -> Vec<WebEntry> {
    cx.try_global::<Extensions>().map(|e| e.0.iter().map(Extension::entry).collect()).unwrap_or_default()
}

/// The extension whose page has sidebar id `id`.
pub fn by_page<'a>(cx: &'a App, id: &str) -> Option<&'a Extension> {
    let name = id.strip_prefix(PAGE_PREFIX)?;
    cx.try_global::<Extensions>()?.0.iter().find(|e| e.name == name)
}

/// Whether its server answers `/_paddock/ping` with its token. Blocking, a second at most.
pub fn ping(port: u16, token: &str) -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(1))).build().into();
    agent.get(format!("http://127.0.0.1:{port}/_paddock/ping")).header("Authorization", format!("Bearer {token}")).call().is_ok()
}

/// Scans every few seconds from now on, and stops the `serve`s Shepherd started when it quits.
pub fn start(cx: &mut App) {
    cx.set_global(Extensions::default());
    cx.spawn(async move |cx| {
        loop {
            rescan(cx).await;
            cx.background_executor().timer(EVERY).await;
        }
    })
    .detach();
    cx.on_app_quit(|_| {
        stop_all();
        async {}
    })
    .detach();
}

/// Scans now, not at the next tick: after an install or a switch.
pub fn refresh(cx: &mut App) {
    cx.spawn(async move |cx| rescan(cx).await).detach();
}

/// The folder scanned and the servers pinged (off the main thread), the `serve`s looked after, and the list put in
/// the global when it changed.
async fn rescan(cx: &mut AsyncApp) {
    let list = cx
        .background_spawn(async {
            let mut list = scan(&root());
            for e in &mut list {
                e.alive = ping(e.port, &e.token);
            }
            supervise(&list);
            list
        })
        .await;
    cx.update(|cx| {
        if cx.global::<Extensions>().0 != list {
            cx.set_global(Extensions(list));
            cx.refresh_windows();
        }
    });
}

/// A `serve` Shepherd runs, and what decides when it starts again.
struct Run {
    child: Option<Child>,
    /// The binary's modification time it was started from: installing again renames a new one over it.
    binary: Option<SystemTime>,
    started: Instant,
    failures: u32,
    retry_at: Instant,
    /// Stopped from Settings or the sidebar: not started again until started there or its page is opened.
    held: bool,
}

/// `ext`'s run, a new one if it has none.
fn run_of<'a>(runs: &'a mut HashMap<String, Run>, ext: &Extension, now: Instant) -> &'a mut Run {
    let binary = modified(&ext.binary);
    runs.entry(ext.name.clone()).or_insert(Run { child: None, binary, started: now, failures: 0, retry_at: now, held: false })
}

static RUNS: LazyLock<Mutex<HashMap<String, Run>>> = LazyLock::new(Default::default);

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn stop(run: &mut Run) {
    if let Some(mut child) = run.child.take() {
        // ponytail: SIGKILL, no time to wind down; paddock keeps nothing it would lose. A SIGTERM first if one ever does.
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// How long after its `n`th crash in a row a `serve` starts again: 2, 4, 8, 16 s.
fn pause(n: u32) -> Duration {
    Duration::from_secs(1 << n.min(4))
}

/// Starts the `serve` of each extension that starts with Shepherd, restarts one whose binary changed, starts one that
/// exited again after a pause, and gives up after `GIVE_UP` crashes in a row until its binary changes. Those gone, or
/// switched to login, stop.
fn supervise(list: &[Extension]) {
    let mut runs = RUNS.lock().unwrap();
    runs.retain(|name, run| {
        let keep = list.iter().any(|e| &e.name == name && e.runs_with_shepherd());
        if !keep {
            stop(run);
        }
        keep
    });
    let now = Instant::now();
    for ext in list.iter().filter(|e| e.runs_with_shepherd()) {
        let binary = modified(&ext.binary);
        let run = run_of(&mut runs, ext, now);
        if run.binary != binary {
            stop(run);
            (run.binary, run.failures, run.retry_at) = (binary, 0, now);
        }
        if let Some(child) = &mut run.child {
            if matches!(child.try_wait(), Ok(None)) {
                continue;
            }
            run.child = None;
            run.failures = if run.started.elapsed() < STEADY { run.failures + 1 } else { 1 };
            run.retry_at = now + pause(run.failures);
            eprintln!("shepherd: extension {} exited; its log is {}", ext.name, ext.binary.with_file_name("log").display());
        }
        // something else already serves it: a `serve` run by hand, or one a Shepherd that crashed left behind
        // ponytail: that one isn't restarted when its binary changes; it would be, killed by its pid from paddock's state
        if run.held || run.failures >= GIVE_UP || now < run.retry_at || ext.alive {
            continue;
        }
        match serve(ext) {
            Ok(child) => (run.child, run.started) = (Some(child), now),
            Err(e) => {
                eprintln!("shepherd: cannot start extension {}: {e}", ext.name);
                run.failures += 1;
                run.retry_at = now + pause(run.failures);
            }
        }
    }
}

/// `<binary> serve` in its folder, its output added to `log` there.
fn serve(ext: &Extension) -> std::io::Result<Child> {
    let dir = ext.binary.parent().unwrap_or(Path::new("."));
    let log = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("log"))?;
    crate::process::command(&ext.binary).arg("serve").current_dir(dir).stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log).spawn()
}

/// Stops every `serve` Shepherd started.
pub fn stop_all() {
    for run in RUNS.lock().unwrap().values_mut() {
        stop(run);
    }
}

/// Starts its server: Shepherd's own `serve` (one that gave up or was stopped too), or the system's service
/// (`<binary> start`); then waits, a few seconds at most, for it to answer. Blocking.
pub fn start_server(ext: &Extension) -> Result<(), String> {
    if ext.runs_with_shepherd() {
        let mut runs = RUNS.lock().unwrap();
        let now = Instant::now();
        let run = run_of(&mut runs, ext, now);
        (run.held, run.failures, run.retry_at) = (false, 0, now);
        if run.child.is_none() && !ping(ext.port, &ext.token) {
            run.child = Some(serve(ext).map_err(|e| format!("{}: {e}", ext.binary.display()))?);
            run.started = now;
        }
    } else {
        run(&ext.binary, &["start"])?;
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ping(ext.port, &ext.token) {
        if Instant::now() > deadline {
            return Err(format!("{} didn't answer; its log is {}", ext.name, ext.binary.with_file_name("log").display()));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

/// Stops its server: Shepherd's own `serve`, held so it isn't started again, or the system's service (`<binary> stop`).
/// Blocking.
pub fn stop_server(ext: &Extension) -> Result<(), String> {
    if !ext.runs_with_shepherd() {
        return run(&ext.binary, &["stop"]);
    }
    let mut runs = RUNS.lock().unwrap();
    let run = run_of(&mut runs, ext, Instant::now());
    run.held = true;
    let ours = run.child.is_some();
    stop(run);
    // ponytail: one Shepherd didn't start (a `serve` by hand, or left by a Shepherd that crashed) is only held, not
    // killed; it would be, by its pid from paddock's state
    if !ours && ext.alive {
        return Err(format!("{} is served by a process Shepherd didn't start: stop that one", ext.name));
    }
    Ok(())
}

/// Switches when the extension runs: `<binary> install --start <start>`, which sets up or takes down the system's
/// service. Shepherd's own `serve` stops first when the system takes over, so the two don't fight for the port; the
/// next scan starts one when Shepherd does. Blocking.
pub fn set_start(ext: &Extension, start: &str) -> Result<(), String> {
    // held throughout, so a scan from before the switch doesn't start the `serve` again meanwhile
    let mut runs = RUNS.lock().unwrap();
    if start != "shepherd"
        && let Some(mut run) = runs.remove(&ext.name)
    {
        stop(&mut run);
    }
    run(&ext.binary, &["install", "--start", start])
}

/// Removes the extension: `<binary> uninstall`, which takes down its service, its skill and its binary and leaves its
/// data, so installing it again finds them. Shepherd's own `serve` stops first (Windows can't remove a running exe).
/// Blocking.
pub fn uninstall(ext: &Extension) -> Result<(), String> {
    let mut runs = RUNS.lock().unwrap();
    if let Some(mut run) = runs.remove(&ext.name) {
        stop(&mut run);
    }
    run(&ext.binary, &["uninstall"])
}

/// `<binary> <args>`, or what it said when it failed.
fn run(binary: &Path, args: &[&str]) -> Result<(), String> {
    let out = crate::process::command(binary).args(args).stdin(Stdio::null()).output().map_err(|e| format!("{}: {e}", binary.display()))?;
    if out.status.success() {
        return Ok(());
    }
    let said = [&out.stderr, &out.stdout].map(|b| String::from_utf8_lossy(b).trim().to_string());
    Err(said.into_iter().find(|s| !s.is_empty()).unwrap_or_else(|| format!("{} exited with {}", args.join(" "), out.status)))
}

/// A GitHub release's build of an extension for this machine.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub repo: String,
    pub tag: String,
    pub asset: String,
    url: String,
}

/// `owner/repo` from what was typed: that, or the repo's github.com URL (a page in it too).
pub fn parse_repo(input: &str) -> Option<String> {
    let s = input.trim();
    let s = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(s);
    let s = s.strip_prefix("www.").unwrap_or(s);
    let s = s.strip_prefix("github.com/").unwrap_or(s);
    let mut parts = s.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?.trim_end_matches(".git"));
    // GitHub's owners have no dots, so `gitlab.com/a/b` isn't taken for one
    let owner_ok = !owner.is_empty() && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let repo_ok = !repo.is_empty() && repo.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    (owner_ok && repo_ok).then(|| format!("{owner}/{repo}"))
}

/// The end of the asset name built for this machine, as paddock's release workflow names them: `-darwin-arm64`,
/// `-windows-amd64.exe`.
pub fn asset_suffix() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        os => os,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        arch => arch,
    };
    format!("-{os}-{arch}{}", if cfg!(windows) { ".exe" } else { "" })
}

/// This machine's build in GitHub's JSON of a release, or why there is none.
fn release_of(repo: &str, json: &serde_json::Value, suffix: &str) -> Result<Release, String> {
    let tag = json["tag_name"].as_str().unwrap_or_default().to_string();
    let named = |a: &&serde_json::Value| a["name"].as_str().is_some_and(|n| n.ends_with(suffix));
    let Some(asset) = json["assets"].as_array().into_iter().flatten().find(named) else {
        return Err(format!("{repo}'s release {tag} has no build for this computer (a file ending in {suffix})."));
    };
    let url = asset["browser_download_url"].as_str().ok_or("GitHub's answer has no download link")?.to_string();
    Ok(Release { repo: repo.into(), tag, asset: asset["name"].as_str().unwrap_or_default().into(), url })
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(15))).timeout_recv_response(Some(Duration::from_secs(30))).build().into()
}

/// `repo`'s latest release, with its build for this machine. Blocking.
pub fn latest_release(repo: &str) -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let body = match agent().get(&url).header("Accept", "application/vnd.github+json").call() {
        Ok(mut r) => r.body_mut().read_to_string().map_err(|e| e.to_string())?,
        Err(ureq::Error::StatusCode(404)) => return Err(format!("{repo} has no release on GitHub, or is not a public repo.")),
        Err(e) => return Err(format!("Cannot reach GitHub: {e}")),
    };
    let json = serde_json::from_str(&body).map_err(|e| format!("GitHub's answer: {e}"))?;
    release_of(repo, &json, &asset_suffix())
}

/// Downloads the release's build to a folder of its own and runs it as `install --start <start>` (`shepherd` for a new
/// one, an update keeps its own), which copies it into the extensions; the download goes with the folder. Then notes
/// where it came from in its folder, for updates. Blocking.
pub fn install(release: &Release, start: &str) -> Result<(), String> {
    let since = SystemTime::now() - Duration::from_secs(1);
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = tmp.path().join(&release.asset);
    let bytes = agent()
        .get(&release.url)
        .call()
        .and_then(|mut r| r.body_mut().with_config().limit(512 << 20).read_to_vec())
        .map_err(|e| format!("Cannot download {}: {e}", release.asset))?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    run(&path, &["install", "--start", start])?;
    let source = Source { repo: release.repo.clone(), tag: release.tag.clone() };
    note_source(&root(), since, &source).map_err(|e| e.to_string())
}

/// Writes `source` into the folder of the extension installed since `since`: its folder is named by the binary
/// itself, so it is the one whose binary was just put there. None found (an install that wrote nowhere): nothing.
fn note_source(root: &Path, since: SystemTime, source: &Source) -> std::io::Result<()> {
    let installed = scan(root).into_iter().filter(|e| modified(&e.binary).is_some_and(|t| t >= since)).max_by_key(|e| modified(&e.binary));
    match installed {
        Some(ext) => std::fs::write(root.join(&ext.name).join(SOURCE), serde_json::to_vec(source)?),
        None => Ok(()),
    }
}

/// The release to update `ext` to: its repo's latest, when that isn't the one installed. `Ok(None)` when it is up to
/// date or wasn't installed from GitHub. Blocking.
pub fn update_for(ext: &Extension) -> Result<Option<Release>, String> {
    let Some(source) = &ext.source else { return Ok(None) };
    let latest = latest_release(&source.repo)?;
    Ok((latest.tag != source.tag).then_some(latest))
}

/// A color as CSS: `#rrggbb` when opaque.
fn css(c: Hsla) -> String {
    let c = gpui::Rgba::from(c);
    let b = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    match b(c.a) {
        255 => format!("#{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b)),
        _ => format!("rgb({} {} {} / {:.3})", b(c.r), b(c.g), b(c.b), c.a),
    }
}

/// paddock's CSS variables (the head of its `paddock.css`) from `theme`, derived as Shepherd draws the same things.
fn theme_vars(t: &Theme) -> Vec<(&'static str, String)> {
    let dark = t.appearance == Appearance::Dark;
    let [s1, s2, s3] = crate::ui::loader::ROW_TINTS.map(|c| css(gpui::rgb(c).into()));
    vec![
        ("background", css(t.background)),
        ("shell", css(t.surface)),
        ("raised", css(t.raised)),
        ("card", css(t.card)),
        ("dialog", css(t.dialog)),
        ("input", css(t.input)),
        ("hover", css(t.hover)),
        ("selected", css(t.selected)),
        ("kbd", css(t.wash(0.06))),
        ("focus", css(t.accent)),
        // `crate::ui::scrollbar`'s thumb
        ("scrollbar", css(t.text_muted.opacity(0.35))),
        ("scrollbar-active", css(t.text_muted.opacity(0.6))),
        ("border", css(t.border)),
        ("border-strong", css(t.border_strong)),
        ("text", css(t.text)),
        ("text-muted", css(t.text_muted)),
        ("text-faint", css(t.text_faint)),
        ("accent", css(t.accent)),
        ("on-accent", css(t.on_accent)),
        ("danger", css(t.danger)),
        ("warning", css(t.warning)),
        ("success", css(t.success)),
        ("on-color", "#ffffff".into()),
        // the command panel's
        ("scrim", "rgb(0 0 0 / 0.350)".into()),
        ("shadow", "0 10px 15px -3px rgb(0 0 0 / 0.1), 0 4px 6px -4px rgb(0 0 0 / 0.1)".into()),
        // `crate::ui::switch`'s
        ("switch-thumb", css(gpui::white().opacity(if dark { 0.96 } else { 1. }))),
        ("switch-thumb-edge", css(gpui::black().opacity(if dark { 0.10 } else { 0.08 }))),
        ("switch-off", css(t.text.opacity(if dark { 0.18 } else { 0.10 }))),
        ("switch-on-edge", css(gpui::white().opacity(0.12))),
        ("spinner-1", s1),
        ("spinner-2", s2),
        ("spinner-3", s3),
        ("font", format!("\"{}\", -apple-system, BlinkMacSystemFont, system-ui, sans-serif", t.font_ui)),
        ("mono", format!("\"{}\", ui-monospace, \"SF Mono\", Menlo, monospace", t.font_terminal)),
        ("font-size", "13px".into()),
    ]
}

/// The script that dresses a page of the extension at `origin` in `theme`: paddock's variables inline on the root,
/// which beats its defaults' media query, and the color scheme. Run at each load and again on each change of theme;
/// a page that went to another site is left alone.
pub fn theme_script(theme: &Theme, origin: &str) -> String {
    let vars: serde_json::Map<String, serde_json::Value> = theme_vars(theme).into_iter().map(|(k, v)| (format!("--{k}"), v.into())).collect();
    let scheme = if theme.appearance == Appearance::Dark { "dark" } else { "light" };
    let (vars, origin) = (serde_json::Value::Object(vars), serde_json::Value::from(origin));
    format!(
        r#"(() => {{
  if (location.origin !== {origin}) return;
  const set = () => {{
    const s = document.documentElement.style, vars = {vars};
    for (const k in vars) s.setProperty(k, vars[k]);
    s.colorScheme = '{scheme}';
  }};
  if (document.documentElement) set(); else document.addEventListener('readystatechange', set, {{ once: true }});
}})();"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repos_are_read_from_names_and_urls() {
        assert_eq!(parse_repo(" you/weather ").as_deref(), Some("you/weather"));
        assert_eq!(parse_repo("https://github.com/you/weather.git").as_deref(), Some("you/weather"));
        assert_eq!(parse_repo("github.com/you/weather/releases").as_deref(), Some("you/weather"));
        assert_eq!(parse_repo("gitlab.com/you/weather"), None);
        assert_eq!(parse_repo("weather"), None);
        assert_eq!(parse_repo("you/"), None);
    }

    #[test]
    fn a_release_gives_this_machines_build_or_says_why_not() {
        let json = serde_json::json!({
            "tag_name": "v0.1.0",
            "assets": [
                { "name": "weather-linux-amd64", "browser_download_url": "https://x/linux" },
                { "name": "weather-darwin-arm64", "browser_download_url": "https://x/mac" },
            ],
        });
        let r = release_of("you/weather", &json, "-darwin-arm64").unwrap();
        assert_eq!((r.tag.as_str(), r.asset.as_str(), r.url.as_str()), ("v0.1.0", "weather-darwin-arm64", "https://x/mac"));
        let e = release_of("you/weather", &json, "-windows-amd64.exe").unwrap_err();
        assert!(e.contains("no build for this computer"), "{e}");
    }

    #[test]
    fn the_release_installed_is_noted_in_its_folder_and_read_back() {
        let root = tempfile::tempdir().unwrap();
        let make = |name: &str| {
            let dir = root.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("state.json"), r#"{"port": 1, "token": "t"}"#).unwrap();
            std::fs::write(dir.join(if cfg!(windows) { format!("{name}.exe") } else { name.into() }), "bin").unwrap();
        };
        make("old");
        let since = SystemTime::now();
        std::thread::sleep(Duration::from_millis(20));
        make("weather");
        let source = Source { repo: "you/weather".into(), tag: "v0.2.0".into() };
        note_source(root.path(), since, &source).unwrap();
        let found = scan(root.path());
        assert_eq!(found.iter().find(|e| e.name == "weather").unwrap().source, Some(source));
        assert_eq!(found.iter().find(|e| e.name == "old").unwrap().source, None, "installed by hand: nothing to update from");
    }

    /// Live: getshepherd/automations' latest release installed from GitHub into a home of its own, its source noted, and
    /// an update offered only once the noted tag is another.
    /// `RUSTUP_HOME=~/.rustup CARGO_HOME=~/.cargo HOME=$(mktemp -d) cargo test --lib extensions::tests::live_update -- --ignored`
    #[test]
    #[ignore]
    fn live_update() {
        // and RUSTUP_HOME and CARGO_HOME the real ones, for cargo itself
        assert!(!root().join("automations").exists(), "a HOME of its own: it installs into it");
        let release = latest_release("getshepherd/automations").unwrap();
        install(&release, "shepherd").unwrap();
        let ext = scan(&root()).into_iter().find(|e| e.name == "automations").expect("installed");
        assert_eq!(ext.source, Some(Source { repo: "getshepherd/automations".into(), tag: release.tag.clone() }));
        assert_eq!(update_for(&ext), Ok(None), "the latest is in");
        let older = Extension { source: Some(Source { repo: "getshepherd/automations".into(), tag: "v0.0.0".into() }), ..ext };
        assert_eq!(update_for(&older).unwrap().map(|r| r.tag), Some(release.tag));
    }

    #[test]
    fn a_folder_is_an_extension_with_its_state_and_binary() {
        let root = tempfile::tempdir().unwrap();
        let make = |name: &str, state: &str, binary: bool| {
            let dir = root.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("state.json"), state).unwrap();
            if binary {
                std::fs::write(dir.join(if cfg!(windows) { format!("{name}.exe") } else { name.into() }), "").unwrap();
            }
        };
        make("demo", r#"{"name":"demo","description":"A demo","port":5000,"token":"t","start":"shepherd"}"#, true);
        make("plain", r#"{"port":5001,"token":"u"}"#, true);
        make("half", r#"{"port":5002,"token":"v"}"#, false);
        make("broken", "{", true);
        let found = scan(root.path());
        assert_eq!(found.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["demo", "plain"]);
        assert!(found[0].runs_with_shepherd());
        assert_eq!(found[1].start, "login");
        assert_eq!(found[0].url(), "http://127.0.0.1:5000/?token=t");
    }

    #[cfg(unix)]
    #[test]
    fn a_serve_restarts_when_its_binary_changes_and_after_it_exits_holds_when_stopped_and_stops_when_removed() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("sleepy");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("state.json"), r#"{"port":1,"token":"t","start":"shepherd"}"#).unwrap();
        let binary = dir.join("sleepy");
        // paddock's `uninstall` takes the binary away and leaves the folder
        std::fs::write(&binary, "#!/bin/sh\n[ \"$1\" = uninstall ] && exec rm \"$0\"\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let list = scan(root.path());
        let pid = || RUNS.lock().unwrap().get("sleepy").and_then(|r| r.child.as_ref().map(Child::id));
        supervise(&list);
        let first = pid().expect("started");
        supervise(&list);
        assert_eq!(pid(), Some(first), "left running");
        // installed again: a newer binary
        let later = SystemTime::now() + Duration::from_secs(5);
        std::fs::File::options().write(true).open(&binary).unwrap().set_modified(later).unwrap();
        supervise(&list);
        let second = pid().expect("restarted");
        assert_ne!(second, first);
        // it dies: started again once the pause is over, not before
        std::process::Command::new("kill").arg(second.to_string()).status().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        supervise(&list);
        assert_eq!(pid(), None, "waits");
        RUNS.lock().unwrap().get_mut("sleepy").unwrap().retry_at = Instant::now();
        supervise(&list);
        assert!(pid().is_some_and(|p| p != second), "started again");
        // stopped by hand: not started again until it is started
        stop_server(&list[0]).unwrap();
        supervise(&list);
        assert_eq!(pid(), None, "held");
        // its server never answers (port 1), so starting it waits it out and says so, but it runs
        assert!(start_server(&list[0]).is_err());
        assert!(pid().is_some(), "started");
        // removed: its serve stopped first, and it is no longer listed
        let third = pid().unwrap();
        uninstall(&list[0]).unwrap();
        assert!(RUNS.lock().unwrap().is_empty());
        assert!(!std::process::Command::new("kill").args(["-0", &third.to_string()]).status().unwrap().success(), "stopped");
        assert!(scan(root.path()).is_empty());
        assert!(dir.join("state.json").exists(), "its data stays");
    }

    #[test]
    fn the_theme_script_sets_every_variable_paddock_reads() {
        let theme = Theme::for_appearance(gpui::WindowAppearance::Dark);
        let script = theme_script(&theme, "http://127.0.0.1:5000");
        for k in ["--background", "--on-accent", "--switch-off", "--spinner-3", "--font-size", "--scrollbar"] {
            assert!(script.contains(&format!("\"{k}\"")), "{k}");
        }
        assert!(script.contains("\"--shell\":\"#0d1017\""));
        assert!(script.contains("colorScheme = 'dark'"));
        assert!(script.contains("!== \"http://127.0.0.1:5000\""));
    }
}
