//! A file tab's editor, Zeron's `crates/ui/src/files/editor.rs` and `editor_adapter.rs` (MIT): gpui-base's
//! `EditorState`, colored by `zeron_syntax` through a highlighter that shifts its spans with each edit until the next
//! highlight lands. Its colors are the panes' (`crate::ui::markdown::syntax_color`), not Zeron's palette.
use crate::theme::Theme;
use gpui::{AppContext as _, Context, DispatchPhase, Entity, EntityId, HighlightStyle, IntoElement, PlatformInput, ScrollDelta, ScrollWheelEvent, SharedString, Styled as _, TouchPhase, Window, canvas, point, px};
use gpui_base::input::{EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputEditorStyle, InputHighlighter, Rope};
use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
    sync::Arc,
    time::Duration,
};
use zeron_syntax::{HighlightKind, HighlightedDocument};

/// The theme's color for each highlight name.
struct StyleResolver {
    theme: Theme,
}

impl HighlightStyleResolver for StyleResolver {
    fn style(&self, name: &str) -> Option<HighlightStyle> {
        kind_for_name(name).map(|kind| HighlightStyle { color: Some(crate::ui::markdown::syntax_color(kind, &self.theme)), ..Default::default() })
    }
}

#[derive(Clone)]
struct Highlighter {
    language: SharedString,
    spans: Vec<(Range<usize>, HighlightKind)>,
}

impl Highlighter {
    fn new(source: &str, document: &HighlightedDocument) -> Self {
        Self { language: format!("{:?}", document.language).into(), spans: absolute_spans(source, document) }
    }
}

impl InputHighlighter for Highlighter {
    fn language(&self) -> SharedString {
        self.language.clone()
    }

    /// Spans after an edit move with it; those it touched go until the next highlight.
    fn update(&mut self, edit: Option<InputEdit>, _text: &Rope, _folding: bool, _window: &mut Window, _cx: &mut Context<EditorState>) {
        let Some(edit) = edit else { return };
        let removed = edit.old_end_byte.saturating_sub(edit.start_byte);
        let inserted = edit.new_end_byte.saturating_sub(edit.start_byte);
        self.spans.retain_mut(|(range, _)| {
            if range.end <= edit.start_byte {
                return true;
            }
            if range.start >= edit.old_end_byte {
                if inserted >= removed {
                    let delta = inserted - removed;
                    range.start = range.start.saturating_add(delta);
                    range.end = range.end.saturating_add(delta);
                } else {
                    let delta = removed - inserted;
                    range.start = range.start.saturating_sub(delta);
                    range.end = range.end.saturating_sub(delta);
                }
                return true;
            }
            false
        });
    }

    fn styles(&self, range: &Range<usize>, resolver: &dyn HighlightStyleResolver) -> Vec<(Range<usize>, HighlightStyle)> {
        if range.is_empty() {
            return Vec::new();
        }
        let mut runs = Vec::new();
        let mut cursor = range.start;
        for (span, kind) in &self.spans {
            if span.end <= range.start {
                continue;
            }
            if span.start >= range.end {
                break;
            }
            let start = span.start.max(range.start).max(cursor);
            let end = span.end.min(range.end);
            if start > cursor {
                runs.push((cursor..start, HighlightStyle::default()));
            }
            if start < end {
                runs.push((start..end, resolver.style(name_for_kind(*kind)).unwrap_or_default()));
                cursor = end;
            }
        }
        if cursor < range.end {
            runs.push((cursor..range.end, HighlightStyle::default()));
        }
        runs
    }

    fn fold_ranges(&self, _text: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

pub(super) fn editor_style(theme: &Theme) -> InputEditorStyle {
    InputEditorStyle {
        foreground: theme.text.opacity(0.93),
        muted_foreground: theme.text_faint,
        background: gpui::transparent_black(),
        border: theme.border,
        selection: theme.accent.opacity(0.22),
        caret: theme.text,
        highlight_styles: Arc::new(StyleResolver { theme: theme.clone() }),
        editor_active_line: Some(theme.wash(0.025)),
        editor_gutter_background: Some(gpui::transparent_black()),
        ..Default::default()
    }
}

/// An editor over a file's `text`, read-only when only part of it was read.
pub(super) fn new_file_editor<T: 'static>(text: impl Into<SharedString>, path: &str, readonly: bool, theme: &Theme, window: &mut Window, cx: &mut Context<T>) -> Entity<EditorState> {
    let editor = cx.new(|cx| EditorState::new(window, cx).language(path.to_string()).line_number(true).folding(false).soft_wrap(false).default_value(text));
    editor.update(cx, |state, cx| {
        state.set_editor_style(editor_style(theme));
        state.set_readonly(readonly, cx);
    });
    editor
}

/// A highlight of `source` into the editor, if the editor still holds `source`: a stale one would color shifted bytes.
pub(super) fn install_highlighter(editor: &Entity<EditorState>, source: String, document: Arc<HighlightedDocument>, cx: &mut gpui::App) {
    if editor.read(cx).value().as_ref() != source {
        return;
    }
    editor.update(cx, |state, cx| {
        state.set_highlighter_factory(Rc::new(move |_| Some(Box::new(Highlighter::new(&source, &document)) as Box<_>)), cx);
    });
}

/// The file as it is on disk now, into an editor without changes: its cursor and scroll stay.
pub(super) fn replace_file_contents<T: 'static>(editor: &Entity<EditorState>, text: impl Into<SharedString>, window: &mut Window, cx: &mut Context<T>) {
    let text = text.into();
    editor.update(cx, |state, cx| {
        let selection = state.selected_range();
        let scroll_offset = state.scroll_offset();
        state.set_value(text, window, cx);
        let end = state.value().len();
        state.set_selected_range(selection.start.min(end)..selection.end.min(end), cx);
        state.set_scroll_offset(scroll_offset, cx);
    });
}

thread_local! {
    /// A wheel event `terminal_wheel` sent on, for the editor itself.
    static REPLAYING: Cell<bool> = const { Cell::new(false) };
}

/// The furthest the text shows pulled past an end.
const BAND: f32 = 120.;
/// The furthest the wheel's pull past an end counts.
const MAX_PULL: f32 = 400.;
/// What a wheel's step or a flick's coasting pulls, against a finger's.
const LOOSE_PULL: f32 = 0.3;
/// What is left of the pull each frame as it springs back.
const SPRING: f32 = 0.78;

/// The editor's rubber band past its first and last lines, as macOS's: the wheel's pull beyond an end shows damped,
/// and springs back once the fingers lift. Positive past the top, as the wheel's delta is.
#[derive(Default)]
pub(super) struct Elastic {
    pull: f32,
    finger: bool,
    /// Each wheel event's: a spring-back runs while it is still the last.
    run: u64,
}

impl Elastic {
    /// How far the text shows moved for the pull: iOS's rubber band, `BAND` at most.
    pub(super) fn shift(&self) -> f32 {
        let x = self.pull.abs();
        self.pull.signum() * (1. - 1. / (x * 0.55 / BAND + 1.)) * BAND
    }

    /// What of `dy` scrolls the text: a wheel back from the pull first takes the pull up.
    fn take_back(&mut self, dy: f32) -> f32 {
        if self.pull == 0. || dy == 0. || dy.signum() == self.pull.signum() {
            return dy;
        }
        let back = dy.abs().min(self.pull.abs());
        self.pull += dy.signum() * back;
        dy - dy.signum() * back
    }

    /// The part of a scroll the editor couldn't make, at its end.
    fn overshoot(&mut self, leftover: f32) {
        let pull = if self.finger { 1. } else { LOOSE_PULL };
        self.pull = (self.pull + leftover * pull).clamp(-MAX_PULL, MAX_PULL);
    }

    /// One frame of the spring back; true once it is home.
    fn spring(&mut self) -> bool {
        self.pull *= SPRING;
        if self.pull.abs() < 0.5 {
            self.pull = 0.;
        }
        self.pull == 0.
    }
}

/// The terminal's wheel speed over the editor, which scrolls 1:1, and `Elastic`'s band at its ends: laid over it, this
/// takes the wheel first and sends it on scaled, so the editor scrolls and clamps it itself (setting its offset would
/// land unclamped after a layout), and what it couldn't scroll pulls the band. `view` draws the band.
pub(super) fn terminal_wheel(elastic: Rc<RefCell<Elastic>>, editor: Entity<EditorState>, view: EntityId) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || REPLAYING.get() || !bounds.contains(&e.position) {
                    return;
                }
                cx.stop_propagation();
                let line = editor.read(cx).line_height().unwrap_or(px(20.));
                let delta = e.delta.pixel_delta(line) * crate::terminal::pane::WHEEL_SPEED;
                let dy = {
                    let mut elastic = elastic.borrow_mut();
                    match e.touch_phase {
                        TouchPhase::Started => elastic.finger = true,
                        TouchPhase::Ended | TouchPhase::Cancelled => elastic.finger = false,
                        TouchPhase::Moved => {}
                    }
                    elastic.run += 1;
                    elastic.take_back(f32::from(delta.y))
                };
                let faster = ScrollWheelEvent { delta: ScrollDelta::Pixels(point(delta.x, px(dy))), ..e.clone() };
                let (elastic, editor) = (elastic.clone(), editor.clone());
                // not from here: the window's listeners are out while it dispatches
                window.defer(cx, move |window, cx| {
                    let before = editor.read(cx).scroll_offset().y;
                    REPLAYING.set(true);
                    window.dispatch_event(PlatformInput::ScrollWheel(faster), cx);
                    REPLAYING.set(false);
                    let moved = f32::from(editor.read(cx).scroll_offset().y - before);
                    let (settle, run) = {
                        let mut elastic = elastic.borrow_mut();
                        elastic.overshoot(dy - moved);
                        (!elastic.finger && elastic.pull != 0., elastic.run)
                    };
                    cx.notify(view);
                    if settle {
                        cx.spawn(async move |cx| {
                            loop {
                                cx.background_executor().timer(Duration::from_millis(16)).await;
                                let home = {
                                    let mut elastic = elastic.borrow_mut();
                                    if elastic.run != run || elastic.finger {
                                        return;
                                    }
                                    elastic.spring()
                                };
                                cx.update(|cx| cx.notify(view));
                                if home {
                                    return;
                                }
                            }
                        })
                        .detach();
                    }
                });
            });
        },
    )
    .absolute()
    .inset_0()
}

fn absolute_spans(source: &str, document: &HighlightedDocument) -> Vec<(Range<usize>, HighlightKind)> {
    let mut starts = vec![0];
    starts.extend(source.bytes().enumerate().filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)));
    let mut spans = Vec::new();
    for (line_index, line) in document.lines.iter().enumerate() {
        let Some(line_start) = starts.get(line_index).copied() else { break };
        for span in line {
            let range = line_start.saturating_add(span.range.start)..line_start.saturating_add(span.range.end);
            if range.start <= range.end && range.end <= source.len() && source.is_char_boundary(range.start) && source.is_char_boundary(range.end) {
                spans.push((range, span.kind));
            }
        }
    }
    spans.sort_unstable_by_key(|(range, _)| (range.start, range.end));
    spans
}

const KINDS: [HighlightKind; 31] = [
    HighlightKind::Comment,
    HighlightKind::Keyword,
    HighlightKind::String,
    HighlightKind::StringSpecial,
    HighlightKind::Escape,
    HighlightKind::Number,
    HighlightKind::Boolean,
    HighlightKind::Type,
    HighlightKind::TypeBuiltin,
    HighlightKind::Constructor,
    HighlightKind::Function,
    HighlightKind::FunctionBuiltin,
    HighlightKind::Macro,
    HighlightKind::Property,
    HighlightKind::Constant,
    HighlightKind::Variable,
    HighlightKind::VariableSpecial,
    HighlightKind::Parameter,
    HighlightKind::Operator,
    HighlightKind::Punctuation,
    HighlightKind::Tag,
    HighlightKind::Attribute,
    HighlightKind::Label,
    HighlightKind::MarkupHeading,
    HighlightKind::MarkupRaw,
    HighlightKind::MarkupLink,
    HighlightKind::MarkupReference,
    HighlightKind::MarkupEmphasis,
    HighlightKind::MarkupStrong,
    HighlightKind::Embedded,
    HighlightKind::Invalid,
];

fn name_for_kind(kind: HighlightKind) -> &'static str {
    match kind {
        HighlightKind::Comment => "comment",
        HighlightKind::Keyword => "keyword",
        HighlightKind::String => "string",
        HighlightKind::StringSpecial => "string_special",
        HighlightKind::Escape => "escape",
        HighlightKind::Number => "number",
        HighlightKind::Boolean => "boolean",
        HighlightKind::Type => "type",
        HighlightKind::TypeBuiltin => "type_builtin",
        HighlightKind::Constructor => "constructor",
        HighlightKind::Function => "function",
        HighlightKind::FunctionBuiltin => "function_builtin",
        HighlightKind::Macro => "macro",
        HighlightKind::Property => "property",
        HighlightKind::Constant => "constant",
        HighlightKind::Variable => "variable",
        HighlightKind::VariableSpecial => "variable_special",
        HighlightKind::Parameter => "parameter",
        HighlightKind::Operator => "operator",
        HighlightKind::Punctuation => "punctuation",
        HighlightKind::Tag => "tag",
        HighlightKind::Attribute => "attribute",
        HighlightKind::Label => "label",
        HighlightKind::MarkupHeading => "markup_heading",
        HighlightKind::MarkupRaw => "markup_raw",
        HighlightKind::MarkupLink => "markup_link",
        HighlightKind::MarkupReference => "markup_reference",
        HighlightKind::MarkupEmphasis => "markup_emphasis",
        HighlightKind::MarkupStrong => "markup_strong",
        HighlightKind::Embedded => "embedded",
        HighlightKind::Invalid => "invalid",
    }
}

fn kind_for_name(name: &str) -> Option<HighlightKind> {
    KINDS.into_iter().find(|kind| name_for_kind(*kind) == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn highlighted(source: &str) -> HighlightedDocument {
        zeron_syntax::highlight(zeron_syntax::HighlightRequest { source, path: Some("a.rs"), fence_tag: None }).unwrap()
    }

    #[test]
    fn line_relative_spans_become_absolute_and_runs_cover_the_request_in_order() {
        let source = "let café = \"😀\";\nlet n = 42;";
        let highlighter = Highlighter::new(source, &highlighted(source));
        let string = source.find('"').unwrap();
        assert!(highlighter.spans.iter().any(|(r, k)| *k == HighlightKind::String && r.start == string));
        let resolver = StyleResolver { theme: Theme::for_appearance(gpui::WindowAppearance::Dark) };
        let runs = highlighter.styles(&(0..source.len()), &resolver);
        assert_eq!(runs.first().unwrap().0.start, 0);
        assert_eq!(runs.last().unwrap().0.end, source.len());
        assert!(runs.windows(2).all(|pair| pair[0].0.end == pair[1].0.start));
    }

    #[test]
    fn the_band_pulls_damped_gives_back_first_and_springs_home() {
        let mut band = Elastic { finger: true, ..Default::default() };
        band.overshoot(60.);
        let shown = band.shift();
        assert!(shown > 0. && shown < 60.);
        band.overshoot(10_000.);
        assert!(band.shift() < BAND);
        // scrolling back takes the pull up before the text moves
        assert_eq!(band.take_back(-100.), 0.);
        assert_eq!(band.take_back(-1_000.), -(1_000. - (MAX_PULL - 100.)));
        assert_eq!(band.pull, 0.);
        band.finger = false;
        band.overshoot(-100.);
        assert_eq!(band.pull, -100. * LOOSE_PULL);
        assert!(band.shift() < 0.);
        while !band.spring() {}
        assert_eq!(band.shift(), 0.);
    }

    #[test]
    fn every_kind_has_a_name_and_back() {
        for kind in KINDS {
            assert_eq!(kind_for_name(name_for_kind(kind)), Some(kind));
        }
    }
}
