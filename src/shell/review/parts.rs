//! The review panel's pieces that hold no state: the PR's head, footer, timeline, summary and checks.
use super::*;

/// The panel's bottom bar while the PR or MR is open (a draft's has Mark as ready): Approve, which once one has reads Approved and takes it back
/// (GitLab), and Merge (off with conflicts or while CI runs); a button that is off says why.
pub(super) fn request_footer(
    theme: &Theme,
    r: &Request,
    running: bool,
    acting: &std::collections::HashSet<String>,
    on_approve: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_merge: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_ready: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Option<AnyElement> {
    let bar = || div().flex_none().px(px(12.)).py(px(10.)).flex().items_center().gap(px(8.)).border_t_1().border_color(theme.border).text_size(px(12.));
    if r.pr.state == "draft" {
        // a draft merges once it's ready
        let readying = acting.contains("ready");
        let hover = theme.hover;
        let ready = footer_button("review-ready", "icons/check.svg", if readying { "Marking…" } else { "Mark as ready" }, theme.text, None, readying)
            .border_1()
            .border_color(theme.border)
            .when(!readying, |el| el.hover(move |s| s.bg(hover)).on_click(on_ready));
        return Some(bar().child(div().flex_1().text_color(theme.text_faint).child("Draft")).child(ready).into_any_element());
    }
    if r.pr.state != "open" {
        return None;
    }
    let (approving, merging) = (acting.contains("approve"), acting.contains("merge"));
    let no_approve = match (r.approved, r.pr.gitlab) {
        (true, false) => Some("Dismiss the review on GitHub to take it back"),
        (true, true) => None,
        (false, _) if r.can_approve => None,
        (false, true) => Some("You can't approve this merge request"),
        (false, false) => Some("You can't approve your own pull request"),
    };
    let no_merge = (!r.mergeable).then_some("Resolve the conflicts first");
    let label = match (r.approved, approving) {
        (true, true) => "Revoking…",
        (true, false) => "Approved",
        (false, true) => "Approving…",
        (false, false) => "Approve",
    };
    let (icon, ink, edge) = if r.approved {
        ("icons/solar/check-circle.svg", theme.success, theme.success.opacity(0.5))
    } else {
        ("icons/check.svg", theme.text, theme.border)
    };
    let approve = footer_button("review-approve", icon, label, ink, no_approve, approving)
        .border_1()
        .border_color(edge)
        .when(r.approved, |el| el.bg(theme.success.opacity(0.12)))
        .when(no_approve.is_none() && !approving, |el| {
            let hover = theme.hover;
            el.hover(move |s| s.bg(hover)).when(r.approved, |el| el.tooltip(tip("Revoke approval", None))).on_click(on_approve)
        });
    // while the pipeline runs it merges once it passes; set so, a click turns that off
    let merge = if r.auto_merge {
        footer_button("review-merge", "icons/solar/clock-circle.svg", if merging { "Cancelling…" } else { "Auto-merge on" }, theme.success, None, merging)
            .border_1()
            .border_color(theme.success.opacity(0.5))
            .bg(theme.success.opacity(0.12))
            .when(!merging, |el| {
                let hover = theme.hover;
                el.hover(move |s| s.bg(hover)).tooltip(tip("Cancel auto-merge", None)).on_click(on_cancel)
            })
    } else {
        let (icon, label) = match (running, merging) {
            (_, true) => ("icons/git-pull-request.svg", "Merging…"),
            (true, false) => ("icons/solar/clock-circle.svg", "Auto-merge"),
            (false, false) => ("icons/git-pull-request.svg", "Merge"),
        };
        footer_button("review-merge", icon, label, gpui::white(), no_merge, merging)
            .bg(theme.danger)
            .when(running && no_merge.is_none() && !merging, |el| el.tooltip(tip("Merge once the pipeline passes", None)))
            .when(no_merge.is_none() && !merging, |el| el.on_click(on_merge))
    };
    Some(bar().child(div().flex_1()).child(approve).child(merge).into_any_element())
}

/// A footer button: icon and label in `ink`, dimmed with `off` (its reason, as the tooltip) or while `busy`.
pub(super) fn footer_button(id: &'static str, icon: &'static str, label: &'static str, ink: gpui::Hsla, off: Option<&'static str>, busy: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.))
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(8.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(ink)
        .child(gpui::svg().path(icon).size(px(13.)).flex_none().text_color(ink))
        .child(label)
        .map(|el| match off {
            Some(why) => el.opacity(0.45).tooltip(tip(why, None)),
            None if busy => el.opacity(0.7),
            None => el.cursor_pointer(),
        })
}

/// A PR state as it is written and coloured.
pub(in crate::shell) fn pr_tone(state: &str, theme: &Theme) -> (&'static str, Hsla) {
    match state {
        "draft" => ("Draft", theme.text_muted),
        "merged" => ("Merged", theme.accent),
        "closed" => ("Closed", theme.danger),
        _ => ("Open", theme.success),
    }
}

/// `1234567` as `1,234,567`.
pub(super) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The project a PR's or MR's page belongs to: `owner/repo` of `https://github.com/owner/repo/pull/42`.
pub(super) fn repo_of(url: &str) -> Option<String> {
    let project = url.split_once("/-/merge_requests/").or_else(|| url.split_once("/pull/"))?.0;
    crate::ui::review::repo_path(project)
}

/// A state pill: `icon` and `label` in `color`, on a wash of it inside its edge.
pub(super) fn pill(icon: &'static str, label: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .flex_none()
        .h(px(22.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(5.))
        .rounded(px(6.))
        .border_1()
        .border_color(color.opacity(0.35))
        .bg(color.opacity(0.1))
        .text_color(color)
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(gpui::svg().path(icon).size(px(13.)).text_color(color))
        .child(label.into())
}

/// The PR's head: its project and number (a click opens it) with its state; the title; the author, when it last
/// changed and the command that checks it out (a click copies it); the branches and how much it changes.
pub(super) fn request_head(theme: &Theme, r: &Request, menu: AnyElement) -> AnyElement {
    let (label, color) = pr_tone(r.pr.state, theme);
    let number = format!("{}{}", if r.pr.gitlab { "!" } else { "#" }, r.pr.number);
    let (accent, text) = (theme.accent, theme.text);
    let open = {
        let url = r.pr.url.clone();
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| super::browser::open_inside(&url, cx)
    };
    let project = div()
        .id("review-project")
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(4.))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(theme.text_muted)
        .hover(move |s| s.text_color(text))
        .children(repo_of(&r.pr.url).map(|repo| div().min_w_0().truncate().child(repo)))
        .child(div().flex_none().text_color(accent).child(number.clone()))
        .child(gpui::svg().path("icons/solar/square-arrow-right-up.svg").size(px(12.)).flex_none().text_color(accent))
        .tooltip(tip("Open in a browser tab", None))
        .on_click(open.clone());
    let open_now = r.pr.state == "open";
    let states = div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .when(open_now && !r.mergeable, |el| el.child(pill("icons/solar/close-circle.svg", "Conflicts", theme.danger)))
        .when(open_now && r.accepted, |el| el.child(pill("icons/solar/check-circle.svg", "Approved", theme.success)))
        .child(pill("icons/git-pull-request.svg", label, color))
        .child(menu);
    let title = div()
        .text_size(px(15.))
        .line_height(px(20.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.text)
        .child(if r.title.is_empty() { number.clone() } else { r.title.clone() });
    let updated = r.updated.map(|at| format!("updated {}", ago_text(at)));
    let checkout = format!("{} checkout {}", if r.pr.gitlab { "glab mr" } else { "gh pr" }, r.pr.number);
    let mono = theme.font_terminal.clone();
    let byline = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(theme.text_faint)
        .when(!r.author.is_empty(), |el| {
            let initial = r.author.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            el.child(
                div()
                    .size(px(18.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(theme.hover)
                    .text_size(px(10.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text_muted)
                    .child(initial),
            )
            .child(div().min_w_0().truncate().text_color(theme.text).child(r.author.clone()))
        })
        .children(updated.map(|u| div().flex_none().child(format!("· {u}"))))
        .child(div().flex_1())
        .child(
            div()
                .id("review-checkout")
                .flex_none()
                .font_family(mono.clone())
                .text_color(theme.text_muted)
                .cursor_pointer()
                .hover(move |s| s.text_color(text))
                .tooltip(tip("Copy", None))
                .child(checkout.clone())
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(checkout.clone()));
                    crate::ui::toast::info(cx, "Copied the checkout command", None);
                }),
        );
    let branches = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .text_size(px(12.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(4.))
                .font_family(mono.clone())
                .text_color(theme.text_faint)
                .child(branch_chip("review-base", &r.base, theme).flex_none())
                .when(!r.head.is_empty(), |el| el.child(div().flex_none().child("←")).child(branch_chip("review-head", &r.head, theme).min_w_0())),
        )
        .children(r.diff.map(|d| {
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(gpui::svg().path("icons/solar/file-text.svg").size(px(13.)).text_color(theme.text_faint))
                .child(div().text_color(theme.text_muted).child(format!("{} file{}", thousands(d.files), if d.files == 1 { "" } else { "s" })))
                .child(div().font_family(mono.clone()).text_color(theme.success).child(format!("+{}", thousands(d.additions))))
                .child(div().font_family(mono.clone()).text_color(theme.danger).child(format!("-{}", thousands(d.deletions))))
        }));
    div()
        .px(px(12.))
        .pt(px(12.))
        .pb(px(12.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .border_b_1()
        .border_color(theme.border)
        .child(div().flex().items_center().gap(px(8.)).child(project).child(states))
        .child(title)
        .child(byline)
        .child(branches)
        .into_any_element()
}

/// A branch's name; a click copies it.
pub(super) fn branch_chip(id: &'static str, name: &str, theme: &Theme) -> Stateful<Div> {
    let (text, name) = (theme.text, name.to_string());
    div()
        .id(id)
        .truncate()
        .cursor_pointer()
        .hover(move |s| s.text_color(text))
        .tooltip(tip("Copy branch name", None))
        .child(name.clone())
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(name.clone()));
            crate::ui::toast::info(cx, format!("Copied {name}"), None);
        })
}


/// How CI went, as the tabs' line says it: its state and a few words.
pub(super) fn ci_line(checks: &[Check], pipeline: Option<CheckState>, gitlab: bool) -> Option<(CheckState, String)> {
    if checks.is_empty() && pipeline.is_none() {
        return None;
    }
    let state = pipeline.unwrap_or_else(|| crate::ui::review::worst(checks.iter().map(|c| c.state)));
    let count = |states: &[CheckState]| checks.iter().filter(|c| states.contains(&c.state)).count();
    let some = |n: usize, what: &str| format!("{n} {}{} {what}", if gitlab { "job" } else { "check" }, if n == 1 { "" } else { "s" });
    let words = match state {
        CheckState::Success if gitlab => "Pipeline passed".into(),
        CheckState::Success => "All checks passed".into(),
        CheckState::Failed => match count(&[CheckState::Failed]) {
            0 => "Pipeline failed".into(),
            n => some(n, "failed"),
        },
        CheckState::Running | CheckState::Pending => match count(&[CheckState::Running, CheckState::Pending]) {
            0 => "Pipeline running".into(),
            n => some(n, "running"),
        },
        CheckState::Skipped => "Checks skipped".into(),
    };
    Some((state, words))
}

/// The tabs as one segmented control, and how CI went beside them (a click shows the checks).
pub(super) fn tab_bar(theme: &Theme, shown: Tab, checks_title: &'static str, ci: Option<(CheckState, String)>, pick: &dyn Fn(Tab) -> Listener, window: &mut Window) -> AnyElement {
    let (text, muted, on) = (theme.text, theme.text_muted, theme.card);
    let tab = |t: Tab, label: &'static str| {
        let selected = t == shown;
        div()
            .id(SharedString::from(format!("review-tab-{t:?}")))
            .h(px(24.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .map(|el| if selected { el.bg(on).text_color(text) } else { el.text_color(muted).cursor_pointer().hover(move |s| s.text_color(text)) })
            .child(label)
            .on_click(pick(t))
    };
    let tabs = div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(8.))
        .bg(theme.hover)
        .child(tab(Tab::Summary, "Summary"))
        .child(tab(Tab::Timeline, "Timeline"))
        .child(tab(Tab::Code, "Code"))
        .child(tab(Tab::Checks, checks_title));
    let on_ci = pick(Tab::Checks);
    div()
        .flex_none()
        .px(px(12.))
        .py(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(theme.border)
        .child(tabs)
        .child(div().flex_1())
        .children(ci.map(|(state, words)| {
            div()
                .id("review-ci")
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(muted)
                .cursor_pointer()
                .hover(move |s| s.text_color(text))
                .tooltip(tip(words.clone(), None))
                // a running check's spinner would redraw the window all the time; the Checks tab has it
                .map(|el| match state {
                    CheckState::Running => el.child(gpui::svg().path("icons/solar/clock-circle.svg").size(px(14.)).flex_none().text_color(theme.warning)),
                    _ => el.child(check_icon(theme, state, 14., window)),
                })
                .child(div().min_w_0().truncate().child(words))
                .on_click(on_ci)
        }))
        .into_any_element()
}

/// How long ago unix second `at` was: "just now", "5m ago", "2d ago".
pub(super) fn ago_text(at: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(at);
    match crate::model::ago(Duration::from_secs((now - at).max(0) as u64)).as_str() {
        "now" => "just now".to_string(),
        ago => format!("{ago} ago"),
    }
}

/// A person's initial in a small circle, standing in for their avatar.
pub(super) fn avatar(theme: &Theme, who: &str) -> Div {
    let initial = who.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    div()
        .size(px(20.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme.hover)
        .text_size(px(10.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.text_muted)
        .child(initial)
}

/// The Timeline tab: what happened on the PR, oldest first. A comment, or a review that says something, is a card with
/// its text; the rest are one line each, a push with its commits under it.
/// The timeline's events, an item each with the space above it.
pub(super) fn timeline(theme: &Theme, events: &[crate::ui::review::Event], look: &crate::ui::markdown::Look) -> Vec<(f32, AnyElement)> {
    use crate::ui::review::Happened;
    let mono = theme.font_terminal.clone();
    let item = |i: usize, e: &crate::ui::review::Event| -> AnyElement {
        let (icon, color, said): (&'static str, Hsla, String) = match &e.what {
            Happened::Opened => ("icons/git-pull-request.svg", theme.success, "opened this".into()),
            Happened::Commented => ("icons/solar/chat-round-line.svg", theme.text_muted, "commented".into()),
            Happened::Approved => ("icons/solar/check-circle.svg", theme.success, "approved".into()),
            Happened::Changes => ("icons/solar/close-circle.svg", theme.danger, "requested changes".into()),
            Happened::Reviewed => ("icons/solar/eye.svg", theme.text_muted, "reviewed".into()),
            Happened::Commits(c) => ("icons/solar/git-commit.svg", theme.text_muted, format!("pushed {} commit{}", c.len(), if c.len() == 1 { "" } else { "s" })),
            Happened::Merged => ("icons/git-pull-request.svg", theme.accent, "merged this".into()),
            Happened::Closed => ("icons/solar/close-circle.svg", theme.danger, "closed this".into()),
            Happened::Note => ("icons/solar/pen.svg", theme.text_faint, e.body.clone()),
        };
        let card = !e.body.trim().is_empty() && !matches!(e.what, Happened::Note);
        let line = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.))
            .text_color(theme.text_muted)
            .child(if card { avatar(theme, &e.who) } else { div().size(px(20.)).flex_none().flex().items_center().justify_center().child(gpui::svg().path(icon).size(px(14.)).text_color(color)) })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    // one run of text, the name set apart: two children would stack
                    .child({
                        let who = if e.who.is_empty() { String::new() } else { format!("{} ", e.who) };
                        let name = gpui::HighlightStyle { color: Some(theme.text), font_weight: Some(gpui::FontWeight::MEDIUM), ..Default::default() };
                        gpui::StyledText::new(format!("{who}{said}")).with_highlights([(0..who.len(), name)])
                    }),
            )
            .when(card && !matches!(e.what, Happened::Commented), |el| el.child(gpui::svg().path(icon).size(px(14.)).flex_none().text_color(color)))
            .child(div().flex_none().text_color(theme.text_faint).child(ago_text(e.at)));
        let commits = match &e.what {
            Happened::Commits(c) => Some(div().pl(px(28.)).flex().flex_col().gap(px(2.)).text_size(px(12.)).children(c.iter().map(|(sha, headline)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_none().font_family(mono.clone()).text_color(theme.text_faint).child(sha.clone()))
                    .child(div().min_w_0().truncate().text_color(theme.text).child(headline.clone()))
            }))),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(line)
            .children(commits)
            .when(card, |el| {
                el.child(
                    div()
                        .ml(px(28.))
                        .p(px(10.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.card)
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .text_size(px(13.))
                        .text_color(theme.text)
                        .children(crate::ui::markdown::render_with(&parsed(&e.body), &format!("review-tl-{i}"), theme, look)),
                )
            })
            .into_any_element()
    };
    events.iter().enumerate().map(|(i, e)| (if i == 0 { SECTION } else { 14. }, item(i, e))).collect()
}

/// The Summary tab's properties: who reviews it and what they said, its labels, and (GitLab) its open threads.
pub(super) fn summary(theme: &Theme, r: &Request) -> AnyElement {
    use crate::ui::review::Verdict;
    let none = || div().text_color(theme.text_faint).child("None").into_any_element();
    let row = |icon: &'static str, name: &'static str, value: AnyElement| {
        div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(
                div()
                    .w(px(96.))
                    .h(px(22.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_color(theme.text_muted)
                    .child(gpui::svg().path(icon).size(px(14.)).flex_none().text_color(theme.text_faint))
                    .child(name),
            )
            .child(div().flex_1().min_w_0().min_h(px(22.)).flex().flex_wrap().items_center().gap(px(6.)).child(value))
    };
    let reviewers = if r.reviewers.is_empty() {
        none()
    } else {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .children(r.reviewers.iter().map(|rv| {
                let (icon, color, said) = match rv.verdict {
                    Verdict::Approved => ("icons/solar/check-circle.svg", theme.success, "approved"),
                    Verdict::Changes => ("icons/solar/close-circle.svg", theme.danger, "requested changes"),
                    Verdict::Commented => ("icons/solar/chat-round-dots.svg", theme.text_muted, "commented"),
                    Verdict::Pending => ("icons/solar/clock-circle.svg", theme.text_faint, "hasn't reviewed yet"),
                };
                div()
                    .id(SharedString::from(format!("review-reviewer-{}", rv.name)))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_color(theme.text)
                    .tooltip(tip(format!("{} {said}", rv.name), None))
                    .child(gpui::svg().path(icon).size(px(14.)).flex_none().text_color(color))
                    .child(rv.name.clone())
            }))
            .into_any_element()
    };
    let labels = if r.labels.is_empty() {
        none()
    } else {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(4.))
            .children(r.labels.iter().map(|l| {
                div().h(px(20.)).px(px(6.)).flex().items_center().rounded(px(5.)).bg(theme.hover).text_color(theme.text).child(l.clone())
            }))
            .into_any_element()
    };
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .text_size(px(12.))
        .child(row("icons/solar/users-group-rounded.svg", "Reviewers", reviewers))
        .child(row("icons/solar/tag.svg", "Labels", labels))
        .children(r.unresolved.map(|n| {
            let value = match n {
                0 => div().text_color(theme.text_faint).child("All resolved"),
                n => div().text_color(theme.warning).child(format!("{n} unresolved")),
            };
            row("icons/solar/chat-round-dots.svg", "Threads", value.into_any_element())
        }))
        .into_any_element()
}

/// Runs CLI calls for a button, under its busy key.
pub(super) type OnAct = Rc<dyn Fn(String, Vec<Vec<String>>, &mut App)>;
/// Pages the checks to a pipeline, by its index.
pub(super) type OnPage = Rc<dyn Fn(usize, &mut App)>;

/// The checks' pages, GitLab pipelines or GitHub workflows, one shown at a time.
pub(super) struct Paging {
    pub(super) pages: Vec<(String, CheckState)>,
    /// The one shown.
    pub(super) page: usize,
    pub(super) on_page: OnPage,
    /// The shown page's offset, and the page beside it with its own, as `Shell::review_offset` has them.
    pub(super) offset: f32,
    pub(super) beside: Option<(usize, f32)>,
    /// How far a page slides: the panel's width.
    pub(super) width: f32,
    /// The job whose log shows under it, and the log's last lines once read.
    pub(super) log: Option<(String, Option<String>)>,
    /// Shows a job's log, or hides it shown.
    pub(super) on_log: Rc<dyn Fn(String, &mut App)>,
}

/// The page a swipe of `travel` px goes to: fingers to the left bring the next one.
pub(super) fn neighbour(count: usize, page: usize, travel: f32) -> Option<usize> {
    if travel < 0. { (page + 1 < count).then_some(page + 1) } else { page.checked_sub(1) }
}

/// The page's offset under the fingers, as the sidebar's list follows them: most of the way to the next page, past
/// that (or with none next) against a pull that grows the further they go.
pub(super) fn page_offset(travel: f32, near: bool, width: f32) -> f32 {
    let reach = if near { width * 0.85 } else { 0. };
    let over = (travel.abs() - reach).max(0.);
    travel.signum() * (travel.abs().min(reach) + (1. - 1. / (over * 0.4 / width + 1.)) * width)
}

/// The checks or jobs, with how many failed, run and passed; a GitLab pipeline's title carries its own state and
/// opens it, and a button retrying the failed ones. With child pipelines (or workflows) they page, one at a time: the
/// title has every page's state (its name on hover, a click opens it) and a swipe drags to the next; a trigger job
/// opens its child's page. A page's jobs come stage by stage under their stage's name. A row's button, shown on hover, retries,
/// plays or cancels it.
pub(super) fn checks_section(
    theme: &Theme,
    title: &'static str,
    checks: &[Check],
    pipeline: Option<(String, CheckState)>,
    gitlab: bool,
    acting: &std::collections::HashSet<String>,
    on_act: OnAct,
    paging: Paging,
    window: &mut Window,
) -> AnyElement {
    let count = |states: &[CheckState]| checks.iter().filter(|c| states.contains(&c.state)).count();
    let summary: Vec<String> = [
        (count(&[CheckState::Failed]), "failed"),
        (count(&[CheckState::Running, CheckState::Pending]), "running"),
        (count(&[CheckState::Success]), "passed"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    let hover = theme.hover;
    let Paging { pages, page, on_page, offset, beside, width, log, on_log } = paging;
    let paged = pages.len() > 1;
    let dots = paged.then(|| {
        div().flex().items_center().gap(px(1.)).children(pages.iter().enumerate().map(|(i, (name, state))| {
            let on_page = on_page.clone();
            div()
                .id(("review-page", i))
                .p(px(3.))
                .rounded(px(5.))
                .cursor_pointer()
                .when(i == page, |el| el.bg(theme.selected))
                .hover(move |s| s.bg(hover))
                .tooltip(tip(name.clone(), None))
                .child(check_icon(theme, *state, 12., window))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    on_page(i, cx);
                })
        }))
    });
    let heading = div()
        .id("review-pipeline")
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text_faint)
        .children(pipeline.as_ref().map(|(_, state)| check_icon(theme, *state, 14., window)))
        .child(div().flex_1().child(title))
        .map(|el| match dots {
            Some(dots) => el.child(dots),
            None => el.child(summary.join(" · ")),
        })
        .children({
            let calls = retry_failed(checks, gitlab);
            (!calls.is_empty()).then(|| {
                let on_act = on_act.clone();
                act_button(theme, "review-retry-failed".into(), "icons/solar/restart.svg", "Retry failed jobs", theme.text_muted, acting.contains("failed"), true, window)
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        on_act("failed".into(), calls.clone(), cx);
                    })
            })
        })
        .when_some(pipeline.map(|(url, _)| url), |el, url| el.cursor_pointer().hover(|s| s.underline()).on_click(move |_, _, cx| super::browser::open_inside(&url, cx)));
    let row = |i: usize, c: &Check, window: &mut Window| {
        let group: SharedString = format!("review-check-{i}").into();
        let action = c.action();
        let button = c.command(gitlab, action).map(|cmd| {
            let busy = cmd.join(" ");
            let (icon, label) = match action {
                crate::ui::review::Action::Cancel if !gitlab => ("icons/solar/stop-circle.svg", "Cancel workflow run"),
                crate::ui::review::Action::Cancel => ("icons/solar/stop-circle.svg", "Cancel job"),
                crate::ui::review::Action::Play => ("icons/solar/play.svg", "Run job"),
                _ => ("icons/solar/restart.svg", "Retry job"),
            };
            let ink = if action == crate::ui::review::Action::Cancel { theme.danger } else { theme.text_muted };
            let on_act = on_act.clone();
            act_button(theme, ("review-check-act", i).into(), icon, label, ink, acting.contains(&busy), false, window)
                .group_hover(group.clone(), |s| s.opacity(1.))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    on_act(busy.clone(), vec![cmd.clone()], cx);
                })
        });
        // a trigger job opens its child pipeline's page, not GitLab
        let child = c.child.as_ref().and_then(|name| pages.iter().position(|(n, _)| n == name));
        div()
            .id(("review-check", i))
            .group(group.clone())
            .h(px(28.))
            .px(px(6.))
            .mx(px(-6.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(6.))
            .child(check_icon(theme, c.state, 14., window))
            .child(div().flex_1().min_w_0().truncate().text_color(theme.text).child(c.name.clone()))
            .children(c.secs.map(|s| div().flex_none().text_color(theme.text_faint).child(duration(s))))
            .children(button)
            .when(child.is_some(), |el| el.child(gpui::svg().path("icons/alt-arrow-right.svg").size(px(12.)).flex_none().text_color(theme.text_faint)))
            // a job (not a trigger job, not a commit status) shows its log under it
            .map(|el| match (child, c.job.clone().filter(|_| !c.bridge && c.state != CheckState::Pending && !c.manual), c.url.clone()) {
                (Some(to), _, _) => {
                    let on_page = on_page.clone();
                    el.cursor_pointer().hover(move |s| s.bg(hover)).on_click(move |_, _, cx| on_page(to, cx))
                }
                (None, Some(job), _) => {
                    let on_log = on_log.clone();
                    el.cursor_pointer().hover(move |s| s.bg(hover)).on_click(move |_, _, cx| on_log(job.clone(), cx))
                }
                (None, None, Some(url)) => el.cursor_pointer().hover(move |s| s.bg(hover)).on_click(move |_, _, cx| super::browser::open_inside(&url, cx)),
                (None, None, None) => el,
            })
    };
    // a page's jobs, a run of them per stage
    let rows_for = |name: &str, window: &mut Window| -> Vec<AnyElement> {
        let mut stages: Vec<(&str, Vec<(usize, &Check)>)> = Vec::new();
        for (i, c) in checks.iter().enumerate().filter(|(_, c)| !paged || c.page == name) {
            match stages.last_mut() {
                Some((name, run)) if *name == c.group => run.push((i, c)),
                _ => stages.push((&c.group, vec![(i, c)])),
            }
        }
        let titled = stages.iter().any(|(name, _)| !name.is_empty());
        let mut rows: Vec<AnyElement> = Vec::new();
        for (stage, run) in stages {
            if titled {
                let state = crate::ui::review::worst(run.iter().map(|(_, c)| c.state));
                rows.push(
                    div()
                        .pt(px(if rows.is_empty() { 0. } else { 8. }))
                        .pb(px(2.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(11.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text_faint)
                        .child(check_icon(theme, state, 12., window))
                        .child(stage.to_string())
                        .into_any_element(),
                );
            }
            for (i, c) in run {
                rows.push(row(i, c, window).into_any_element());
                if let Some((_, text)) = log.as_ref().filter(|(job, _)| c.job.as_deref() == Some(job.as_str())) {
                    rows.push(log_box(theme, i, text.as_deref(), c.url.clone(), window));
                }
            }
        }
        rows
    };
    let shown = pages.get(page).map(|(n, _)| n.clone()).unwrap_or_default();
    let list = div().flex().flex_col().gap(px(2.)).children(rows_for(&shown, window));
    // pages slide within the panel, past the body's padding to its edges
    let pager = div().mx(px(-12.)).px(px(12.)).relative().overflow_hidden().child(list.relative().left(px(offset))).children(beside.map(|(n, at)| {
        let name = pages[n].0.clone();
        div().absolute().top_0().left(px(12. + at)).w(px(width - 24.)).flex().flex_col().gap(px(2.)).children(rows_for(&name, window))
    }));
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .text_size(px(12.))
        .child(div().pb(px(4.)).child(heading))
        .child(pager)
        .into_any_element()
}

/// A job's log under its row: its last lines as a code block, selectable and copied with Cmd+C, a spinner until read,
/// a link to its page.
pub(super) fn log_box(theme: &Theme, i: usize, text: Option<&str>, url: Option<String>, window: &mut Window) -> AnyElement {
    let accent = theme.accent;
    div()
        .mb(px(6.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .flex()
                .items_center()
                .text_size(px(11.))
                .text_color(theme.text_faint)
                .child(div().flex_1().child(format!("Last {} lines", crate::ui::review::LOG_LINES)))
                .children(url.map(|url| {
                    div()
                        .id(("review-log-open", i))
                        .cursor_pointer()
                        .hover(move |s| s.text_color(accent))
                        .child("Open job ↗")
                        .on_click(move |_, _, cx| super::browser::open_inside(&url, cx))
                })),
        )
        .child(match text {
            None => div().py(px(4.)).flex().justify_center().child(crate::ui::loader::mini_spinner(2., theme.text_faint, window)).into_any_element(),
            Some("") => div().text_size(px(11.)).text_color(theme.text_faint).child("No log yet").into_any_element(),
            Some(text) => {
                div().line_height(px(15.)).child(crate::ui::markdown::code(text, &format!("review-log-{i}"), px(10.5), theme)).into_any_element()
            }
        })
        .into_any_element()
}

/// A check's or the section's small button, its icon in `ink`; `busy` puts a spinner in it and keeps it shown, as
/// `shown` does.
pub(super) fn act_button(
    theme: &Theme,
    id: gpui::ElementId,
    icon: &'static str,
    label: &'static str,
    ink: gpui::Hsla,
    busy: bool,
    shown: bool,
    window: &mut Window,
) -> Stateful<Div> {
    let hover = theme.hover;
    div()
        .id(id)
        .size(px(20.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .when(!busy && !shown, |el| el.opacity(0.))
        .map(|el| {
            if busy {
                el.child(crate::ui::loader::mini_spinner(2., theme.text_faint, window))
            } else {
                el.cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .tooltip(tip(label, None))
                    .child(gpui::svg().path(icon).size(px(13.)).text_color(ink))
            }
        })
}
