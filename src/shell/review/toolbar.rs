//! The review panel's bars and menus: the Pipeline tab's picker, the Code tab's toolbar and the panel's menu.
use super::*;

impl Shell {
    /// The Pipeline tab's picker of the MR's pipelines, `past` newest first, `picked` the one shown; its menu while
    /// it's open.
    pub(super) fn pipeline_picker(&mut self, theme: &Theme, url: &str, past: &[crate::ui::review::PastPipeline], picked: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (hover, text) = (theme.hover, theme.text);
        let mono = theme.font_terminal.clone();
        let line = |i: usize| {
            let p = &past[i];
            let when = p.at.map(ago_text).unwrap_or_default();
            if i == 0 { format!("Latest · {when}") } else { format!("#{} · {when}", p.iid) }
        };
        let menu_open = self.review_pipeline_menu.as_ref().is_some_and(|m| m.is_open());
        let picker = div()
            .id("review-pipeline-picker")
            .flex_none()
            .max_w(px(260.))
            .h(px(26.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(6.))
            .bg(theme.hover)
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(text)
            .when(menu_open, |el| el.bg(theme.selected))
            .child(crate::ui::review::check_icon(theme, past[picked].state, 12., window))
            .child(div().min_w_0().truncate().child(line(picked)))
            .child(gpui::svg().path("icons/alt-arrow-down.svg").size(px(12.)).flex_none().text_color(theme.text_faint))
            .relative()
            .child(super::widgets::anchor_probe(&self.anchors, "pipelines"));
        let picker = crate::ui::trigger(
            picker,
            cx.listener(|this, e: &gpui::MouseDownEvent, _, cx| {
                let at = super::widgets::under(&this.anchors, "pipelines", e.position);
                crate::ui::Popup::toggle(&mut this.review_pipeline_menu, || at);
                cx.notify();
            }),
        );
        let menu = self.review_pipeline_menu.as_ref().map(|menu| {
            let rows = past.iter().enumerate().map(|(i, p)| {
                let (url, iid) = (url.to_string(), p.iid.clone());
                let short: String = p.sha.chars().take(7).collect();
                div()
                    .id(("review-pipeline", i))
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(i == picked, |el| el.bg(theme.selected))
                    .when(i != picked, |el| el.hover(move |s| s.bg(hover)))
                    .child(crate::ui::review::check_icon(theme, p.state, 12., window))
                    .child(div().flex_1().min_w_0().truncate().text_color(text).child(line(i)))
                    .child(div().flex_none().font_family(mono.clone()).text_color(theme.text_faint).child(short))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.review_pipeline = (i > 0).then(|| (url.clone(), iid.clone()));
                        if let Some(m) = this.review_pipeline_menu.as_mut() {
                            m.close();
                        }
                        cx.notify();
                    }))
            });
            let card = menu_card(theme, "review-pipeline-card", 260.)
                .max_h(px(420.))
                .overflow_y_scroll()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if let Some(m) = this.review_pipeline_menu.as_mut() {
                        m.close();
                    }
                    cx.notify();
                }))
                .flex()
                .flex_col()
                .gap(px(1.))
                .text_size(px(12.))
                .children(rows);
            let at = menu.value;
            let card = crate::ui::menu_motion("review-pipeline-in", menu, card);
            gpui::deferred(gpui::anchored().position(at).snap_to_window_with_margin(px(8.)).child(card)).with_priority(1).into_any_element()
        });
        div().flex().items_center().child(picker).children(menu).into_any_element()
    }

    /// The Code tab's bar: the commit picker, how many files there are and how many are viewed, what they add and
    /// remove, and a button folding or unfolding them all; the picker's menu while it's open.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn code_toolbar(
        &mut self,
        theme: &Theme,
        r: &Request,
        commits: &[(String, String)],
        commit: Option<&str>,
        files: &[crate::changes::FileDiff],
        viewed: &[bool],
        code: &Entity<super::changes::Changes>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (hover, text, muted) = (theme.hover, theme.text, theme.text_muted);
        let mono = theme.font_terminal.clone();
        let picked = commit.and_then(|c| commits.iter().find(|(sha, _)| sha == c)).map(|(_, line)| line.clone());
        let menu_open = self.review_commit_menu.as_ref().is_some_and(|m| m.is_open());
        let picker = div()
            .id("review-commit-picker")
            .flex_none()
            .max_w(px(220.))
            .h(px(26.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(6.))
            .bg(theme.hover)
            .cursor_pointer()
            .text_color(text)
            .when(menu_open, |el| el.bg(theme.selected))
            .child(div().min_w_0().truncate().child(picked.unwrap_or_else(|| if commit.is_some() { "One commit".into() } else { "All commits".into() })))
            .child(gpui::svg().path("icons/alt-arrow-down.svg").size(px(12.)).flex_none().text_color(theme.text_faint))
            .relative()
            .child(super::widgets::anchor_probe(&self.anchors, "commits"));
        let picker = crate::ui::trigger(
            picker,
            cx.listener(|this, e: &gpui::MouseDownEvent, _, cx| {
                // the list hangs under the picker, at its left edge
                let at = super::widgets::under(&this.anchors, "commits", e.position);
                if crate::ui::Popup::toggle(&mut this.review_commit_menu, || at) {
                    this.review_commit_more = false;
                }
                cx.notify();
            }),
        );
        let n = files.len();
        let seen = viewed.iter().filter(|v| **v).count();
        let count = match (n, commit) {
            (0, _) => String::new(),
            (n, Some(_)) => format!("{} file{}", thousands(n as u64), if n == 1 { "" } else { "s" }),
            (n, None) => format!("{} file{} · {seen} / {n} viewed", thousands(n as u64), if n == 1 { "" } else { "s" }),
        };
        let (adds, dels) = files.iter().fold((0, 0), |(a, d), f| (a + f.additions as u64, d + f.deletions as u64));
        // the diff's own toggles, as its tab has them
        use super::changes::Changes;
        let (split, wrap, fold_label) = {
            let c = code.read(cx);
            (c.is_split(), c.wraps(), c.fold_all_label())
        };
        let toggle = |id: &'static str, icon: &'static str, label: &'static str, on: bool, act: fn(&mut Changes, &mut Context<Changes>)| {
            let code = code.clone();
            Changes::header_toggle(id, icon, label, on, theme).on_click(move |_, _, cx| code.update(cx, act))
        };
        let toggles = (n > 0).then(|| {
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(toggle("review-split", "icons/split-columns.svg", "Split view", split, Changes::toggle_mode))
                .child(toggle("review-wrap", "icons/wrap-text.svg", "Wrap long lines", wrap, Changes::toggle_wrap))
                .child(toggle("review-fold-all", "icons/fold-vertical.svg", fold_label, false, Changes::toggle_collapse_all))
        });
        let menu = self.review_commit_menu.as_ref().map(|menu| {
            let more = self.review_commit_more;
            let pick = |id: gpui::ElementId, line: String, sha: Option<String>, selected: bool| {
                let url = r.pr.url.clone();
                let short: String = sha.as_deref().unwrap_or("").chars().take(7).collect();
                div()
                    .id(id)
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |el| el.bg(theme.selected))
                    .when(!selected, |el| el.hover(move |s| s.bg(hover)))
                    .child(div().flex_1().min_w_0().truncate().text_color(text).child(line))
                    .when(!short.is_empty(), |el| el.child(div().flex_none().font_family(mono.clone()).text_color(theme.text_faint).child(short)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.review_commit = sha.clone().map(|sha| (url.clone(), sha));
                        if let Some(m) = this.review_commit_menu.as_mut() {
                            m.close();
                        }
                        cx.notify();
                    }))
            };
            const SHOWN: usize = 10;
            let shown = if more { commits.len() } else { commits.len().min(SHOWN) };
            let card = menu_card(theme, "review-commit-card", 300.)
                .max_w(px(440.))
                .max_h(px(420.))
                .overflow_y_scroll()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if let Some(m) = this.review_commit_menu.as_mut() {
                        m.close();
                    }
                    cx.notify();
                }))
                .flex()
                .flex_col()
                .gap(px(1.))
                .text_size(px(12.))
                .child(pick("review-commit-all".into(), "All commits".into(), None, commit.is_none()))
                .children(commits.iter().take(shown).enumerate().map(|(i, (sha, line))| pick(("review-commit", i).into(), line.clone(), Some(sha.clone()), commit == Some(sha.as_str()))))
                .when(commits.len() > shown, |el| {
                    el.child(
                        div()
                            .id("review-commit-more")
                            .h(px(28.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_color(muted)
                            .hover(move |s| s.bg(hover))
                            .child(format!("Show more ({} left)", commits.len() - shown))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.review_commit_more = true;
                                cx.notify();
                            })),
                    )
                });
            let at = menu.value;
            let card = crate::ui::menu_motion("review-commit-in", menu, card);
            gpui::deferred(gpui::anchored().position(at).snap_to_window_with_margin(px(8.)).child(card)).with_priority(1).into_any_element()
        });
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.))
            .text_color(muted)
            .child(picker)
            .child(div().flex_1().min_w_0().truncate().child(count))
            .when(n > 0, |el| {
                el.child(div().flex_none().font_family(mono.clone()).text_color(theme.success).child(format!("+{}", thousands(adds))))
                    .child(div().flex_none().font_family(mono.clone()).text_color(theme.danger).child(format!("-{}", thousands(dels))))
            })
            .children(toggles)
            .children(menu)
            .into_any_element()
    }

    /// An image of the PR's text in the panes' image viewer, the one a diff's image opens in: it's a pane's and takes
    /// its keys there, so the keyboard goes along.
    pub(super) fn preview_review_image(&mut self, path: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.active_pane(window, cx).and_then(|id| self.panes.get(&id)).cloned() else { return };
        self.release_panel();
        window.focus(&pane.focus_handle(cx), cx);
        pane.update(cx, |pane, cx| pane.show_preview(path, cx));
    }

    /// The shown PR's link (`link`) or number on the clipboard, while the review panel is what the right panel shows.
    pub(in crate::shell) fn copy_review(&mut self, link: bool, window: &Window, cx: &mut Context<Self>) {
        let shown = self.right_panel_shown(window) && self.panel().is_some_and(|p| p.active == Some(crate::right_panel::Surface::PullRequest));
        let Some(r) = self.review_request.as_ref().filter(|_| shown) else { return };
        let (text, said) = if link {
            (r.pr.url.clone(), "Copied the link")
        } else {
            (format!("{}{}", if r.pr.gitlab { "!" } else { "#" }, r.pr.number), "Copied the number")
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        crate::ui::toast::info(cx, said, None);
    }

    /// The agent the review panel's "…" menu talks to: the shown pane's, else the workspace's first.
    pub(super) fn review_agent(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<crate::model::Pane> {
        let workspace = self.shown_tab()?.workspace_id;
        let pane = self.active_pane(window, cx);
        let agents = self.snapshot.as_ref()?.agents_in(&workspace, SettingsGlobal::of(cx).settings.annotation_agents);
        crate::annotations::default_recipient(&agents, pane.as_deref(), Some(&workspace)).cloned()
    }

    /// Puts `text` in the agent's prompt, without sending it, and gives it the keyboard; an agent whose pane isn't
    /// shown gets it on the clipboard instead.
    pub(super) fn ask_agent(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(agent) = self.review_agent(window, cx) else { return crate::ui::toast::error(cx, "No agent here to ask") };
        let name = agent.agent.clone().unwrap_or_default();
        match self.panes.get(&agent.pane_id).cloned() {
            Some(pane) => {
                pane.update(cx, |pane, cx| pane.paste(&text, cx));
                self.release_panel();
                window.focus(&pane.focus_handle(cx), cx);
            }
            None => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                crate::ui::toast::info(cx, format!("Copied: paste it to {name}"), None);
            }
        }
        cx.notify();
    }

    /// Sends `prompt` to the agent, as the diff's comments go.
    pub(super) fn tell_agent(&mut self, prompt: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(agent) = self.review_agent(window, cx) else { return crate::ui::toast::error(cx, "No agent here to send it to") };
        let name = agent.agent.clone().unwrap_or_default();
        let (backend, pane) = (self.backend.clone(), agent.pane_id);
        self.ask(async move { backend.mux.send(pane, Some(prompt)).await }, cx, move |_, result, cx| match result {
            Ok(_) => crate::ui::toast::info(cx, format!("Sent to {name}"), None),
            Err(e) => crate::ui::toast::error(cx, format!("Not sent to {name}: {e:#}")),
        });
    }

    /// The "…" menu: the agent's questions about the PR, then refresh, open and copy.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_review_menu(
        &mut self,
        theme: &Theme,
        at: gpui::Point<Pixels>,
        r: &Request,
        pr_key: Option<String>,
        key: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        type Act = Box<dyn Fn(&mut Shell, &mut Window, &mut Context<Shell>)>;
        let menu = self.review_menu.as_ref()?;
        let agent = self.review_agent(window, cx).and_then(|a| a.agent);
        let prompts = review_prompts(r);
        let host = if r.pr.gitlab { "GitLab" } else { "GitHub" };
        let noun = if r.pr.gitlab { "MR" } else { "PR" };
        let number = format!("{}{}", if r.pr.gitlab { "!" } else { "#" }, r.pr.number);
        let chords = cx.try_global::<crate::bindings::ActiveBindings>().map(|b| b.0.clone()).unwrap_or_default();
        // the keys bound to `action`, as the menu shows them: ⇧⌘C
        let keys = |action: &str| {
            chords.iter().find(|(name, _)| name == action).map(|(_, keys)| keys.split_whitespace().map(crate::ui::command_panel::glyphs).collect::<Vec<_>>().join(" "))
        };
        let item = |id: &'static str, icon: &'static str, label: String, hint: Option<String>, off: bool, act: Act| {
            let hover = theme.hover;
            div()
                .id(id)
                .px(px(8.))
                .py(px(5.))
                .rounded(px(6.))
                .flex()
                .items_start()
                .gap(px(8.))
                .child(div().h(px(18.)).flex().items_center().child(crate::ui::icon(icon, 14., theme.text_muted)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().line_height(px(18.)).text_color(theme.text).child(label))
                        .children(hint.map(|h| div().text_size(px(11.)).line_height(px(15.)).text_color(theme.text_faint).child(h))),
                )
                .map(|el| {
                    if off {
                        el.opacity(0.45)
                    } else {
                        el.cursor_pointer().hover(move |s| s.bg(hover)).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            if let Some(m) = this.review_menu.as_mut() {
                                m.close();
                            }
                            act(this, window, cx);
                            cx.notify();
                        }))
                    }
                })
        };
        let to = agent.clone().unwrap_or_else(|| "the agent".into());
        let no_agent = agent.is_none();
        let (ask, explain, fix) = prompts;
        let (url, url2) = (r.pr.url.clone(), r.pr.url.clone());
        let checkout = format!("{} checkout {}", if r.pr.gitlab { "glab mr" } else { "gh pr" }, r.pr.number);
        let copy = |text: String, said: &'static str| -> Act {
            Box::new(move |_, _, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
                crate::ui::toast::info(cx, said, None);
            })
        };
        let card = menu_card(theme, "review-menu-card", 260.)
            .max_w(px(320.))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if let Some(m) = this.review_menu.as_mut() {
                    m.close();
                }
                cx.notify();
            }))
            .flex()
            .flex_col()
            .gap(px(1.))
            .text_size(px(12.))
            .child(item(
                "review-menu-refresh",
                "icons/solar/refresh.svg",
                "Refresh".into(),
                None,
                false,
                Box::new(move |this, _, cx| {
                    if let Some(k) = &key {
                        refetch::<Review>(k, cx);
                    }
                    if let Some(k) = &pr_key {
                        this.refetch_pr(k, cx);
                    }
                }),
            ))
            .child(item(
                "review-menu-ask",
                "icons/lucide-message-circle-question.svg",
                "Ask a question".into(),
                Some(format!("Puts the {noun} in {to}'s prompt.")),
                no_agent,
                Box::new(move |this, window, cx| this.ask_agent(ask.clone(), window, cx)),
            ))
            .child(item(
                "review-menu-explain",
                "icons/solar/document-text.svg",
                format!("Explain this {noun}"),
                Some("A walk through the diff and what to read closely.".into()),
                no_agent,
                Box::new(move |this, window, cx| this.tell_agent(explain.clone(), window, cx)),
            ))
            .child(item(
                "review-menu-fix",
                "icons/solar/pen.svg",
                "Fix review findings".into(),
                Some("Asks for its comments and failing checks to be dealt with.".into()),
                no_agent,
                Box::new(move |this, window, cx| this.tell_agent(fix.clone(), window, cx)),
            ))
            .child(div().mx(px(-4.)).my(px(4.)).h(px(1.)).bg(theme.border))
            .child(item("review-menu-open", "icons/solar/global.svg", format!("Open {noun}"), Some("In a browser tab here.".into()), false, {
                let url = url.clone();
                Box::new(move |_, _, cx| super::browser::open_inside(&url, cx))
            }))
            .child(item("review-menu-open-outside", "icons/solar/square-arrow-right-up.svg", format!("Open on {host}"), Some("In the default browser.".into()), false, Box::new(move |_, _, cx| crate::ui::open_outside(&url, cx))))
            .child(item("review-menu-link", "icons/solar/link.svg", "Copy link".into(), None, false, copy(url2, "Copied the link")).children(keys("CopyPageUrl").map(|k| shortcut(theme, k))))
            .child(item("review-menu-number", "icons/solar/copy.svg", format!("Copy {noun} number"), None, false, copy(number, "Copied the number")).children(keys("CopyPrNumber").map(|k| shortcut(theme, k))))
            .child(item("review-menu-checkout", "icons/terminal.svg", "Copy checkout command".into(), None, false, copy(checkout, "Copied the checkout command")));
        let card = crate::ui::menu_motion("review-menu-in", menu, card);
        Some(gpui::deferred(gpui::anchored().position(at).anchor(gpui::Anchor::TopRight).snap_to_window_with_margin(px(8.)).child(card)).with_priority(1).into_any_element())
    }
}
