//! Tweens and springs: the sidebar's slide and edge nudge, worktree folds, and the seams that light up on hover.
use super::*;

/// Zeron's sidebar disclosure (`motion::COLLAPSE`): 180 ms CSS ease-out.
pub(super) const FOLD_DURATION: Duration = Duration::from_millis(180);

/// CSS `ease-out`, cubic-bezier(0, 0, 0.58, 1), solved for x by bisection.
pub(super) fn ease_out(x: f32) -> f32 {
    if x <= 0. || x >= 1. {
        return x.clamp(0., 1.);
    }
    let bezier = |p2: f32, t: f32| 3. * p2 * t * t * (1. - t) + t * t * t;
    let (mut lo, mut hi) = (0f32, 1f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.;
        if bezier(0.58, mid) < x { lo = mid } else { hi = mid }
    }
    bezier(1., (lo + hi) / 2.).clamp(0., 1.)
}

/// A damped spring (ζ 0.7) over its duration: a quick rise, about 5 % past the end, then home.
pub(super) fn spring(t: f32) -> f32 {
    damped_spring(0.7, t)
}

/// A spring damped by `zeta` over its duration: the closer to 1, the less it goes past the end.
pub(super) fn damped_spring(zeta: f32, t: f32) -> f32 {
    if t >= 1. {
        return 1.;
    }
    let omega = 14f32;
    let damped = omega * (1. - zeta * zeta).sqrt();
    1. - (-zeta * omega * t).exp() * ((damped * t).cos() + zeta * omega / damped * (damped * t).sin())
}

pub(super) const SPRING_DURATION: Duration = Duration::from_millis(240);

/// A place going from `from` to `to` since `at` on a [`spring`] that lasts as long as its user says: a reordered
/// row's top, a tab's tile's slot.
#[derive(Clone, Copy)]
pub(super) struct Slide {
    pub(super) from: f32,
    pub(super) to: f32,
    pub(super) at: Instant,
}

impl Slide {
    pub(super) fn now(&self, now: Instant, over: Duration) -> f32 {
        self.from + (self.to - self.from) * spring(now.saturating_duration_since(self.at).as_secs_f32() / over.as_secs_f32())
    }

    pub(super) fn done(&self, now: Instant, over: Duration) -> bool {
        now.saturating_duration_since(self.at) >= over
    }
}

/// Zeron's resize-edge spring (`motion::resize_bounce_offset`): 5 px out over the first 32 % of 220 ms, then
/// home over the rest, smoothstep both ways.
pub(super) const EDGE_SPRING: Duration = Duration::from_millis(220);

pub(super) fn edge_nudge(t: f32) -> f32 {
    let smooth = |t: f32| {
        let t = t.clamp(0., 1.);
        t * t * (3. - 2. * t)
    };
    let out = 0.32;
    5. * if t < out { smooth(t / out) } else { 1. - smooth((t - out) / (1. - out)) }
}

/// A dragged edge's spring: the limit it's on (-1 the least, 1 the most) and since when.
pub(super) type EdgeSpring = Option<(f32, Instant)>;

/// Keeps `spring` up with an edge dragged to `wanted`: one spring per arrival at a limit, not one per mouse move while
/// held there.
pub(super) fn spring_at_limit(spring: &mut EdgeSpring, wanted: f32, min: f32, max: f32) {
    let edge = if wanted <= min { -1. } else if wanted >= max { 1. } else { 0. };
    if edge == 0. {
        *spring = None;
    } else if spring.map(|(s, _)| s) != Some(edge) {
        *spring = Some((edge, Instant::now()));
    }
}

/// The spring's offset while it plays.
pub(super) fn spring_offset(spring: EdgeSpring) -> Option<f32> {
    let (sign, started) = spring?;
    let t = started.elapsed().as_secs_f32() / EDGE_SPRING.as_secs_f32();
    (t < 1.).then(|| sign * edge_nudge(t))
}

/// Zeron's resize seam (`Shell::resize_handle`): a 1 px line (the accent) `at` px into the hit strip that
/// fades out toward both ends. Shown while dragged; `hover` (hovered, flip count) fades it in and out like Zeron's
/// `HOVER_FADE`, restarting on every flip.
pub(super) fn resize_seam(
    id: SharedString,
    vertical: bool,
    at: f32,
    color: Hsla,
    dragging: bool,
    hover: Option<(bool, u64)>,
) -> gpui::AnyElement {
    let clear = color.opacity(0.);
    let angle = if vertical { 180. } else { 90. };
    let half = |from, to| {
        div().flex_1().bg(gpui::linear_gradient(
            angle,
            gpui::linear_color_stop(from, 0.),
            gpui::linear_color_stop(to, 1.),
        ))
    };
    let line = div().absolute().flex().child(half(clear, color)).child(half(color, clear));
    let line = if vertical {
        line.flex_col().top_0().bottom_0().left(px(at)).w(px(1.))
    } else {
        line.flex_row().left_0().right_0().top(px(at)).h(px(1.))
    };
    match (dragging, hover) {
        (true, _) => line.into_any_element(),
        (false, Some((on, flips))) => line
            .with_animation(
                SharedString::from(format!("{id}-fade-{flips}")),
                gpui::Animation::new(SEAM_FADE).with_easing(ease_out),
                move |el, t| el.opacity(if on { t } else { 1. - t }),
            )
            .into_any_element(),
        (false, None) => line.invisible().into_any_element(),
    }
}

pub(super) const SEAM_FADE: Duration = Duration::from_millis(150);

/// A pane divider's hit strip, centred on the panes' shared edge.
pub(super) const PANE_SEAM_HIT: f32 = 10.;

/// A pane divider: a 1 px `rest` line always on the panes' shared edge (the strip's centre), and over it the
/// sidebar's `lit` seam (`resize_seam`, fading toward both ends) while hovered or dragged.
pub(super) fn pane_seam(id: SharedString, vertical: bool, rest: Hsla, lit: Hsla, dragging: bool, hover: Option<(bool, u64)>) -> Div {
    let at = (PANE_SEAM_HIT - 1.) / 2.;
    let line = div().absolute().bg(rest);
    let line = if vertical { line.top_0().bottom_0().left(px(at)).w(px(1.)) } else { line.left_0().right_0().top(px(at)).h(px(1.)) };
    div().absolute().inset_0().child(line).child(resize_seam(id, vertical, at, lit, dragging, hover))
}

pub(super) fn fold_animation() -> gpui::Animation {
    gpui::Animation::new(FOLD_DURATION).with_easing(ease_out)
}

/// A group body's height tween; `epoch` restarts the animation on every toggle.
#[derive(Clone, Copy)]
pub(super) struct FoldMotion {
    pub(super) epoch: u64,
    pub(super) from: f32,
    pub(super) to: f32,
    pub(super) started: Instant,
}

impl FoldMotion {
    pub(super) fn current(self) -> f32 {
        let t = (self.started.elapsed().as_secs_f32() / FOLD_DURATION.as_secs_f32()).min(1.);
        self.from + (self.to - self.from) * ease_out(t)
    }

    /// A little past the end, so the last frame lands before the static height takes over.
    pub(super) fn animating(self) -> bool {
        self.started.elapsed() < FOLD_DURATION + Duration::from_millis(120)
    }
}

/// How long a pane's [`spring`] takes to grow it to fill its tab, or to take it back.
pub(super) const ZOOM_DURATION: Duration = Duration::from_millis(340);

/// A pane zooming (`grow`) from its place in the split to the whole tab, or unzooming back to it. The whole split is
/// scaled about the pane, so its neighbours and the dividers between them slide out past the tab's edges (and back in)
/// rather than being covered.
pub(super) struct ZoomMotion {
    pub(super) tab_id: String,
    pane_id: String,
    /// The tab's unzoomed frames: where the pane goes back to, and the panes pushed out on the way.
    split: Vec<crate::model::PaneFrame>,
    /// The unzoomed layout's dividers, pushed out with them.
    dividers: Vec<crate::model::SplitFrame>,
    grow: bool,
    at: Instant,
}

/// What the last render showed of a tab: its id, whether a pane filled it, its panes and its dividers.
pub(super) type ZoomSeen = Option<(String, bool, Vec<crate::model::PaneFrame>, Vec<crate::model::SplitFrame>)>;

impl ZoomMotion {
    /// Seen at a render: the tab `tab_id` with a pane filling it (`zoomed`) or not, and its `frames` and `splits`,
    /// against what the last render showed (`seen`, which this updates). A change of zoom in the same tab starts the
    /// motion.
    pub(super) fn follow(seen: &mut ZoomSeen, tab_id: &str, zoomed: bool, frames: &[crate::model::PaneFrame], splits: &[crate::model::SplitFrame]) -> Option<Self> {
        let (last_tab, was, before, before_splits) = seen.replace((tab_id.to_string(), zoomed, frames.to_vec(), splits.to_vec()))?;
        if last_tab != tab_id || was == zoomed {
            return None;
        }
        // the zoomed layout is the one pane alone; the other is the split it sits in
        let (split, dividers, pane_id) = match zoomed {
            true => (before, before_splits, frames.first()?.pane_id.clone()),
            false => (frames.to_vec(), splits.to_vec(), before.first()?.pane_id.clone()),
        };
        split.iter().any(|f| f.pane_id == pane_id).then(|| ZoomMotion { tab_id: tab_id.to_string(), pane_id, split, dividers, grow: zoomed, at: Instant::now() })
    }

    /// The tab's frames and dividers now; `None` once it has arrived.
    pub(super) fn frames(&self, now: Instant) -> Option<(Vec<crate::model::PaneFrame>, Vec<crate::model::SplitFrame>)> {
        let t = now.saturating_duration_since(self.at).as_secs_f32() / ZOOM_DURATION.as_secs_f32();
        (t < 1.).then(|| self.frames_at(spring(t)))
    }

    /// The frame pane `id`'s terminal keeps meanwhile, the one it had: it is resized once, on arrival, not on every
    /// frame. The zooming pane's is its place or the whole tab; the others' their place.
    pub(super) fn held(&self, id: &str) -> Option<crate::model::Frac> {
        let place = self.split.iter().find(|f| f.pane_id == id)?.frame;
        let full = crate::model::Frac { x: 0., y: 0., w: 1., h: 1. };
        Some(if id == self.pane_id && !self.grow { full } else { place })
    }

    fn frames_at(&self, t: f32) -> (Vec<crate::model::PaneFrame>, Vec<crate::model::SplitFrame>) {
        use crate::model::Frac;
        let p = if self.grow { t } else { 1. - t };
        let Some(place) = self.split.iter().find(|f| f.pane_id == self.pane_id).map(|f| f.frame) else {
            return (self.split.clone(), self.dividers.clone());
        };
        let mix = |a: f32, b: f32| a + (b - a) * p;
        // the pane's frame now, and the scale that takes its place there, applied to everything
        let now = Frac { x: mix(place.x, 0.), y: mix(place.y, 0.), w: mix(place.w, 1.), h: mix(place.h, 1.) };
        let (sx, sy) = (now.w / place.w.max(1e-4), now.h / place.h.max(1e-4));
        let map = |f: Frac| Frac { x: now.x + (f.x - place.x) * sx, y: now.y + (f.y - place.y) * sy, w: f.w * sx, h: f.h * sy };
        let frames = self
            .split
            .iter()
            .cloned()
            .map(|mut f| {
                f.frame = map(f.frame);
                f.focused = f.pane_id == self.pane_id;
                f
            })
            .collect();
        let dividers = self.dividers.iter().cloned().map(|mut d| { d.frame = map(d.frame); d }).collect();
        (frames, dividers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zoom_pushes_the_neighbour_and_divider_out_past_the_edge() {
        use crate::model::{Frac, PaneFrame, SplitFrame};
        let pane = |id: &str, x: f32| PaneFrame { pane_id: id.into(), frame: Frac { x, y: 0., w: 0.5, h: 1. }, focused: false, layer: None };
        let divider = SplitFrame { path: vec![], vertical: true, ratio: 0.5, frame: Frac { x: 0., y: 0., w: 1., h: 1. } };
        let m = ZoomMotion { tab_id: "t".into(), pane_id: "a".into(), split: vec![pane("a", 0.), pane("b", 0.5)], dividers: vec![divider], grow: true, at: Instant::now() };
        let (frames, dividers) = m.frames_at(1.);
        assert_eq!(frames[0].frame, Frac { x: 0., y: 0., w: 1., h: 1. });
        assert_eq!((frames[1].frame.x, frames[1].frame.w), (1., 1.));
        assert_eq!(crate::model::split_edge(&frames, &dividers[0]), 1.);
        let (frames, _) = m.frames_at(0.);
        assert_eq!(frames[1].frame, pane("b", 0.5).frame);
    }

    #[test]
    fn the_spring_overshoots_and_settles() {
        assert_eq!(spring(0.), 0.);
        let peak = (0..100).map(|i| spring(i as f32 / 100.)).fold(0f32, f32::max);
        assert!(peak > 1.02 && peak < 1.08, "{peak}");
        assert!((spring(0.99) - 1.).abs() < 0.001);
    }

    #[test]
    fn edge_nudge_springs_out_and_home() {
        assert_eq!(edge_nudge(0.), 0.);
        assert_eq!(edge_nudge(0.32), 5.);
        assert_eq!(edge_nudge(1.), 0.);
    }

    #[test]
    fn a_limit_springs_once_per_arrival() {
        let mut spring = None;
        spring_at_limit(&mut spring, 300., 200., 400.);
        assert!(spring.is_none());
        spring_at_limit(&mut spring, 450., 200., 400.);
        let arrived = spring.unwrap();
        assert_eq!(arrived.0, 1.);
        spring_at_limit(&mut spring, 480., 200., 400.); // held there
        assert_eq!(spring.unwrap().1, arrived.1);
        spring_at_limit(&mut spring, 300., 200., 400.);
        assert!(spring.is_none());
        spring_at_limit(&mut spring, 100., 200., 400.);
        assert_eq!(spring.unwrap().0, -1.);
    }

    #[test]
    fn ease_out_matches_css() {
        assert_eq!(ease_out(0.), 0.);
        assert_eq!(ease_out(1.), 1.);
        // CSS ease-out at x = 0.5 is ≈ 0.684
        assert!((ease_out(0.5) - 0.684).abs() < 0.01);
    }

    #[test]
    fn a_pane_grows_from_its_place_and_back() {
        use crate::model::{Frac, PaneFrame};
        let frame = |id: &str, x: f32| PaneFrame { pane_id: id.into(), frame: Frac { x, y: 0., w: 0.5, h: 1. }, focused: false, layer: None };
        let split = vec![frame("a", 0.), frame("b", 0.5)];
        let mut seen = None;
        assert!(ZoomMotion::follow(&mut seen, "t", false, &split, &[]).is_none(), "nothing seen before");
        let full = vec![PaneFrame { pane_id: "a".into(), frame: Frac { x: 0., y: 0., w: 1., h: 1. }, focused: true, layer: None }];
        assert!(ZoomMotion::follow(&mut seen.clone().map(|(_, z, f, d)| ("u".into(), z, f, d)), "t", true, &full, &[]).is_none(), "another tab");
        let grow = ZoomMotion::follow(&mut seen, "t", true, &full, &[]).expect("zooms");
        assert_eq!(grow.frames_at(0.).0[0].frame, split[0].frame);
        assert_eq!(grow.frames_at(1.).0[0].frame, full[0].frame);
        let back = ZoomMotion::follow(&mut seen, "t", false, &split, &[]).expect("unzooms");
        assert_eq!((back.frames_at(0.).0[0].frame, back.frames_at(1.).0[0].frame), (full[0].frame, split[0].frame));
        assert_eq!(back.held("a"), Some(full[0].frame), "the terminal keeps its size");
        assert_eq!(back.held("b"), Some(split[1].frame), "and the others theirs");
        assert!(ZoomMotion::follow(&mut seen, "t", false, &split, &[]).is_none(), "no change");
    }
}
