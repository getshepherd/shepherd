//! The `shepherd` agent skill: how an agent in a pane starts, follows and cleans up after other agents on the backend
//! it runs on. The guide lives in this binary (`assets/agent-skill`) and `shepherd skill` prints it for the terminal's
//! backend, so it changes with Shepherd. What goes into the agents' skill folders, once the user turns it on
//! (Settings → Backend), is only a SKILL.md that says to run that command.
use std::io;
use std::path::{Path, PathBuf};

const GUIDE: &str = include_str!("../assets/agent-skill/guide.md");
const WORKTREES: &str = include_str!("../assets/agent-skill/worktrees.md");
const BACKENDS: [(&str, &str); 4] = [
    ("herdr", include_str!("../assets/agent-skill/herdr.md")),
    ("rex", include_str!("../assets/agent-skill/rex.md")),
    ("hc", include_str!("../assets/agent-skill/hc.md")),
    ("tmux", include_str!("../assets/agent-skill/tmux.md")),
];

/// The line in SKILL.md that says the folder is Shepherd's: one without it is the user's, and stays untouched.
const MARK: &str = "<!-- Written by Shepherd";

/// The backend a terminal runs on, from its environment. A multiplexer started inside another inherits the outer one's
/// variables, so the inner ones are asked first: hc and herdr run in a Rex or tmux terminal, not the other way round.
fn detect(var: impl Fn(&str) -> Option<String>) -> Option<&'static str> {
    let set = |name: &str| var(name).is_some_and(|v| !v.is_empty());
    if set("HC_TERMINAL_ID") {
        Some("hc")
    } else if var("HERDR_ENV").as_deref() == Some("1") {
        Some("herdr")
    } else if set("TMUX") {
        Some("tmux")
    } else if set("REX_BLOCK") {
        Some("rex")
    } else {
        None
    }
}

/// Shepherd's app id, which macOS hands down to every process the app starts: the backend server Shepherd started,
/// and so each shell and agent in its panes.
const APP_ID: &str = "dev.freethinkel.shepherd";

/// Why this agent is not one the skill is for, if it isn't: the skill is for an agent in a terminal Shepherd shows.
/// - Claude Code another app runs (`claude -p` from an SDK) says so in its entrypoint, interactive Claude Code says
///   `cli`. Such an app inherits a pane's variables when started from one, so the backend's alone don't tell.
/// - No backend: a terminal outside them all (Ghostty, an editor's).
/// - On macOS, herdr or tmux whose server another app started: the herdr TUI in Ghostty, a tmux of the user's own.
///   Rex and hc terminals belong to their own apps, so there the app says nothing.
// ponytail: a herdr server the TUI started that Shepherd attached to later counts as outside; a Codex another app runs
// (`codex exec`) says nothing of it and still gets the guide
fn not_for_this_agent(var: impl Fn(&str) -> Option<String>, backend: Option<&str>) -> Option<&'static str> {
    let outside = var("CLAUDE_CODE_ENTRYPOINT").is_some_and(|e| e != "cli")
        || backend.is_none()
        || (cfg!(target_os = "macos")
            && matches!(backend, Some("herdr" | "tmux"))
            && var("__CFBundleIdentifier").is_some_and(|id| id != APP_ID));
    outside.then_some(
        "This agent doesn't run in a terminal of Shepherd, so the shepherd skill isn't for it. Don't use it or a terminal \
         multiplexer's CLI for subagents; do the work with your own tools.",
    )
}

/// The guide for `backend`.
fn guide(name: &str) -> Result<String, String> {
    let (_, text) = BACKENDS.iter().find(|(n, _)| *n == name).ok_or_else(|| format!("no backend {name}: herdr, rex, hc or tmux"))?;
    // herdr makes worktrees itself, the others leave them to git
    Ok(if name == "herdr" { format!("{GUIDE}\n{text}") } else { format!("{GUIDE}\n{text}\n{WORKTREES}") })
}

/// The guide's last section: the extensions that have a skill, each asked for it (`extensions::skills`), and how to
/// read one. Nothing when none has.
fn extensions_section(skills: &[(String, String)]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let list: String = skills.iter().map(|(name, skill)| format!("- `{name}`: {}\n", crate::extensions::skill_description(skill))).collect();
    format!(
        "\n# Extensions\n\nThe user's Shepherd extensions: local tools, each with a page the user may have open in Shepherd and a CLI for \
         you. Before using one, read its skill: run this command again with the extension's name after `skill`.\n\n{list}"
    )
}

/// `shepherd skill [backend | extension]`: prints the guide, for this terminal's backend unless one is named, with the
/// extensions that have skills; or an extension's skill.
// ponytail: a release build on Windows has no console, so this prints nothing there, as `hooks` does
pub fn cli(args: &[String]) -> i32 {
    let named = args.first().map(String::as_str);
    let var = |name: &str| std::env::var(name).ok();
    // a backend named here is the user's word for it, from a terminal the variables don't tell
    let backend = named.filter(|n| BACKENDS.iter().any(|(b, _)| b == n)).or_else(|| detect(var));
    if let Some(why) = not_for_this_agent(var, backend) {
        println!("{why}");
        return 0;
    }
    let skills = crate::extensions::skills(&crate::extensions::root());
    if let Some((_, skill)) = named.filter(|n| !BACKENDS.iter().any(|(b, _)| b == n)).and_then(|n| skills.iter().find(|(name, _)| name == n)) {
        // an extension's binary lives in its own folder, not on PATH, so `tasks …` as its skill writes it would not run
        let binary = crate::extensions::scan(&crate::extensions::root()).into_iter().find(|e| Some(e.name.as_str()) == named).map(|e| e.binary);
        if let Some(binary) = binary {
            let name = named.unwrap_or_default();
            println!("`{name}` below is `{}`, which isn't on PATH: run it by that path.\n", binary.display());
        }
        println!("{skill}");
        return 0;
    }
    let Some(backend) = named.or(backend) else { return 0 };
    match guide(backend) {
        Ok(text) => {
            print!("{text}{}", extensions_section(&skills));
            0
        }
        Err(e) => {
            eprintln!("shepherd skill: {e}");
            1
        }
    }
}

/// The SKILL.md each folder gets: run `exe skill`. Its description names the extensions installed, since an agent
/// loads a skill by its description: one that opens a PR has to recognise `tasks` there, or it never reads its skill.
fn stub(exe: &Path, extensions: &[(String, String)]) -> String {
    let exe = exe.to_string_lossy().replace('\'', r"'\''");
    let named = if extensions.is_empty() {
        String::new()
    } else {
        let list: Vec<String> = extensions.iter().map(|(name, what)| format!("`{name}` ({})", what.trim_end_matches('.'))).collect();
        format!(
            " Use it too whenever the work touches one of them (a task to track, a PR or MR you open, something to do later or on \
             a schedule), to learn its CLI: {}.",
            list.join(", ")
        )
    };
    format!(
        "---\nname: shepherd\ndescription: For an interactive agent in a terminal of the multiplexer Shepherd runs on (herdr, Rex, hc or tmux), \
         not one another app runs (`claude -p`, an SDK). Subagents as interactive `claude` or `codex` in their own terminals there, where the \
         user sees each one and can take it over. Use whenever work goes to another agent: a new agent the \
         user asks for, a subagent, a review, research, a parallel subtask, a handoff to its own branch, or a skill that asks for subagents. \
         Also drives Shepherd's own browser (the tabs in a workspace's right panel and the pages in its sidebar) from a pane: open and close tabs, \
         read a page and click, fill, press keys and wait in it, run JavaScript, read the console and requests, set the viewport, take \
         screenshots; use it instead of other browser tools \
         whenever the page is one the user sees in Shepherd or should see there. Also lists the user's Shepherd extensions (local tools with a \
         page in Shepherd and a CLI) and their skills.{named}\n---\n\
         {HEAD}\n\n\
         Run `'{exe}' skill` and follow the guide it prints. It comes from the installed Shepherd and gives the commands of the backend this \
         terminal runs on, and the extensions you can use.\n"
    )
}

/// The line after the front matter that says the file is Shepherd's.
const HEAD: &str = "<!-- Written by Shepherd (Settings → Backend → Agent skill), which rewrites this file: edits here are lost. -->";

/// The SKILL.md a herdr machine gets: no Shepherd binary there to run, so the herdr guide itself.
fn remote_skill() -> String {
    let guide = guide("herdr").expect("herdr has a guide");
    format!(
        "---\nname: shepherd\ndescription: Subagents as interactive `claude` or `codex` in their own herdr panes, where the user sees each one \
         and can take it over. Use whenever work goes to another agent: a new agent the user asks for, a subagent, a review, research, a parallel \
         subtask, a handoff to its own branch, or a skill that asks for subagents.\n---\n{HEAD}\n\n{guide}"
    )
}

/// The script that puts [`remote_skill`] into a machine's skill folders, or takes it out (`on` false), leaving a
/// `shepherd` skill the user made there.
fn remote_script(on: bool) -> String {
    let q = crate::herdr::quote;
    let write = if on { format!("mkdir -p \"$d\"; printf %s {} > \"$d/SKILL.md\"", q(&remote_skill())) } else { ":".into() };
    format!(
        "for d in \"$HOME/.claude/skills/shepherd\" \"$HOME/.agents/skills/shepherd\"; do\n\
         if [ -e \"$d\" ] && ! grep -qF {mark} \"$d/SKILL.md\" 2>/dev/null; then continue; fi\n\
         rm -rf \"$d\"; {write}\ndone\n",
        mark = q(MARK)
    )
}

/// Brings the herdr machine at `target` in line with this Mac: the skill there when it is on here, gone when off.
/// In the background; a machine out of reach keeps what it had.
pub fn sync_machine(target: String) {
    let on = installed();
    std::thread::spawn(move || match crate::herdr::ssh(&target, &remote_script(on)) {
        Ok(out) if !out.status.success() => log::warn!("agent skill on {target}: {}", String::from_utf8_lossy(&out.stderr).trim()),
        Err(e) => log::warn!("agent skill on {target}: {e}"),
        Ok(_) => {}
    });
}

/// The Claude Code plugin that holds the skill for Claude Code. Not `~/.claude/skills`, which every Claude Code reads:
/// Claude Code loads a plugin named in `CLAUDE_CODE_PLUGIN_DIRS`, which Shepherd sets for the servers it starts
/// ([`export_plugin_dir`]), so only an agent in one of their terminals has the skill.
fn plugin_dir() -> PathBuf {
    crate::settings::config_dir().join("agent-plugin")
}

/// Adds [`plugin_dir`] to this process's `CLAUDE_CODE_PLUGIN_DIRS`, for the backend servers it starts, and their
/// terminals, to inherit. Whether the skill is on or not: Claude Code passes over a folder that isn't there.
// ponytail: a server Shepherd didn't start (the herdr TUI's, one from before this) lacks it until restarted from here
pub fn export_plugin_dir() {
    let mut dirs: Vec<PathBuf> = std::env::var_os("CLAUDE_CODE_PLUGIN_DIRS").map(|v| std::env::split_paths(&v).collect()).unwrap_or_default();
    let ours = plugin_dir();
    if dirs.contains(&ours) {
        return;
    }
    dirs.push(ours);
    if let Ok(joined) = std::env::join_paths(dirs) {
        // SAFETY: called first thing in `main`, before any other thread is started
        unsafe { std::env::set_var("CLAUDE_CODE_PLUGIN_DIRS", joined) };
    }
}

/// The skill in Shepherd's plugin, for Claude Code, and in `~/.agents/skills`, which Codex and Pi read.
// ponytail: Codex has no folder of its own per process, so every Codex has it; `shepherd skill` turns away one outside
fn dirs(home: &Path, plugin: &Path) -> [PathBuf; 2] {
    [plugin.join("skills/shepherd"), home.join(".agents/skills/shepherd")]
}

fn ours(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("SKILL.md")).is_ok_and(|s| s.contains(MARK))
}

/// Whether Shepherd's skill is in any of the folders.
pub fn installed() -> bool {
    dirs(&crate::settings::home(), &plugin_dir()).iter().any(|d| ours(d))
}

/// Writes the skill into each folder, pointing at this binary, or takes it out, and so on the herdr machine shown. A
/// `shepherd` skill the user made is left as it is.
pub fn set(on: bool) -> io::Result<()> {
    set_in(&crate::settings::home(), &plugin_dir(), on.then(std::env::current_exe).transpose()?.as_deref())?;
    if let Some(target) = crate::herdr::shown_machine() {
        sync_machine(target);
    }
    Ok(())
}

/// `exe`: the binary to point at, `None` to take the skill out.
fn set_in(home: &Path, plugin: &Path, exe: Option<&Path>) -> io::Result<()> {
    // ponytail: every extension by its own description, those without a skill too; ask each for its skill (a process
    // each) if one without turns up
    let extensions: Vec<(String, String)> = crate::extensions::scan(&crate::extensions::root()).into_iter().map(|e| (e.name, e.description)).collect();
    // where an older Shepherd put it for Claude Code, which every Claude Code read
    let legacy = home.join(".claude/skills/shepherd");
    if ours(&legacy) {
        std::fs::remove_dir_all(&legacy)?;
    }
    if exe.is_some() {
        std::fs::create_dir_all(plugin.join(".claude-plugin"))?;
        std::fs::write(plugin.join(".claude-plugin/plugin.json"), "{\"name\": \"shepherd\", \"description\": \"Shepherd's skill for its agents\"}\n")?;
    }
    for dir in dirs(home, plugin) {
        if dir.exists() && !ours(&dir) {
            continue;
        }
        // the whole folder, so a skill an older Shepherd wrote with more files leaves none behind
        match std::fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        if let Some(exe) = exe {
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("SKILL.md"), stub(exe, &extensions))?;
        }
    }
    Ok(())
}

/// On start: an installed skill points at this binary, wherever the app moved. Not from a debug build, which runs
/// from a checkout's `target` that may be gone tomorrow.
pub fn refresh() -> io::Result<()> {
    if cfg!(debug_assertions) || !installed() { Ok(()) } else { set(true) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_names_the_extensions_in_its_description() {
        let description = |s: String| s.lines().find(|l| l.starts_with("description:")).unwrap().to_string();
        assert!(!description(stub(Path::new("/s"), &[])).contains("touches one of them"));
        let named = description(stub(Path::new("/s"), &[("tasks".into(), "A task tracker.".into())]));
        assert!(named.ends_with("to learn its CLI: `tasks` (A task tracker)."), "{named}");
    }

    #[test]
    fn extensions_with_skills_are_listed_by_their_description() {
        assert_eq!(extensions_section(&[]), "");
        let section = extensions_section(&[("prs".into(), "---\nname: prs\ndescription: Every PR of a task.\n---\n# prs".into())]);
        assert!(section.starts_with("\n# Extensions\n") && section.ends_with("- `prs`: Every PR of a task.\n"), "{section}");
    }

    #[test]
    fn only_an_agent_in_a_terminal_of_shepherd_gets_the_guide() {
        let env = |set: &'static [(&str, &str)]| move |name: &str| set.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string());
        let ours = env(&[("CLAUDE_CODE_ENTRYPOINT", "cli"), ("__CFBundleIdentifier", APP_ID)]);
        assert_eq!(not_for_this_agent(ours, Some("herdr")), None);
        assert_eq!(not_for_this_agent(env(&[]), Some("herdr")), None, "Codex, or Linux: no entrypoint, no app");
        assert!(not_for_this_agent(env(&[("CLAUDE_CODE_ENTRYPOINT", "commutator"), ("__CFBundleIdentifier", APP_ID)]), Some("herdr")).is_some());
        assert!(not_for_this_agent(env(&[("CLAUDE_CODE_ENTRYPOINT", "cli")]), None).is_some(), "Ghostty, no backend");
        let ghostty = env(&[("__CFBundleIdentifier", "com.mitchellh.ghostty")]);
        assert_eq!(not_for_this_agent(ghostty, Some("tmux")).is_some(), cfg!(target_os = "macos"), "a tmux of the user's own");
        assert_eq!(not_for_this_agent(env(&[("__CFBundleIdentifier", "com.rex.app")]), Some("rex")), None, "Rex's terminals are Rex's");
    }

    #[test]
    fn inner_backend_first() {
        let env = |set: &'static [(&str, &str)]| move |name: &str| set.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string());
        assert_eq!(detect(env(&[("REX_BLOCK", "b"), ("TMUX", "/tmp/s,1,0")])), Some("tmux"));
        assert_eq!(detect(env(&[("REX_BLOCK", "b"), ("HERDR_ENV", "1")])), Some("herdr"));
        assert_eq!(detect(env(&[("HERDR_ENV", "0"), ("REX_BLOCK", "b")])), Some("rex"));
        assert_eq!(detect(env(&[("HC_TERMINAL_ID", "t3"), ("HERDR_ENV", "1")])), Some("hc"));
        assert_eq!(detect(env(&[("TMUX", "")])), None);
    }

    #[test]
    fn every_backend_has_its_guide() {
        for (name, text) in BACKENDS {
            let out = guide(name).unwrap();
            assert!(out.starts_with(GUIDE) && out.contains(text));
            assert_eq!(out.contains(WORKTREES), name != "herdr", "{name}");
            assert!(!out.contains(".md)"), "{name}: a link to a file the agent doesn't have");
        }
        assert!(guide("zellij").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_machine_gets_the_herdr_guide_and_keeps_the_users_own() {
        let home = tempfile::tempdir().unwrap();
        let (claude, agents) = (home.path().join(".claude/skills/shepherd"), home.path().join(".agents/skills/shepherd"));
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("SKILL.md"), "mine").unwrap();
        // as `herdr::ssh` runs it: `sh -c '…'`
        let run = |on| {
            let script = format!("sh -c {}", crate::herdr::quote(&remote_script(on)));
            let out = std::process::Command::new("sh").arg("-c").arg(script).env("HOME", home.path()).output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        };
        run(true);
        assert_eq!(std::fs::read_to_string(claude.join("SKILL.md")).unwrap(), remote_skill());
        assert!(remote_skill().contains("herdr agent start") && remote_skill().contains(MARK));
        run(false);
        assert!(!claude.exists());
        assert_eq!(std::fs::read_to_string(agents.join("SKILL.md")).unwrap(), "mine");
    }

    #[test]
    fn the_skill_goes_to_the_plugin_and_leaves_the_users_own() {
        let home = tempfile::tempdir().unwrap();
        let plugin = home.path().join("plugin");
        let [claude, agents] = dirs(home.path(), &plugin);
        let legacy = home.path().join(".claude/skills/shepherd");
        std::fs::create_dir_all(legacy.join("backends")).unwrap();
        std::fs::write(legacy.join("SKILL.md"), format!("{MARK} once -->")).unwrap();
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("SKILL.md"), "---\nname: shepherd\n---\nmine").unwrap();

        set_in(home.path(), &plugin, Some(Path::new("/Applications/It's.app/shepherd"))).unwrap();
        let skill = std::fs::read_to_string(claude.join("SKILL.md")).unwrap();
        assert!(skill.starts_with("---\nname: shepherd\n") && skill.contains(r"Run `'/Applications/It'\''s.app/shepherd' skill`"), "{skill}");
        assert!(plugin.join(".claude-plugin/plugin.json").exists());
        assert!(!legacy.exists(), "out of the folder every Claude Code reads");
        assert_eq!(std::fs::read_to_string(agents.join("SKILL.md")).unwrap(), "---\nname: shepherd\n---\nmine", "not overwritten");

        set_in(home.path(), &plugin, None).unwrap();
        assert!(!claude.exists());
        assert!(agents.join("SKILL.md").exists(), "not removed");
    }
}
