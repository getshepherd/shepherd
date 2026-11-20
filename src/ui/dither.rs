//! The ordered-dither field, the site's one texture (`site/site.js`, `dither`): an area is cut into cells of `cell` px
//! with a 1 px gap, and a cell is lit where `value` at it passes the 4×4 Bayer threshold. It is static: nothing in it
//! asks for another frame.
use gpui::{Hsla, IntoElement, Pixels, Styled, canvas, point, px, size};

const BAYER: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

/// The cells are painted as quads every frame the window draws, rather than drawn once into a cached image: a field is
/// a few hundred quads, which gpui batches into one draw call, while an image would need a cache keyed by size, colour
/// and scale factor, rebuilt on every theme change or resize.
// ponytail: at most MAX_CELLS cells are looked at (a 640×150 px field at 3 px), so a field stretched over a wide area
// stays bounded and its far side is left blank; an image cache is the upgrade if fields ever fill whole panes.
const MAX_CELLS: usize = 214 * 50;

/// A field of `height` px that fills its parent's width, lit in `color` where `value(x, y)` (the cell's place as
/// 0..1 fractions of the field) passes the cell's threshold. `value` at 0 leaves a cell dark, at 1 lights it.
pub fn dither(height: Pixels, cell: Pixels, color: Hsla, value: fn(f32, f32) -> f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let (w, h, c) = (f32::from(bounds.size.width), f32::from(bounds.size.height), f32::from(cell));
            if w <= 0. || h <= 0. || c < 2. {
                return;
            }
            let (cols, rows) = ((w / c) as usize, (h / c) as usize);
            let cols = cols.min(MAX_CELLS / rows.max(1));
            for j in 0..rows {
                for i in 0..cols {
                    let threshold = (f32::from(BAYER[(j & 3) * 4 + (i & 3)]) + 0.5) / 16.;
                    if value(i as f32 * c / w, j as f32 * c / h) > threshold {
                        let origin = bounds.origin + point(px(i as f32 * c), px(j as f32 * c));
                        window.paint_quad(gpui::fill(gpui::Bounds { origin, size: size(cell - px(1.), cell - px(1.)) }, color));
                    }
                }
            }
        },
    )
    .w_full()
    .h(height)
}

/// A band that lies above a block of text: dense at its top corners and thinning down and in to nothing along its
/// bottom edge and its middle, so it never reaches the words under it. A slow wave keeps it from reading as a gradient.
pub fn horizon(x: f32, y: f32) -> f32 {
    let side = (x - 0.5).abs() * 2.;
    let wave = 0.5 + 0.5 * (x * 6. + (y * 5.).sin() * 2.2).sin();
    (1. - y) * (1. - y) * side * (0.35 + 0.65 * wave)
}

#[cfg(test)]
mod tests {
    use super::horizon;

    #[test]
    fn horizon_is_dark_where_the_words_are() {
        // The bottom edge and the middle column sit over the text: nothing may light there.
        for n in 0..=10 {
            let t = n as f32 / 10.;
            assert_eq!(horizon(t, 1.), 0.);
            assert_eq!(horizon(0.5, t), 0.);
        }
        assert!(horizon(0., 0.) > 0.1);
    }
}
