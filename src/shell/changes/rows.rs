//! The rows a file's diff is made of: notices, hunk headers, and the unified and split code lines, with their gutters,
//! markers and the horizontally scrolled code. They draw from what they are given and know nothing of [`Changes`].
use super::*;

/// The marker column between the gutters and the code.
const MARKER_WIDTH: f32 = 28.0;
/// The marker column in split mode: each half pays for its own, so it is narrower than [`MARKER_WIDTH`].
const SPLIT_MARKER_WIDTH: f32 = 18.0;
/// Hairline between the two split columns.
const SPLIT_DIVIDER_WIDTH: f32 = 1.0;

/// One notice row ("New file", "Binary file — contents not shown", …).
pub(super) fn notice_row(notice: String, theme: &Theme) -> AnyElement {
    div().h(px(NOTICE_HEIGHT)).w_full().flex_none().flex().items_center().px(px(16.0)).text_size(px(11.0)).text_color(theme.text_faint).child(notice).into_any_element()
}

/// One `@@ … @@` hunk-header row on the bluish-grey wash.
pub(super) fn hunk_header_row(header: &str, theme: &Theme) -> AnyElement {
    div()
        .h(px(HUNK_HEADER_HEIGHT))
        .w_full()
        .flex_none()
        .flex()
        .items_center()
        .px(px(16.0))
        .bg(theme.diff_hunk)
        .font_family(theme.font_terminal.clone())
        .text_size(px(12.0))
        .text_color(theme.text_faint)
        .child(header.to_string())
        .into_any_element()
}

/// The only part of a diff row allowed to exceed its viewport. The outer element keeps row chrome fixed; the inner
/// element owns the intrinsic code width and is the only plane moved by the file's horizontal scroll handle.
fn code_text_viewport(text: String, runs: Vec<gpui::TextRun>, theme: &Theme, padding_left: f32, content_width: Option<f32>, wrapped: bool, scroll: DiffCodeScroll) -> AnyElement {
    let content = div()
        .when(wrapped, |el| el.w_full().min_w_0())
        // Keep every tracked row's scroll extent identical. The width already includes shaping slack on the right, so
        // clipping here only prevents a child from redefining the shared maximum.
        .when_some(content_width, |el, width| el.w(px(width)).flex_none().overflow_hidden())
        .pl(px(padding_left))
        .font_family(theme.font_terminal.clone())
        .text_size(px(DIFF_TEXT_SIZE))
        .line_height(px(DIFF_LINE_HEIGHT))
        .map(|el| if wrapped { el.whitespace_normal() } else { el.whitespace_nowrap() })
        .child(gpui::StyledText::new(text).with_runs(runs));
    let viewport = div().flex_1().min_w_0().min_h(px(DIFF_LINE_HEIGHT)).overflow_hidden().child(content);
    if wrapped {
        return viewport.into_any_element();
    }
    // scrolled off its start, the code slides under the line numbers: a shadow along their edge says so, deepening over
    // the first few pixels; none where the lines fit and only the right padding scrolls
    let (offset, max) = (-f32::from(scroll.handle.offset().x), f32::from(scroll.handle.max_offset().x));
    let depth = if max > CODE_PADDING_RIGHT { (offset / 32.).clamp(0., 1.) } else { 0. };
    let scrolled = depth > 0.;
    let mut viewport = viewport.id(scroll.id).overflow_x_scroll().track_scroll(&scroll.handle);
    // Without this GPUI maps a vertical-only wheel delta onto x for an x-only scroller, starving the virtualized list
    // underneath.
    viewport.style().restrict_scroll_to_axis = Some(true);
    if !scrolled {
        return viewport.into_any_element();
    }
    let shadow = div().absolute().top_0().bottom_0().left_0().w(px(16.)).bg(gpui::linear_gradient(
        90.,
        gpui::linear_color_stop(gpui::black().opacity(0.14 * depth), 0.),
        gpui::linear_color_stop(gpui::black().opacity(0.), 1.),
    ));
    div().relative().flex_1().min_w_0().flex().child(viewport).child(shadow).into_any_element()
}

/// What sets a line's row apart: its marker, the marker's and the numbers' color, its tint and accent bar.
struct LineTone {
    marker: &'static str,
    marker_color: Hsla,
    row_bg: Option<Hsla>,
    accent: Option<Hsla>,
    number_color: Hsla,
}

fn line_tone(kind: LineKind, theme: &Theme) -> LineTone {
    // Row tints sampled from the reference: ~5–6% washes over the pane tone.
    let tinted = |marker, color: Hsla| LineTone { marker, marker_color: color, row_bg: Some(color.opacity(0.055)), accent: Some(color.opacity(0.55)), number_color: color.opacity(0.9) };
    match kind {
        LineKind::Add => tinted("+", theme.diff_add),
        LineKind::Del => tinted("−", theme.diff_del),
        _ => LineTone { marker: "·", marker_color: theme.text_faint.opacity(0.5), row_bg: None, accent: None, number_color: theme.text_faint.opacity(0.8) },
    }
}

fn gutter(no: Option<u32>, color: Hsla, gutter_px: f32, theme: &Theme) -> Div {
    div()
        .w(px(gutter_px))
        .flex_none()
        .font_family(theme.font_terminal.clone())
        .text_size(px(12.0))
        .line_height(px(DIFF_LINE_HEIGHT))
        .text_color(color)
        .flex()
        .justify_end()
        .pr(px(8.0))
        .child(no.map(|n| n.to_string()).unwrap_or_default())
}

fn marker_cell(tone: &LineTone, width: f32, theme: &Theme) -> Div {
    div()
        .w(px(width))
        .flex_none()
        .flex()
        .justify_center()
        .text_size(px(DIFF_TEXT_SIZE))
        .line_height(px(DIFF_LINE_HEIGHT))
        .text_color(tone.marker_color)
        .font_family(theme.font_terminal.clone())
        .child(tone.marker)
}

fn accent_bar(tone: &LineTone) -> Div {
    // solid on +/− rows, an invisible spacer on context rows so columns always align
    div().w(px(ACCENT_BAR_WIDTH)).self_stretch().flex_none().when_some(tone.accent, |el, color| el.bg(color))
}

/// One +/−/context/meta diff line: coloured accent bar, dual line-number gutters, marker column, and paint-only syntax
/// runs.
pub(super) fn diff_line_row(line: &DiffLine, highlights: Option<&DiffHighlights>, theme: &Theme, gutter_px: f32, code_width: DiffCodeWidth, scroll: DiffCodeScroll) -> AnyElement {
    if line.kind == LineKind::Meta {
        return meta_line_row(&line.text, theme, ACCENT_BAR_WIDTH + 2.0 * gutter_px + MARKER_WIDTH + 12.0);
    }
    let tone = line_tone(line.kind, theme);
    let idle = theme.text_faint.opacity(0.8);
    let content_width = match code_width {
        DiffCodeWidth::Scrollable(metrics) => Some(metrics.unified_content_width(gutter_px)),
        DiffCodeWidth::Wrapped => None,
    };
    let wrapped = matches!(code_width, DiffCodeWidth::Wrapped);
    div()
        .map(|el| if wrapped { el.min_h(px(DIFF_LINE_HEIGHT)) } else { el.h(px(DIFF_LINE_HEIGHT)) })
        .w_full()
        .flex_none()
        .flex()
        .flex_row()
        .items_start()
        .when_some(tone.row_bg, |el, bg| el.bg(bg))
        .child(accent_bar(&tone))
        .child(gutter(line.old_no, if line.kind == LineKind::Del { tone.number_color } else { idle }, gutter_px, theme))
        .child(gutter(line.new_no, if line.kind == LineKind::Add { tone.number_color } else { idle }, gutter_px, theme))
        .child(marker_cell(&tone, MARKER_WIDTH, theme))
        .child(code_text_viewport(line.text.clone(), line_runs(line, highlights, theme), theme, UNIFIED_CODE_PADDING_LEFT, content_width, wrapped, scroll))
        .into_any_element()
}

/// `\ No newline at end of file` and friends: a note about the row rather than code, so it is indented past the columns
/// and never tinted. In split mode it spans both halves.
pub(super) fn meta_line_row(text: &str, theme: &Theme, pad_left: f32) -> AnyElement {
    div().h(px(DIFF_LINE_HEIGHT)).w_full().flex_none().flex().items_center().pl(px(pad_left)).text_size(px(10.5)).text_color(theme.text_faint).italic().child(text.to_string()).into_any_element()
}

/// One half of a split row: the same accent bar / gutter / marker / code columns a unified row uses, minus the second
/// gutter. Takes prebuilt `runs` so a mirrored row can share one set across both columns.
pub(super) fn split_line_cell(line: &DiffLine, number: Option<u32>, runs: Vec<gpui::TextRun>, theme: &Theme, gutter_px: f32, code_width: DiffCodeWidth, scroll: DiffCodeScroll) -> Div {
    let tone = line_tone(line.kind, theme);
    let content_width = match code_width {
        DiffCodeWidth::Scrollable(metrics) => Some(metrics.split_content_width(gutter_px)),
        DiffCodeWidth::Wrapped => None,
    };
    let wrapped = matches!(code_width, DiffCodeWidth::Wrapped);
    div()
        .flex_1()
        .min_w_0()
        .self_stretch()
        .overflow_hidden()
        .flex()
        .flex_row()
        .items_start()
        .when_some(tone.row_bg, |el, bg| el.bg(bg))
        .child(accent_bar(&tone))
        .child(gutter(number, tone.number_color, gutter_px, theme))
        .child(marker_cell(&tone, SPLIT_MARKER_WIDTH, theme))
        .child(code_text_viewport(line.text.clone(), runs, theme, SPLIT_CODE_PADDING_LEFT, content_width, wrapped, scroll))
}

/// The empty half of a one-sided split row. A flat wash, quieter than either tint, reads as "nothing here".
pub(super) fn split_filler(theme: &Theme) -> Div {
    div().flex_1().min_w_0().self_stretch().bg(theme.wash(0.03))
}

/// Compose the two halves with the centre hairline.
pub(super) fn split_row(left: AnyElement, right: AnyElement, wrapped: bool, theme: &Theme) -> Div {
    div()
        .map(|el| if wrapped { el.min_h(px(DIFF_LINE_HEIGHT)) } else { el.h(px(DIFF_LINE_HEIGHT)) })
        .w_full()
        .flex_none()
        .flex()
        .flex_row()
        .items_stretch()
        .child(left)
        .child(div().w(px(SPLIT_DIVIDER_WIDTH)).self_stretch().flex_none().bg(theme.wash(0.06)))
        .child(right)
}

/// A split row's `+` only ever appears in the right column, which carries one gutter, so the offset is the same for
/// every line. It is measured from the column: the halves are fluid.
pub(super) fn split_adder_left(gutter_px: f32) -> f32 {
    ACCENT_BAR_WIDTH + (gutter_px - COMMENT_ADDER_SIZE) / 2.0
}

/// A unified row carries both gutters side by side, and a deletion numbers in the first.
pub(super) fn comment_adder_left(side: CommentSide, gutter_px: f32) -> f32 {
    let column = match side {
        CommentSide::Old => 0.0,
        CommentSide::New => gutter_px,
    };
    ACCENT_BAR_WIDTH + column + (gutter_px - COMMENT_ADDER_SIZE) / 2.0
}

pub(super) fn positioned_adder(left: f32, adder: AnyElement) -> Div {
    div().absolute().left(px(left)).top(px(0.0)).h_full().flex().items_center().child(adder)
}

/// Build only rows that start above `max_px` so the fold tween's stand-in never materializes lines its clip cannot
/// reveal.
pub(super) fn render_file_body_upto(file: &FileDiff, highlight: Option<Arc<DiffHighlights>>, theme: &Theme, max_px: f32, mode: DiffMode, code_width: DiffCodeWidth, scroll: DiffCodeScrollContext) -> AnyElement {
    let mut children: Vec<AnyElement> = Vec::new();
    let mut y = 0.0f32;
    let gutter_px = gutter_width(file);
    let wrapped = matches!(code_width, DiffCodeWidth::Wrapped);
    'build: {
        for notice in file_notices(file) {
            if y >= max_px {
                break 'build;
            }
            children.push(notice_row(notice, theme));
            y += NOTICE_HEIGHT;
        }
        // ponytail: the picture's room only, blank for the tween's few frames
        if file.image {
            children.push(div().w_full().h(px(IMAGE_HEIGHT)).into_any_element());
            y += IMAGE_HEIGHT;
        }
        for (hunk_ix, hunk) in file.hunks.iter().enumerate() {
            if y >= max_px {
                break 'build;
            }
            children.push(hunk_header_row(&hunk.header, theme));
            y += HUNK_HEADER_HEIGHT;
            match mode {
                DiffMode::Unified => {
                    for (line_ix, line) in hunk.lines.iter().enumerate() {
                        if y >= max_px {
                            break 'build;
                        }
                        children.push(diff_line_row(line, highlight.as_deref(), theme, gutter_px, code_width, scroll.slot(format_args!("{hunk_ix}-{line_ix}"))));
                        y += DIFF_LINE_HEIGHT;
                    }
                }
                DiffMode::Split => {
                    // Pair only what the clip can still reveal: the unified arm breaks out of a lazy walk, so the split arm
                    // must not materialize the whole hunk first.
                    let budget = ((max_px - y) / DIFF_LINE_HEIGHT).ceil().max(0.0) as usize;
                    for (pair_ix, (left, right)) in split_pairs_upto(&hunk.lines, budget).into_iter().enumerate() {
                        if y >= max_px {
                            break 'build;
                        }
                        let line_at = |slot: Option<u32>| slot.and_then(|slot| hunk.lines.get(slot as usize));
                        let cell = |line: Option<&DiffLine>, old: bool| match line {
                            Some(line) => split_line_cell(
                                line,
                                if old { line.old_no } else { line.new_no },
                                line_runs(line, highlight.as_deref(), theme),
                                theme,
                                gutter_px,
                                code_width,
                                scroll.slot(format_args!("{hunk_ix}-{pair_ix}-{}", if old { "old" } else { "new" })),
                            )
                            .into_any_element(),
                            None => split_filler(theme).into_any_element(),
                        };
                        let (left, right) = (line_at(left), line_at(right));
                        let marker = [left, right].into_iter().flatten().find(|line| line.kind == LineKind::Meta);
                        children.push(match marker {
                            Some(line) => meta_line_row(&line.text, theme, 2.0 * (ACCENT_BAR_WIDTH + gutter_px)),
                            None => split_row(cell(left, true), cell(right, false), wrapped, theme).into_any_element(),
                        });
                        y += DIFF_LINE_HEIGHT;
                    }
                }
            }
        }
    }
    div().flex().flex_col().pb(px(BODY_BOTTOM_PAD)).children(children).into_any_element()
}
