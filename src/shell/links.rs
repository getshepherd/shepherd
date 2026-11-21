//! `shepherd://` links, from Raycast, Alfred, a chat or a note, and the `SHEPHERD_PANEL` names they share:
//!
//! - `shepherd://workspace/<id, number or name>` shows that workspace's active tab
//! - `shepherd://tab/<tab id>`, `shepherd://pane/<pane id>` show that tab, the pane's tab
//! - `shepherd://open?path=<absolute path>` shows the workspace whose folder holds the path, and the file (or the
//!   folder in the Files tree) in its right panel
//! - `shepherd://panel/<name>` opens a panel as `SHEPHERD_PANEL=<name>` does (`agents`, `diffs`, `review:checks`, …)

use gpui::{Context, Window};

use super::Shell;

#[derive(Debug, PartialEq)]
pub enum Link {
    Workspace(String),
    Tab(String),
    Pane(String),
    Open(String),
    Panel(String),
}

impl Link {
    pub fn parse(url: &str) -> Option<Link> {
        let rest = url.strip_prefix("shepherd://")?;
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let (kind, arg) = path.split_once('/').unwrap_or((path, ""));
        let arg = decode(arg.trim_end_matches('/'));
        let nonempty = |s: String| (!s.is_empty()).then_some(s);
        match kind {
            "workspace" => nonempty(arg).map(Link::Workspace),
            "tab" => nonempty(arg).map(Link::Tab),
            "pane" => nonempty(arg).map(Link::Pane),
            "panel" => nonempty(arg).map(Link::Panel),
            "open" => {
                let path = query.split('&').find_map(|kv| kv.strip_prefix("path="))?;
                nonempty(decode(&path.replace('+', " "))).map(Link::Open)
            }
            _ => None,
        }
    }
}

fn decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()
}

impl Shell {
    /// Follows `url`; false while there is no snapshot yet to find its target in (a link that launched the app),
    /// so the caller asks again a moment later. A link to nothing that exists is told in a toast.
    pub fn follow_link(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(link) = Link::parse(url) else {
            crate::ui::toast::error(cx, format!("Not a Shepherd link: {url}"));
            return true;
        };
        if let Link::Panel(name) = &link {
            self.open_panel(name, window, cx);
            return true;
        }
        let Some(s) = self.snapshot.as_ref() else { return false };
        let active_tab = |w: &crate::model::Workspace| w.active_tab_id.clone().or_else(|| s.tabs_of(&w.workspace_id).first().map(|t| t.tab_id.clone()));
        let mut file = None;
        let tab = match &link {
            Link::Workspace(key) => s
                .workspaces
                .iter()
                .find(|w| &w.workspace_id == key || w.number.to_string() == *key)
                .or_else(|| s.workspaces.iter().find(|w| w.label.eq_ignore_ascii_case(key)))
                .and_then(active_tab),
            Link::Tab(id) => s.tabs.iter().find(|t| &t.tab_id == id).map(|t| t.tab_id.clone()),
            Link::Pane(id) => s.panes.iter().find(|p| &p.pane_id == id).map(|p| p.tab_id.clone()),
            Link::Open(path) => {
                // the deepest workspace folder holding the path: a worktree's before the repo it sits in
                let found = s
                    .workspaces
                    .iter()
                    .filter_map(|w| Some((w, super::sidebar::workspace_dir(w, Some(s))?)))
                    .filter(|(_, dir)| std::path::Path::new(path).starts_with(dir))
                    .max_by_key(|(_, dir)| dir.len());
                found.and_then(|(w, dir)| {
                    let relative = std::path::Path::new(path).strip_prefix(&dir).ok()?.to_string_lossy().into_owned();
                    let kind = if std::path::Path::new(path).is_dir() { "files" } else { "file" };
                    file = (!relative.is_empty()).then(|| format!("{kind}:{relative}"));
                    active_tab(w)
                })
            }
            Link::Panel(_) => unreachable!(),
        };
        let Some(tab) = tab else {
            crate::ui::toast::error(cx, format!("Nothing in this session for {url}"));
            return true;
        };
        self.select_tab(Some(tab));
        cx.notify();
        if let Some(which) = file {
            self.show_files(&which, window, cx);
        }
        true
    }

    /// Opens the panel `SHEPHERD_PANEL` and `shepherd://panel/` name (main.rs lists the names).
    pub fn open_panel(&mut self, which: &str, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::command_panel::Kind;
        if which.starts_with("device") {
            return self.show_devices(which, cx);
        }
        // `review:timeline` opens the review panel on its Timeline tab, `review:menu` with its "…" menu open
        if let Some(tab) = which.strip_prefix("review") {
            self.toggle_review(window, cx);
            return self.show_review_tab(tab.trim_start_matches(':'), window, cx);
        }
        match which {
            "browser" => self.open_blank_tab(None, window, cx),
            "browser-agent" => self.show_agent_driving(window, cx),
            "browser-menu" => {
                self.open_blank_tab(None, window, cx);
                self.open_browser_menu(cx);
            }
            tab if tab.starts_with("browser:") => self.open_blank_tab(Some(tab["browser:".len()..].to_string()), window, cx),
            "diffs" => self.toggle_diff(window, cx),
            ext if ext.starts_with("extension:") => self.add_extension_surface(&ext["extension:".len()..], window, cx),
            files if files.starts_with("file") => self.show_files(files, window, cx),
            "usage" => self.open_usage_menu(window, cx),
            "projects" => self.open_project_menu(cx),
            "rename" => self.rename_shown_workspace(window, cx),
            "agents" => self.toggle_command_panel(Kind::Agents, window, cx),
            "workspaces" => self.toggle_command_panel(Kind::Workspaces, window, cx),
            "sessions" => self.toggle_command_panel(Kind::Sessions, window, cx),
            _ => self.toggle_command_panel(Kind::Commands, window, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Link;

    #[test]
    fn parses_links() {
        assert_eq!(Link::parse("shepherd://workspace/my%20repo/"), Some(Link::Workspace("my repo".into())));
        assert_eq!(Link::parse("shepherd://pane/w1:p2"), Some(Link::Pane("w1:p2".into())));
        assert_eq!(Link::parse("shepherd://panel/review:checks"), Some(Link::Panel("review:checks".into())));
        assert_eq!(Link::parse("shepherd://open?x=1&path=%2FUsers%2Fme%2Fa+b.rs"), Some(Link::Open("/Users/me/a b.rs".into())));
        assert_eq!(Link::parse("shepherd://tab/"), None);
        assert_eq!(Link::parse("shepherd://open"), None);
        assert_eq!(Link::parse("https://example.com"), None);
    }
}
