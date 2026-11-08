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

/// The guide for `backend`, or for none known, with what to do then.
fn guide(backend: Option<&str>) -> Result<String, String> {
    let Some(name) = backend else {
        return Ok(format!(
            "{GUIDE}\n# This terminal's backend\n\nNone of HC_TERMINAL_ID, HERDR_ENV=1, TMUX or REX_BLOCK is set, so the backend is unknown. Say so and \
             ask the user which one it is, then run this command again with its name after `skill`: herdr, rex, hc or tmux.\n"
        ));
    };
    let (_, text) = BACKENDS.iter().find(|(n, _)| *n == name).ok_or_else(|| format!("no backend {name}: herdr, rex, hc or tmux"))?;
    // herdr makes worktrees itself, the others leave them to git
    Ok(if name == "herdr" { format!("{GUIDE}\n{text}") } else { format!("{GUIDE}\n{text}\n{WORKTREES}") })
}

/// `shepherd skill [backend]`: prints the guide, for this terminal's backend unless one is named.
// ponytail: a release build on Windows has no console, so this prints nothing there, as `hooks` does
pub fn cli(args: &[String]) -> i32 {
    let backend = args.first().map(String::as_str).or_else(|| detect(|name| std::env::var(name).ok()));
    match guide(backend) {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(e) => {
            eprintln!("shepherd skill: {e}");
            1
        }
    }
}

/// The SKILL.md each folder gets: run `exe skill`.
fn stub(exe: &Path) -> String {
    let exe = exe.to_string_lossy().replace('\'', r"'\''");
    format!(
        "---\nname: shepherd\ndescription: Subagents as interactive `claude` or `codex` in their own terminals of the multiplexer Shepherd runs on \
         (herdr, Rex, hc or tmux), where the user sees each one and can take it over. Use whenever work goes to another agent: a new agent the \
         user asks for, a subagent, a review, research, a parallel subtask, a handoff to its own branch, or a skill that asks for subagents.\n---\n\
         {MARK} (Settings → Backend → Agent skill), which rewrites this file: edits here are lost. -->\n\n\
         Run `'{exe}' skill` and follow the guide it prints. It comes from the installed Shepherd and gives the commands of the backend this \
         terminal runs on.\n"
    )
}

/// Claude Code's skills, and `~/.agents/skills`, which Codex and Pi read.
fn dirs(home: &Path) -> [PathBuf; 2] {
    [home.join(".claude/skills/shepherd"), home.join(".agents/skills/shepherd")]
}

fn ours(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("SKILL.md")).is_ok_and(|s| s.contains(MARK))
}

/// Whether Shepherd's skill is in any of the folders.
pub fn installed() -> bool {
    dirs(&crate::settings::home()).iter().any(|d| ours(d))
}

/// Writes the skill into each folder, pointing at this binary, or takes it out. A `shepherd` skill the user made is
/// left as it is.
pub fn set(on: bool) -> io::Result<()> {
    set_in(&crate::settings::home(), on.then(std::env::current_exe).transpose()?.as_deref())
}

/// `exe`: the binary to point at, `None` to take the skill out.
fn set_in(home: &Path, exe: Option<&Path>) -> io::Result<()> {
    for dir in dirs(home) {
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
            std::fs::write(dir.join("SKILL.md"), stub(exe))?;
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
            let out = guide(Some(name)).unwrap();
            assert!(out.starts_with(GUIDE) && out.contains(text));
            assert_eq!(out.contains(WORKTREES), name != "herdr", "{name}");
            assert!(!out.contains(".md)"), "{name}: a link to a file the agent doesn't have");
        }
        assert!(guide(Some("zellij")).is_err());
        assert!(guide(None).unwrap().contains("ask the user"));
    }

    #[test]
    fn leaves_the_users_own() {
        let home = tempfile::tempdir().unwrap();
        let [claude, agents] = dirs(home.path());
        std::fs::create_dir_all(claude.join("backends")).unwrap();
        std::fs::write(claude.join("SKILL.md"), format!("{MARK} once -->")).unwrap();
        std::fs::write(claude.join("backends/rex.md"), "old").unwrap();
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("SKILL.md"), "---\nname: shepherd\n---\nmine").unwrap();

        set_in(home.path(), Some(Path::new("/Applications/It's.app/shepherd"))).unwrap();
        let skill = std::fs::read_to_string(claude.join("SKILL.md")).unwrap();
        assert!(skill.starts_with("---\nname: shepherd\n") && skill.contains(r"Run `'/Applications/It'\''s.app/shepherd' skill`"), "{skill}");
        assert!(!claude.join("backends").exists(), "an older Shepherd's files gone");
        assert_eq!(std::fs::read_to_string(agents.join("SKILL.md")).unwrap(), "---\nname: shepherd\n---\nmine", "not overwritten");

        set_in(home.path(), None).unwrap();
        assert!(!claude.exists());
        assert!(agents.join("SKILL.md").exists(), "not removed");
    }
}
