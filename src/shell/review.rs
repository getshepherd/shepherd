//! The review panel on the right, after Superconductor's: the shown pane's branch's PR or MR (title, state, author,
//! description) and its checks or pipeline jobs, each opening in the browser.
use super::*;
use gpui::AnyElement;
use crate::ui::review::{Check, CheckState, Request, Review, act, check_icon, duration, markdown, read_review, retry_failed};
use crate::ui::statusbar::{branch_of, cached, fetching, pane_dir, refetch};

mod parts;
mod toolbar;
use parts::*;
pub(super) use parts::pr_tone;

/// The space between the review body's sections.
const SECTION: f32 = 16.;
/// How often gh or glab are asked again about a branch, as the status bar asks about its PR.
pub(super) const REVIEW_TTL: Duration = Duration::from_secs(120);
/// How often while one of its checks runs or waits to.
pub(super) const REVIEW_POLL: Duration = Duration::from_secs(10);
/// Past this a swipe over the checks pages when the fingers lift; short of it, the page springs back.
const PAGE_COMMIT: f32 = 60.;
/// How long after a retry, play or cancel before asking again: the host takes a moment to queue it.
const SETTLE: Duration = Duration::from_secs(3);

/// The shown pane's branch and its review, for the panel and the status bar; or the branch a `shepherd://review` link
/// picked, while that pane stays shown.
#[derive(Default)]
pub(super) struct ShownReview {
    pub dir: Option<String>,
    pub branch: Option<String>,
    /// The shown pane's own, for the Diffs and Files tabs, which a picked review doesn't move.
    pub pane_dir: Option<String>,
    pub pane_branch: Option<String>,
    /// The review cache's key: the directory, the branch and a picked PR's URL, a line apart.
    pub key: Option<String>,
    pub review: Option<Review>,
}

/// The review panel's tabs under a PR's head.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    Summary,
    Timeline,
    Code,
    Checks,
}

impl Tab {
    /// `summary`, `timeline`, `code`, `checks`.
    fn of_name(name: &str) -> Option<Tab> {
        [Tab::Summary, Tab::Timeline, Tab::Code, Tab::Checks].into_iter().find(|t| format!("{t:?}").eq_ignore_ascii_case(name))
    }
}

/// A merge the panel asks about before it runs.
#[derive(Clone)]
pub(super) struct MergeAsk {
    pub dir: String,
    pub key: String,
    pub gitlab: bool,
    pub number: u64,
    /// `#12 Fix it`.
    pub label: String,
    pub base: String,
    /// Merges once the pipeline passes, not now.
    pub auto: bool,
    pub squash: bool,
    pub remove_branch: bool,
}

impl Shell {
    pub fn toggle_review(&mut self, window: &Window, cx: &mut Context<Self>) {
        let seen = self.right_panel_shown(window);
        self.update_panel(cx, |p| p.toggle_pull_request(seen));
    }

    /// Shows the review panel's tab `name` (`summary`, `timeline`, `code`, `checks`), or with `menu` its "…" menu, for
    /// screenshots.
    pub fn show_review_tab(&mut self, name: &str, window: &Window, cx: &mut Context<Self>) {
        if name == "menu" {
            let size = window.viewport_size();
            let at = self.anchors.borrow().get("review-menu").map_or(gpui::point(size.width - px(16.), px(112.)), |b| b.bottom_right() + gpui::point(px(0.), px(4.)));
            self.review_menu = Some(crate::ui::Popup::new(at));
        } else if let Some(tab) = Tab::of_name(name) {
            self.review_tab = tab;
        }
        cx.notify();
    }

    /// Asks gh or glab again about the branch the panel shows while its checks run; called every `REVIEW_POLL`.
    pub(super) fn poll_review(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = self.review_poll.clone() {
            refetch::<Review>(&key, cx);
        }
    }

    /// The shown pane's branch's review, asked again while its checks run (also with the window in the background,
    /// so their end is noticed); when they stop, says how CI went.
    pub(super) fn shown_review(&mut self, snapshot: Option<&Snapshot>, window: &mut Window, cx: &mut Context<Self>) -> ShownReview {
        let pane = self.active_pane(window, cx);
        let pane_dir = pane_dir(snapshot, pane.as_deref());
        let pane_branch = pane_dir.as_deref().and_then(|d| branch_of(d, cx));
        // a picked review lasts while the pane it was picked over is the shown one
        if self.review_pick.as_ref().is_some_and(|(p, ..)| *p != pane) {
            self.review_pick = None;
        }
        let (dir, branch, url) = match &self.review_pick {
            Some((_, d, b, url)) => (Some(d.clone()), Some(b.clone()), url.as_ref()),
            None => (pane_dir.clone(), pane_branch.clone(), None),
        };
        let picked = url.map(|u| format!("\n{u}")).unwrap_or_default();
        let key = dir.clone().zip(branch.clone()).map(|(d, b)| format!("{d}\n{b}{picked}"));
        let ttl = if window.is_window_active() { REVIEW_TTL } else { Duration::MAX };
        let review = key.as_deref().and_then(|k| cached(k, ttl, read_review, cx));
        let running = review.as_ref().is_some_and(Review::running);
        self.review_poll = key.clone().filter(|_| running);
        if let (Some(k), Some(r)) = (&key, &review) {
            let last = self.review_last.replace((k.clone(), running));
            if !running && last.as_ref().is_some_and(|(l, ran)| l == k && *ran) {
                self.ci_finished(r, branch.as_deref().unwrap_or_default(), pane.as_deref(), window, cx);
            }
        }
        ShownReview { dir, branch, pane_dir, pane_branch, key, review }
    }

    /// Shows the PR at `url`, else of `branch`, in the checkout at `dir`, in the review panel: what a `shepherd://review` link in a
    /// page asks for (the pull-requests extension's rows), until another pane is shown. A sidebar page stays, with
    /// the panel beside it.
    pub(super) fn pick_review(&mut self, dir: String, branch: String, url: Option<String>, window: &Window, cx: &mut Context<Self>) {
        self.review_pick = Some((self.active_pane(window, cx), dir, branch, url));
        self.review_tab = Tab::Summary;
        self.update_panel(cx, |p| p.add(crate::right_panel::Surface::PullRequest));
        cx.notify();
    }

    /// A toast when the window is key, else a notification, saying how the shown branch's CI ended.
    fn ci_finished(&mut self, review: &Review, branch: &str, pane: Option<&str>, window: &Window, cx: &mut Context<Self>) {
        let (title, failed) = match review.ci() {
            Some(CheckState::Failed) => ("CI failed", true),
            Some(CheckState::Success) => ("CI passed", false),
            _ => ("CI finished", false),
        };
        if window.is_window_active() {
            let message = format!("{title} on {branch}");
            if failed {
                crate::ui::toast::error(cx, message);
            } else {
                crate::ui::toast::info(cx, message, None);
            }
        } else {
            let sound = SettingsGlobal::of(cx).settings.notify_sound;
            crate::notify::show(cx, pane.unwrap_or_default(), title.into(), branch.into(), sound, None, None);
        }
    }

    /// Runs `job` (CLI calls) off the main thread under `busy` (a spinner on its button); a failure's line goes to a
    /// toast. The review at `key` is asked again once the host has had a moment.
    pub(super) fn review_act(&mut self, busy: String, key: String, job: impl FnOnce() -> Result<(), String> + Send + 'static, cx: &mut Context<Self>) {
        self.review_act_then(busy, key, job, |_, _, _| {}, cx);
    }

    /// [`Self::review_act`], and `then` with what the job gave once it has gone through.
    fn review_act_then<T: Send + 'static>(
        &mut self,
        busy: String,
        key: String,
        job: impl FnOnce() -> Result<T, String> + Send + 'static,
        then: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        if !self.review_acting.insert(busy.clone()) {
            return;
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let done = cx.background_spawn(async move { job() }).await;
            if let Err(e) = &done {
                let e = e.clone();
                let _ = this.update(cx, |_, cx| crate::ui::toast::error(cx, e));
            } else {
                cx.background_executor().timer(SETTLE).await;
            }
            let _ = this.update(cx, |s, cx| {
                s.review_acting.remove(&busy);
                if let Ok(done) = done {
                    then(s, done, cx);
                }
                refetch::<Review>(&key, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The panel, and its edge: drag to resize, double-click for the default width.
    pub(super) fn render_review(&mut self, theme: &Theme, shown: &ShownReview, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let panel_width = self.right_panel_drawn(window);
        let (key, review, branch) = (shown.key.clone(), shown.review.clone(), shown.branch.clone());
        self.review_request = match &review {
            Some(Review::Found { request, .. }) => request.clone(),
            _ => None,
        };
        // the PR's timeline and diff are cached by its number, apart from the branch's review
        let pr_key = match (&shown.dir, &review) {
            (Some(dir), Some(Review::Found { request: Some(r), .. })) => Some(format!("{dir}\n{}\n{}", r.pr.gitlab, r.pr.number)),
            _ => None,
        };
        let busy = key.as_deref().is_some_and(|k| fetching::<Review>(k, cx))
            || pr_key.as_deref().is_some_and(|k| match self.review_tab {
                Tab::Timeline => fetching::<Timeline>(k, cx),
                Tab::Code => fetching::<PrDiff>(&self.diff_key(k, &shown.review), cx),
                _ => false,
            });

        let gitlab = matches!(&review, Some(Review::Found { request: Some(r), .. }) if r.pr.gitlab)
            || matches!(&review, Some(Review::Found { pipeline: Some(_), .. }) | Some(Review::NoTool("glab") | Review::NoLogin("glab", _)));
        let noun = if gitlab { "merge request" } else { "pull request" };
        let noun_title = match &review {
            Some(Review::Found { request: None, pipeline: Some(_), .. }) => "Pipeline",
            _ if gitlab => "Merge request",
            _ => "Pull request",
        };
        let header = crate::ui::surface_chrome::toolbar(theme)
            .child(div().flex_1().min_w_0().truncate().font_weight(gpui::FontWeight::MEDIUM).child(noun_title))
            .children(key.clone().map(|key| {
                if busy {
                    div().size(px(24.)).flex().items_center().justify_center().child(crate::ui::loader::mini_spinner(2., theme.text_faint, window)).into_any_element()
                } else {
                    titlebar_button(theme, "review-refresh", "icons/solar/refresh.svg")
                        .tooltip(tip("Refresh", None))
                        .on_click(cx.listener({
                            let pr_key = pr_key.clone();
                            move |this, _: &ClickEvent, _, cx| {
                                refetch::<Review>(&key, cx);
                                if let Some(k) = &pr_key {
                                    this.refetch_pr(k, cx);
                                }
                            }
                        }))
                        .into_any_element()
                }
            }));
        let (dir, key_or) = (shown.dir.clone().unwrap_or_default(), key.clone().unwrap_or_default());

        let mut footer = None;
        // where the "…" menu was clicked and the PR it is about
        let mut menu: Option<(gpui::Point<Pixels>, Request)> = None;
        // the PR's head and its tabs, above the body that scrolls
        let mut top = None;
        // a body that scrolls itself (the Code tab's diff), in place of the list
        let mut fill: Option<AnyElement> = None;
        // each item with the space above it
        let body: Vec<(f32, AnyElement)> = match (&key, review) {
            (None, _) => vec![(0., empty(theme, "No branch", "The focused pane isn't in a git repo."))],
            (Some(_), None) => vec![(0., loading(window))],
            (Some(_), Some(Review::NoRemote)) => vec![(0., empty(theme, "No remote", "This repo has no origin to ask about."))],
            (Some(_), Some(Review::NoTool(tool))) => vec![(0., empty(theme, format!("{tool} isn't installed"), format!("Install it to see the {noun} here: brew install {tool}")))],
            (Some(_), Some(Review::NoLogin(tool, host))) => {
                vec![(0., empty(theme, format!("{tool} isn't logged in"), format!("Run {tool} auth login --hostname {host} in a pane, then refresh.")))]
            }
            (Some(_), Some(Review::Failed(why))) => vec![(0., empty(theme, format!("Couldn't load the {noun}"), why))],
            (Some(_), Some(Review::Found { request, checks, pipeline })) => {
                let mut out = Vec::new();
                let checks_title = if gitlab { "Pipeline" } else { "Checks" };
                let ci = ci_line(&checks, pipeline.as_ref().map(|(_, s)| *s), gitlab);
                // the MR's pipelines, for the Pipeline tab's picker; an earlier one picked there is shown in the latest's place
                let past = match (&request, &pr_key) {
                    (Some(r), Some(k)) if r.pr.gitlab && self.review_tab == Tab::Checks => cached(k, REVIEW_TTL, crate::ui::review::read_pipelines, cx).and_then(Result::ok).unwrap_or_default(),
                    _ => Vec::new(),
                };
                let url = request.as_ref().map(|r| r.pr.url.clone()).unwrap_or_default();
                let picked = self.review_pipeline.as_ref().filter(|(u, _)| *u == url).and_then(|(_, iid)| past.iter().position(|p| p.iid == *iid)).filter(|i| *i > 0);
                let (shown_checks, shown_pipeline) = match picked.map(|i| &past[i]) {
                    Some(p) => {
                        let ttl = if matches!(p.state, CheckState::Running | CheckState::Pending) { Duration::from_secs(5) } else { Duration::MAX };
                        let jobs = cached(&format!("{dir}\n{}", p.iid), ttl, crate::ui::review::read_pipeline_jobs, cx).and_then(Result::ok).unwrap_or_default();
                        (jobs, Some((p.url.clone(), p.state)))
                    }
                    None => (checks.clone(), pipeline.clone()),
                };
                let picker = (past.len() > 1).then(|| self.pipeline_picker(theme, &url, &past, picked.unwrap_or(0), window, cx));
                let checks_el = (!shown_checks.is_empty() || shown_pipeline.is_some()).then(|| {
                    let this = cx.entity().downgrade();
                    let (dir, key) = (dir.clone(), key_or.clone());
                    let on_act: OnAct = Rc::new(move |busy, calls, cx| {
                        let dir = dir.clone();
                        let job = move || calls.iter().try_for_each(|c| act(&dir, c));
                        let _ = this.update(cx, |s, cx| s.review_act(busy, key.clone(), job, cx));
                    });
                    let pages = crate::ui::review::pages(&shown_checks);
                    let picked = self.review_page.as_ref().filter(|(k, _)| *k == key_or).and_then(|(_, name)| pages.iter().position(|(n, _)| n == name));
                    let page = picked.unwrap_or_else(|| crate::ui::review::default_page(&pages));
                    let this = cx.entity().downgrade();
                    let names: Vec<String> = pages.iter().map(|(n, _)| n.clone()).collect();
                    // the panel's whole width: pages slide in from its edge
                    let width = panel_width;
                    let on_page: OnPage = Rc::new({
                        let (names, key) = (names.clone(), key_or.clone());
                        move |to, cx| {
                            let _ = this.update(cx, |s, cx| s.page_to(&key, &names, page, to, width, cx));
                        }
                    });
                    let (offset, beside) = self.review_offset(&names, page, width, window);
                    // the log asked for, read again while its job runs
                    let shown_log = self.review_log.as_ref().filter(|(k, _)| *k == key_or).map(|(_, job)| job.clone());
                    let log = shown_log.and_then(|job| {
                        let c = shown_checks.iter().find(|c| c.job.as_deref() == Some(job.as_str()))?;
                        let log_key = format!("{}\n{gitlab}\n{job}\n{}", shown.dir.as_deref().unwrap_or_default(), c.repo.as_deref().unwrap_or_default());
                        let busy = matches!(c.state, CheckState::Running | CheckState::Pending);
                        let ttl = if busy { Duration::from_secs(5) } else { Duration::MAX };
                        Some((job, cached(&log_key, ttl, crate::ui::review::read_log, cx)))
                    });
                    let this = cx.entity().downgrade();
                    let on_log: Rc<dyn Fn(String, &mut App)> = Rc::new({
                        let key = key_or.clone();
                        move |job, cx| {
                            let _ = this.update(cx, |s, cx| {
                                let open = s.review_log.as_ref().is_some_and(|(k, j)| *k == key && *j == job);
                                s.review_log = (!open).then(|| (key.clone(), job));
                                cx.notify();
                            });
                        }
                    });
                    let paging = Paging { pages, page, on_page, offset, beside, width, log, on_log };
                    let section = checks_section(theme, checks_title, &shown_checks, shown_pipeline.clone(), gitlab, &self.review_acting, on_act, paging, window);
                    let key = key_or.clone();
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .on_scroll_wheel(cx.listener(move |this, e: &gpui::ScrollWheelEvent, _, cx| this.review_scroll(e, &key, &names, page, width, cx)))
                        .children(picker)
                        .child(section)
                        .into_any_element()
                });
                match &request {
                    Some(r) => {
                        let on_approve = cx.listener({
                            let (dir, key, number, approved) = (dir.clone(), key_or.clone(), r.pr.number, r.approved);
                            move |this, _: &ClickEvent, _, cx| {
                                let dir = dir.clone();
                                // an approved GitLab MR's button takes it back
                                let job = move || if approved { crate::ui::review::revoke(&dir, number) } else { crate::ui::review::approve(&dir, gitlab, number) };
                                this.review_act("approve".into(), key.clone(), job, cx)
                            }
                        });
                        let running = checks.iter().any(|c| matches!(c.state, CheckState::Running | CheckState::Pending))
                            || pipeline.as_ref().is_some_and(|(_, st)| matches!(st, CheckState::Running | CheckState::Pending));
                        let on_cancel = cx.listener({
                            let (dir, key, number) = (dir.clone(), key_or.clone(), r.pr.number);
                            move |this, _: &ClickEvent, _, cx| {
                                let dir = dir.clone();
                                this.review_act("merge".into(), key.clone(), move || crate::ui::review::cancel_auto_merge(&dir, gitlab, number), cx)
                            }
                        });
                        let on_merge = cx.listener({
                            let ask = MergeAsk {
                                dir: dir.clone(),
                                key: key_or.clone(),
                                gitlab,
                                number: r.pr.number,
                                label: format!("{}{} {}", if gitlab { "!" } else { "#" }, r.pr.number, r.title),
                                base: r.base.clone(),
                                auto: running,
                                squash: r.squash,
                                remove_branch: r.remove_branch,
                            };
                            move |this, _: &ClickEvent, _, cx| {
                                this.merge_confirm = Some(crate::ui::Popup::new(ask.clone()));
                                cx.notify();
                            }
                        });
                        let on_ready = cx.listener({
                            let (dir, key, number) = (dir.clone(), key_or.clone(), r.pr.number);
                            move |this, _: &ClickEvent, _, cx| {
                                let dir = dir.clone();
                                this.review_act("ready".into(), key.clone(), move || crate::ui::review::mark_ready(&dir, gitlab, number), cx)
                            }
                        });
                        footer = request_footer(theme, r, running, &self.review_acting, on_approve, on_merge, on_cancel, on_ready);
                        let tab = self.review_tab;
                        let pick = |to: Tab| -> Listener {
                            Box::new(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.review_tab = to;
                                cx.notify();
                            }))
                        };
                        let tabs = tab_bar(theme, tab, checks_title, ci, &pick, window);
                        let menu_open = self.review_menu.as_ref().is_some_and(|m| m.is_open());
                        // the menu hangs under the button, its right edge at the button's
                        let menu_button = crate::ui::trigger(
                            titlebar_button(theme, "review-menu", "icons/more-horizontal.svg"),
                            cx.listener(|this, e: &gpui::MouseDownEvent, _, cx| {
                                let at = this.anchors.borrow().get("review-menu").map_or(e.position, |b| b.bottom_right() + gpui::point(px(0.), px(4.)));
                                crate::ui::Popup::toggle(&mut this.review_menu, || at);
                                cx.notify();
                            }),
                        )
                        .relative()
                            .when(menu_open, |el| el.bg(theme.hover))
                            .tooltip(tip("More", None))
                            .child(super::widgets::anchor_probe(&self.anchors, "review-menu"))
                            .into_any_element();
                        top = Some(div().flex_none().flex().flex_col().child(request_head(theme, r, menu_button)).child(tabs).into_any_element());
                        menu = self.review_menu.as_ref().map(|m| (m.value, r.clone()));
                        let pr_key = pr_key.clone().unwrap_or_default();
                        match tab {
                            Tab::Summary => {
                                out.push((SECTION, summary(theme, r)));
                                if !r.body.trim().is_empty() {
                                    let toggle = cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.review_collapsed = !this.review_collapsed;
                                        cx.notify();
                                    });
                                    let look = pr_look(r, &dir, &[r.body.as_str()], cx);
                                    out.extend(description(theme, &r.body, self.review_collapsed, &look, toggle));
                                }
                            }
                            Tab::Timeline => out.extend(match cached(&pr_key, REVIEW_TTL, crate::ui::review::read_timeline, cx) {
                                None => vec![(SECTION, loading(window))],
                                Some(Err(why)) => vec![(SECTION, empty(theme, "Couldn't load the timeline", why))],
                                Some(Ok(events)) if events.is_empty() => vec![(SECTION, empty(theme, "Nothing yet", "No comments, reviews or commits."))],
                                Some(Ok(events)) => {
                                    let bodies: Vec<&str> = events.iter().map(|e| e.body.as_str()).collect();
                                    timeline(theme, &events, &pr_look(r, &dir, &bodies, cx))
                                }
                            }),
                            Tab::Code => {
                                let diff_key = self.diff_key(&pr_key, &shown.review);
                                let commits = cached(&pr_key, REVIEW_TTL, crate::ui::review::read_commits, cx).and_then(Result::ok).unwrap_or_default();
                                let commit = self.review_commit.as_ref().filter(|(u, _)| *u == r.pr.url).map(|(_, c)| c.clone());
                                let diff = cached(&diff_key, REVIEW_TTL, crate::ui::review::read_diff, cx);
                                let files = diff.as_ref().and_then(|d| d.as_ref().ok()).map(|d| d.files.as_slice()).unwrap_or_default();
                                let url = r.pr.url.clone();
                                let viewed: Vec<bool> = match commit {
                                    Some(_) => vec![false; files.len()],
                                    None => {
                                        let seen = self.viewed(&url);
                                        files.iter().map(|f| seen.contains(&crate::ui::review::fingerprint(f))).collect()
                                    }
                                };
                                let code = self.review_code(window, cx);
                                let toolbar = self.code_toolbar(theme, r, &commits, commit.as_deref(), files, &viewed, &code, cx);
                                let content = match &diff {
                                    None => loading(window),
                                    Some(Err(why)) => empty(theme, "Couldn't load the diff", why.clone()),
                                    Some(Ok(d)) if d.files.is_empty() => empty(theme, "No changes", format!("This {} changes no files.", if commit.is_some() { "commit" } else { noun })),
                                    Some(Ok(d)) => {
                                        let viewed = d.files.iter().zip(&viewed).filter(|(_, v)| **v).map(|(f, _)| f.path.clone()).collect();
                                        // the comments already on its lines; over one commit they'd sit on the wrong ones
                                        let posted = if commit.is_none() { cached(&pr_key, REVIEW_TTL, crate::ui::review::read_line_comments, cx).and_then(Result::ok).unwrap_or_default() } else { Vec::new() };
                                        let view = super::changes::PrView { url, diff: d.clone(), viewed, commentable: commit.is_none(), posted };
                                        code.update(cx, |c, cx| c.show_pr(&dir, view, cx));
                                        div().flex_1().min_h_0().child(code.clone()).into_any_element()
                                    }
                                };
                                // the diff scrolls itself, under its bar: not in the body's list
                                fill = Some(
                                    div()
                                        .id("review-code")
                                        .flex_1()
                                        .min_h_0()
                                        .flex()
                                        .flex_col()
                                        .capture_any_mouse_down(cx.listener(|this, _: &gpui::MouseDownEvent, window, cx| this.keys_to_review(window, cx)))
                                        .child(div().flex_none().px(px(12.)).py(px(8.)).child(toolbar))
                                        .child(content)
                                        .into_any_element(),
                                );
                            }
                            Tab::Checks => out.push((SECTION, checks_el.unwrap_or_else(|| empty(theme, format!("No {}", checks_title.to_lowercase()), "Nothing has run on this branch yet.")))),
                        }
                    }
                    None => {
                        out.push((SECTION, empty(theme, format!("No {noun}"), format!("{} has none yet.", branch.unwrap_or_default()))));
                        out.extend(checks_el.map(|el| (SECTION, el)));
                    }
                }
                out
            }
        };

        let main = match fill {
            Some(fill) => fill,
            // the app's scrollbar over the body's right edge, as the settings page has it
            None => div()
                .relative()
                .flex_1()
                .min_h_0()
                // one reset for the description's and the logs' selectable text, before either paints
                .child(crate::ui::markdown::frame_reset())
                .child(self.review_body(body, format!("{pr_key:?}\n{:?}\n{:?}\n{:?}", self.review_tab, self.review_commit, self.review_pipeline)))
                .child(crate::ui::scrollbar("review-scrollbar", &self.review_list, theme))
                .into_any_element(),
        };
        let notes = self.review_notes_bar(theme, &dir, &key_or, cx);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(header)
            .children(top)
            .child(main)
            .children(notes)
            .children(footer)
            .children(menu.and_then(|(at, r)| self.render_review_menu(theme, at, &r, pr_key.clone(), key.clone(), window, cx)))
            .into_any_element()
    }

    /// The body's items as a list, `for` naming what they show: a scroll lays out only the items in view, not a whole
    /// timeline's markdown each frame. Built for something else, it starts at the top; items come and go at its end.
    fn review_body(&mut self, body: Vec<(f32, AnyElement)>, built_for: String) -> gpui::List {
        let (list, count) = (&self.review_list, body.len());
        if built_for != self.review_list_for {
            self.review_list_for = built_for;
            list.reset(count);
        } else if list.item_count() != count {
            let had = list.item_count();
            list.splice(count.min(had)..had, count.saturating_sub(had));
        }
        // ponytail: each item is built once per render and handed over once; a list asking for one twice in a frame
        // (only an autoscroll does, nothing here asks for one) gets it empty
        let items = std::cell::RefCell::new(body.into_iter().map(Some).collect::<Vec<_>>());
        gpui::list(list.clone(), move |i, _, _| {
            let Some((space, item)) = items.borrow_mut().get_mut(i).and_then(Option::take) else { return div().into_any_element() };
            div().px(px(12.)).pt(px(if i == 0 { 12. } else { space })).when(i + 1 == count, |el| el.pb(px(12.))).child(item).into_any_element()
        })
        .size_full()
    }

    /// The Code tab's diff view, made the first time: the diffs' own, over the PR's patch.
    fn review_code(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<super::changes::Changes> {
        use super::changes::{Changes, ChangesEvent};
        if let Some(code) = &self.review_code {
            return code.clone();
        }
        let code = cx.new(Changes::for_pr);
        code.update(cx, |c, _| c.back = Some(self.focus.clone()));
        cx.subscribe_in(&code, window, |this, _, event: &ChangesEvent, window, cx| match event {
            ChangesEvent::Viewed { path } => this.flip_viewed(path, cx),
            ChangesEvent::Open { dir, root, path } => this.edit_diff_file(dir, root, path, cx),
            ChangesEvent::Preview { dir, root, path } => this.preview_diff_file(dir, root, path, window, cx),
            ChangesEvent::Discard { .. } | ChangesEvent::Send | ChangesEvent::ShowPr => {}
        })
        .detach();
        self.review_code = Some(code.clone());
        code
    }

    /// The Viewed box of the shown PR's file at `path`: marked, the file folds shut; taken back, it stays as it is.
    fn flip_viewed(&mut self, path: &str, cx: &mut Context<Self>) {
        let (Some(code), Some(url)) = (self.review_code.clone(), self.review_request.as_ref().map(|r| r.pr.url.clone())) else { return };
        let Some(file) = code.read(cx).files().and_then(|files| files.iter().find(|f| f.path == path).cloned()) else { return };
        let mark = crate::ui::review::fingerprint(&file);
        let seen = self.viewed_mut(&url);
        let now = seen.insert(mark.clone());
        if !now {
            seen.remove(&mark);
        }
        self.save_viewed(cx);
        if now {
            code.update(cx, |c, cx| c.fold_file(path, true, cx));
        }
        cx.notify();
    }

    /// A press in the diff or on the review's bar: the keyboard comes to the panel, so a comment's field keeps it (a
    /// pane takes it back on the next render otherwise), as the diffs' tab has it.
    fn keys_to_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.panel_focused() || !self.focus.is_focused(window) {
            self.focus_panel(true, window, cx);
        }
    }

    /// The comments waiting on the shown PR: the agent's and the review's.
    fn pr_comments(&self, cx: &App) -> (Vec<crate::changes::ReviewComment>, Vec<crate::changes::ReviewComment>) {
        let url = self.review_request.as_ref().map(|r| r.pr.url.as_str());
        let code = self.review_code.as_ref().filter(|c| url.is_some_and(|url| c.read(cx).shows_pr(url)));
        code.map(|c| c.read(cx).waiting_comments()).unwrap_or_default().into_iter().partition(|c| !c.review)
    }

    /// The comments for the agent go to it, and leave once sent.
    fn send_pr_comments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.review_acting.contains("send") {
            return;
        }
        // a card still open is a comment too
        if let Some(code) = &self.review_code {
            code.update(cx, |c, cx| c.commit_draft(cx));
        }
        let (agent_comments, _) = self.pr_comments(cx);
        let Some(code) = self.review_code.clone().filter(|_| !agent_comments.is_empty()) else { return };
        let Some(agent) = self.review_agent(window, cx) else { return crate::ui::toast::error(cx, "No agent here to send the comments to") };
        self.review_acting.insert("send".into());
        let name = agent.agent.clone().unwrap_or_default();
        let (backend, pane) = (self.backend.clone(), agent.pane_id);
        let prompt = crate::changes::comments_prompt(&agent_comments);
        let ids: Vec<u64> = agent_comments.iter().map(|c| c.id).collect();
        self.ask(async move { backend.mux.send(pane, Some(prompt)).await }, cx, move |this, result, cx| {
            this.review_acting.remove("send");
            match result {
                Ok(_) => {
                    crate::ui::toast::info(cx, format!("Sent to {name}"), None);
                    code.update(cx, |c, cx| c.drop_comments(&ids, cx));
                }
                Err(e) => crate::ui::toast::error(cx, format!("Not sent to {name}: {e:#}")),
            }
        });
    }

    /// Sends the review: its comments, the summary and `event`; they leave once the host has them.
    fn submit_pr_review(&mut self, event: crate::ui::review::ReviewEvent, dir: String, key: String, cx: &mut Context<Self>) {
        use crate::changes::{CommentSide, LineKind};
        let (_, comments) = self.pr_comments(cx);
        let (Some(code), Some(r)) = (self.review_code.clone(), self.review_request.clone()) else { return };
        let (url, gitlab, number) = (r.pr.url.clone(), r.pr.gitlab, r.pr.number);
        let pr_key = format!("{dir}\n{gitlab}\n{number}");
        // the whole PR's diff, not the commit the Code tab may be showing: the comments are on the PR's lines
        let Some(Ok(diff)) = cached(&format!("{pr_key}\n"), REVIEW_TTL, crate::ui::review::read_diff, cx) else {
            return crate::ui::toast::error(cx, "The PR's diff isn't loaded yet");
        };
        let files = &diff.files;
        // both of a line's numbers: GitLab places a note on an unchanged line by both
        let numbers = |c: &crate::changes::ReviewComment| {
            let lines = files.iter().filter(|f| f.path == c.path).flat_map(|f| f.hunks.iter().flat_map(|h| &h.lines));
            let found = lines.into_iter().find(|l| match c.side {
                CommentSide::Old => l.kind != LineKind::Add && l.old_no == Some(c.line),
                CommentSide::New => l.kind != LineKind::Del && l.new_no == Some(c.line),
            });
            found.map_or((None, None), |l| (l.old_no, l.new_no))
        };
        let notes: Vec<crate::ui::review::ReviewNote> = comments
            .iter()
            .map(|c| {
                let (old_no, new_no) = numbers(c);
                crate::ui::review::ReviewNote { comment: c.clone(), old_no, new_no }
            })
            .collect();
        let body = self.review_submit.as_ref().map(|input| input.read(cx).value().to_string()).unwrap_or_default();
        let ids: Vec<u64> = comments.iter().map(|c| c.id).collect();
        let job = move || crate::ui::review::submit_review(&dir, &url, gitlab, number, event, &body, &notes);
        self.review_act_then(
            "submit".into(),
            key,
            job,
            move |this, failed: Option<String>, cx| {
                this.review_submit = None;
                code.update(cx, |c, cx| c.drop_comments(&ids, cx));
                // they're the host's now: asked for again, they come back under their lines
                refetch::<LineComments>(&pr_key, cx);
                refetch::<Timeline>(&pr_key, cx);
                match failed {
                    Some(why) => crate::ui::toast::error(cx, format!("Review sent, but not all of it: {why}")),
                    None => crate::ui::toast::info(cx, "Review sent", None),
                }
            },
            cx,
        );
    }

    /// Over the footer while comments wait on the PR: how many go to the agent (and the send), how many are in the
    /// review (and its submit, a card with the summary and the verdicts).
    fn review_notes_bar(&mut self, theme: &Theme, dir: &str, key: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        use crate::ui::review::ReviewEvent;
        let (agent_comments, review_comments) = self.pr_comments(cx);
        if agent_comments.is_empty() && review_comments.is_empty() {
            self.review_submit = None;
            return None;
        }
        let r = self.review_request.clone()?;
        let sending = self.review_acting.contains("submit");
        let (hover, accent) = (theme.hover, theme.accent);
        let button = |id: &'static str, label: &'static str, primary: bool| {
            let hovered = accent.opacity(0.85);
            div()
                .id(id)
                .flex_none()
                .h(px(26.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .cursor_pointer()
                .when(primary, |el| el.bg(accent).text_color(gpui::white()).hover(move |s| s.bg(hovered)))
                .when(!primary, |el| el.border_1().border_color(theme.border).text_color(theme.text).hover(move |s| s.bg(hover)))
                .child(label)
        };
        let count = |n: usize, what: &str| format!("{n} comment{} {what}", if n == 1 { "" } else { "s" });
        let row = || div().flex().items_center().gap(px(8.)).child(div().flex_1());
        let agent_row = (!agent_comments.is_empty()).then(|| {
            row()
                .child(div().flex_none().text_color(theme.text_muted).child(count(agent_comments.len(), "for the agent")))
                .child(button("review-notes-send", "Send to agent", false).on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.send_pr_comments(window, cx))))
        });
        let review_row = (!review_comments.is_empty() && self.review_submit.is_none()).then(|| {
            row().child(div().flex_none().text_color(theme.text_muted).child(count(review_comments.len(), "in review"))).child(button("review-notes-submit", "Submit review", true).on_click(cx.listener(
                |this, _: &ClickEvent, window, cx| {
                    let theme = Theme::of(cx).clone();
                    let input = cx.new(|cx| gpui_base::input::TextareaState::new(window, cx).placeholder("Leave a summary (optional)"));
                    input.update(cx, |input, cx| {
                        input.set_editor_style(crate::ui::input_style(&theme));
                        input.focus(window, cx);
                    });
                    this.review_submit = Some(input);
                    cx.notify();
                },
            )))
        });
        let submit = self.review_submit.clone().filter(|_| !review_comments.is_empty()).map(|input| {
            let verdict = |id: &'static str, label: &'static str, event: ReviewEvent, primary: bool| {
                let (dir, key) = (dir.to_string(), key.to_string());
                button(id, if sending && primary { "Sending…" } else { label }, primary)
                    .when(sending, |el| el.opacity(0.5))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.submit_pr_review(event, dir.clone(), key.clone(), cx)))
            };
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(div().text_color(theme.text_muted).child(count(review_comments.len(), "in review")))
                .child(div().h(px(54.)).px(px(8.)).py(px(6.)).rounded(px(6.)).border_1().border_color(theme.border).bg(theme.card).child(gpui_base::input::Textarea::new(&input)))
                .child(
                    row()
                        .child(button("review-submit-cancel", "Cancel", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.review_submit = None;
                            cx.notify();
                        })))
                        .when(r.can_approve && !r.pr.gitlab, |el| el.child(verdict("review-submit-changes", "Request changes", ReviewEvent::RequestChanges, false)))
                        .when(r.can_approve, |el| el.child(verdict("review-submit-approve", "Approve", ReviewEvent::Approve, false)))
                        .child(verdict("review-submit-comment", "Comment", ReviewEvent::Comment, true)),
                )
        });
        Some(
            div()
                .id("review-notes")
                .flex_none()
                .px(px(12.))
                .py(px(8.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .capture_any_mouse_down(cx.listener(|this, _: &gpui::MouseDownEvent, window, cx| this.keys_to_review(window, cx)))
                .border_t_1()
                .border_color(theme.border)
                .text_size(px(12.))
                .children(agent_row)
                .children(review_row)
                .children(submit)
                .into_any_element(),
        )
    }

    /// The Code tab's diff's cache key: the PR's (`pr_key`), and the commit picked for it, if any.
    fn diff_key(&self, pr_key: &str, review: &Option<Review>) -> String {
        let url = match review {
            Some(Review::Found { request: Some(r), .. }) => r.pr.url.as_str(),
            _ => "",
        };
        let commit = self.review_commit.as_ref().filter(|(u, _)| u == url).map_or("", |(_, c)| c.as_str());
        format!("{pr_key}\n{commit}")
    }

    /// Asks again for what the PR's tabs show: its timeline, commits and diffs.
    fn refetch_pr(&self, pr_key: &str, cx: &mut App) {
        refetch::<Timeline>(pr_key, cx);
        refetch::<PrCommits>(pr_key, cx);
        refetch::<Pipelines>(pr_key, cx);
        if let Some((_, iid)) = &self.review_pipeline {
            let dir = pr_key.split('\n').next().unwrap_or_default();
            refetch::<PipelineJobs>(&format!("{dir}\n{iid}"), cx);
        }
        refetch::<LineComments>(pr_key, cx);
        refetch::<PrDiff>(&format!("{pr_key}\n"), cx);
        if let Some((_, sha)) = &self.review_commit {
            refetch::<PrDiff>(&format!("{pr_key}\n{sha}"), cx);
        }
    }

    /// The fingerprints of the files marked viewed on the PR at `url`.
    fn viewed(&mut self, url: &str) -> &std::collections::HashSet<String> {
        self.viewed_mut(url)
    }

    fn viewed_mut(&mut self, url: &str) -> &mut std::collections::HashSet<String> {
        let all = self.review_viewed.get_or_insert_with(|| std::fs::read(viewed_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default());
        all.entry(url.to_string()).or_default()
    }

    /// Writes the files marked viewed to the cache folder, off the main thread.
    fn save_viewed(&mut self, cx: &mut App) {
        let Some(all) = self.review_viewed.as_mut() else { return };
        all.retain(|_, seen| !seen.is_empty());
        let Ok(bytes) = serde_json::to_vec(all) else { return };
        cx.background_spawn(async move {
            let path = viewed_file();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, bytes);
        })
        .detach();
    }
}

/// A menu item's keys, at its right.
fn shortcut(theme: &Theme, keys: String) -> Div {
    div().flex_none().pl(px(16.)).h(px(18.)).flex().items_center().text_color(theme.text_faint).child(keys)
}

/// What the "…" menu gives the agent about `r`: the question's start, left to finish, and the requests to explain it
/// and to deal with its findings.
fn review_prompts(r: &Request) -> (String, String, String) {
    let (noun, cli) = if r.pr.gitlab { ("merge request", "glab mr") } else { ("pull request", "gh pr") };
    let number = format!("{}{}", if r.pr.gitlab { "!" } else { "#" }, r.pr.number);
    let which = format!("{noun} {number} \"{}\" ({})", r.title, r.pr.url);
    let ask = format!("About {which}: ");
    let explain = format!(
        "Explain {which}. Read it with `{cli} view {n}` and its diff with `{cli} diff {n}`, then walk me through the change: what it does, \
         in what order to read it, and which parts deserve a close look.",
        n = r.pr.number
    );
    let comments = if r.pr.gitlab { format!("`glab mr view {} --comments`", r.pr.number) } else { format!("`gh pr view {} --comments` and `gh api repos/{{owner}}/{{repo}}/pulls/{}/comments`", r.pr.number, r.pr.number) };
    let checks = if r.pr.gitlab { "`glab ci status`".to_string() } else { format!("`gh pr checks {}`", r.pr.number) };
    let fix = format!(
        "Deal with the review findings on {which}: read its comments ({comments}) and its failing checks ({checks}), \
         fix in this checkout what should be fixed, and tell me what you left and why."
    );
    (ask, explain, fix)
}

/// A click handler handed to a tab.
type Listener = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
/// The Timeline tab's cache entry: a PR's events, or why they couldn't be read.
type Timeline = Result<Vec<crate::ui::review::Event>, String>;
/// The Code tab's: a PR's files and whether the diff was cut short, or why it couldn't be read.
type PrDiff = Result<std::sync::Arc<crate::ui::review::PrDiff>, String>;
type LineComments = Result<Vec<crate::changes::ReviewComment>, String>;
/// The Code tab's commit picker's: a PR's commits, newest first, or why they couldn't be read.
type PrCommits = Result<Vec<(String, String)>, String>;
/// The Pipeline tab's picker's: an MR's pipelines, newest first; and an earlier one's jobs, by `dir\niid`.
type Pipelines = Result<Vec<crate::ui::review::PastPipeline>, String>;
type PipelineJobs = Result<Vec<Check>, String>;

/// Where files marked viewed are kept between runs.
fn viewed_file() -> std::path::PathBuf {
    crate::settings::cache_dir().join("review-viewed.json")
}

fn loading(window: &mut Window) -> AnyElement {
    div().pt(px(24.)).flex().justify_center().child(crate::ui::loader::spinner(2.5, window)).into_any_element()
}

fn empty(theme: &Theme, title: impl Into<SharedString>, hint: impl Into<SharedString>) -> AnyElement {
    div()
        .pt(px(24.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.))
        .text_center()
        // The site's dither texture, piloted here first: a band over the words in a faint theme colour, so it holds in
        // light themes as in dark ones.
        .child(crate::ui::dither::dither(px(36.), px(3.), theme.text_faint.opacity(0.45), crate::ui::dither::horizon))
        .child(div().text_color(theme.text).child(title.into()))
        .child(div().text_size(px(12.)).text_color(theme.text_muted).child(hint.into()))
        .into_any_element()
}

/// How a PR's text shows: its links open in a browser tab here, as the panel's own do (a link that isn't a web page
/// goes to the system); its images (in `bodies`) are fetched, with the CLI's token on the PR's host, and show once
/// they are, their alt text until then.
fn pr_look(r: &Request, dir: &str, bodies: &[&str], cx: &mut App) -> crate::ui::markdown::Look {
    let link: Rc<dyn Fn(&str, &mut Window, &mut App)> = Rc::new(|url, _, cx| {
        if url.starts_with("http://") || url.starts_with("https://") {
            super::browser::open_inside(url, cx);
        } else {
            crate::ui::open_outside(url, cx);
        }
    });
    let mut sources = Vec::new();
    for body in bodies.iter().filter(|b| b.contains("![") || b.contains("<img") || b.contains("<IMG")) {
        crate::ui::review::image_sources(&parsed(body), &mut sources);
    }
    let images: std::collections::HashMap<String, std::path::PathBuf> = sources
        .into_iter()
        .filter_map(|source| {
            let key = format!("{dir}\n{}\n{}\n{source}", r.pr.gitlab, r.pr.url);
            let path = cached(&key, Duration::MAX, crate::ui::review::read_image, cx)?.0?;
            Some((source, path))
        })
        .collect();
    let image: Option<Rc<dyn Fn(&str) -> Option<std::path::PathBuf>>> = (!images.is_empty()).then(|| Rc::new(move |source: &str| images.get(source).cloned()) as Rc<dyn Fn(&str) -> Option<std::path::PathBuf>>);
    let open_image: Rc<dyn Fn(&std::path::Path, &mut Window, &mut App)> = Rc::new(|path, window, cx| {
        let Some(shell) = cx.try_global::<super::browser::Links>().and_then(|l| l.0.upgrade()) else { return };
        let path = path.to_path_buf();
        shell.update(cx, |s, cx| s.preview_review_image(path, window, cx));
    });
    crate::ui::markdown::Look { link: Some(link), image, open_image: Some(open_image), ..Default::default() }
}

/// `body`'s blocks, parsed once while it stays around: a drag over the text redraws every move, and the timeline
/// shows a body per comment.
fn parsed(body: &str) -> Rc<Vec<crate::ui::review::Block>> {
    thread_local! {
        static SEEN: std::cell::RefCell<std::collections::HashMap<String, Rc<Vec<crate::ui::review::Block>>>> = Default::default();
    }
    SEEN.with(|seen| {
        let mut seen = seen.borrow_mut();
        if let Some(blocks) = seen.get(body) {
            return blocks.clone();
        }
        // a PR's worth, more or less; past that start over rather than keep every body ever shown
        if seen.len() > 256 {
            seen.clear();
        }
        let blocks = Rc::new(markdown(body));
        seen.insert(body.to_string(), blocks.clone());
        blocks
    })
}

/// The description under a heading that folds it away.
/// The description's head, then its markdown a block an item, each with the space above it.
fn description(theme: &Theme, body: &str, collapsed: bool, look: &crate::ui::markdown::Look, toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Vec<(f32, AnyElement)> {
    let text = theme.text;
    let head = div()
        .id("review-description")
        .flex()
        .items_center()
        .gap(px(4.))
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text_muted)
        .cursor_pointer()
        .hover(move |s| s.text_color(text))
        .child("Description")
        .child(gpui::svg().path(if collapsed { "icons/alt-arrow-right.svg" } else { "icons/alt-arrow-down.svg" }).size(px(12.)).text_color(theme.text_faint))
        .on_click(toggle)
        .into_any_element();
    let blocks = if collapsed { Vec::new() } else { crate::ui::markdown::render_with(&parsed(body), "review-md", theme, look) };
    let blocks = blocks.into_iter().enumerate().map(|(i, b)| (if i == 0 { 10. } else { 8. }, div().text_size(px(13.)).text_color(theme.text).child(b).into_any_element()));
    std::iter::once((SECTION, head)).chain(blocks).collect()
}

impl Shell {
    /// Pages the checks from `page` to `to` of `names` (the branch at `key`'s), the new page sliding in from the
    /// side it lies on, the old one going out the other.
    pub(super) fn page_to(&mut self, key: &str, names: &[String], page: usize, to: usize, width: f32, cx: &mut Context<Self>) {
        let Some(name) = names.get(to).filter(|_| to != page) else { return };
        self.review_page = Some((key.to_string(), name.clone()));
        self.review_spring = Some((if to > page { width } else { -width }, Instant::now(), names.get(page).cloned()));
        cx.notify();
    }

    /// One event of a scroll over the checks, as the sidebar takes a swipe between places: across, the page follows
    /// the fingers with its neighbour beside it, and when they lift far enough that one comes in; else it springs
    /// back. Momentum after the lift is ignored.
    fn review_scroll(&mut self, e: &gpui::ScrollWheelEvent, key: &str, names: &[String], page: usize, width: f32, cx: &mut Context<Self>) {
        use gpui::TouchPhase;
        if names.len() < 2 {
            return;
        }
        let gpui::ScrollDelta::Pixels(d) = e.delta else { return };
        let (dx, dy) = (f32::from(d.x), f32::from(d.y));
        match e.touch_phase {
            TouchPhase::Started => self.review_swipe = Some((None, 0.)),
            TouchPhase::Moved => {
                let Some((across, travel)) = self.review_swipe.as_mut() else { return };
                if across.is_none() && dx.hypot(dy) > 2. {
                    *across = Some(dx.abs() > dy.abs() * 1.5);
                }
                if *across == Some(true) {
                    *travel += dx;
                    self.review_spring = None;
                    cx.stop_propagation();
                    cx.notify();
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let Some((Some(true), travel)) = self.review_swipe.take() else { return };
                let offset = page_offset(travel, neighbour(names.len(), page, travel).is_some(), width);
                match neighbour(names.len(), page, travel).filter(|_| travel.abs() > PAGE_COMMIT) {
                    Some(to) => {
                        self.review_page = Some((key.to_string(), names[to].clone()));
                        // the new page carries on from where it was beside the old one
                        let from = if travel < 0. { width + offset } else { offset - width };
                        self.review_spring = Some((from, Instant::now(), names.get(page).cloned()));
                    }
                    None => self.review_spring = Some((offset, Instant::now(), None)),
                }
                cx.notify();
            }
        }
    }

    /// The shown page's offset now (under the fingers, springing home, else 0) and the page beside it with its
    /// offset: the one a swipe goes to, or the one just left.
    fn review_offset(&mut self, names: &[String], page: usize, width: f32, window: &mut Window) -> (f32, Option<(usize, f32)>) {
        if let Some((Some(true), travel)) = self.review_swipe {
            let near = neighbour(names.len(), page, travel);
            let offset = page_offset(travel, near.is_some(), width);
            return (offset, near.map(|n| (n, if travel < 0. { offset + width } else { offset - width })));
        }
        let Some((from, at, left)) = self.review_spring.clone() else { return (0., None) };
        let t = at.elapsed().as_secs_f32() / SPRING_DURATION.as_secs_f32();
        if t >= 1. {
            self.review_spring = None;
            return (0., None);
        }
        window.request_animation_frame();
        let offset = from * (1. - damped_spring(0.85, t));
        let left = left.and_then(|l| names.iter().position(|n| *n == l));
        (offset, left.map(|l| (l, if from > 0. { offset - width } else { offset + width })))
    }

    pub(super) fn finish_merge(&mut self, go: bool, cx: &mut Context<Self>) {
        let Some(confirm) = self.merge_confirm.as_mut().filter(|c| c.is_open()) else { return };
        confirm.close();
        let MergeAsk { dir, key, gitlab, number, auto, squash, remove_branch, .. } = confirm.value.clone();
        if go {
            self.review_act("merge".into(), key, move || crate::ui::review::merge(&dir, gitlab, number, auto, squash, remove_branch), cx);
        }
        cx.notify();
    }
}
