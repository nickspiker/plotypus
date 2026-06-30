//! Plot region: axes, grid, and curve. View bounds are kept in `PlotView` so pan/zoom can mutate them without touching the rendering code.
//!
//! The plot draws straight into the host's CPU present buffer (`&mut [u32]`) that Fluor hands `FluorApp::render`. Grid + curve are raw per-pixel writes; axis labels go through Fluor's `TextRenderer`, which draws into a `Canvas` over the same buffer (constructed after the raw writes finish, so the borrows don't overlap). The plot does no hit-testing — the app routes plot interaction by rect containment, so no per-pixel hit map is threaded here.
use crate::formula::{self, Token};
use crate::ui::theme;
use fluor::canvas::{Canvas, Damage};
use fluor::paint::Clip;
use fluor::pixel::Blend;
use fluor::text::TextRenderer;
use fluor::BlendMode;
use spirix::ScalarF4E3 as S43;

/// Complement of the RGB bytes — converts a visible-RGB colour to fluor's darkness
/// convention (and vice-versa, it's an involution). Alpha is untouched.
#[inline]
const fn darken(visible_rgb: u32) -> u32 {
    visible_rgb ^ 0x00FF_FFFF
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct PlotView {
    pub x_min: S43,
    pub x_max: S43,
    pub y_min: S43,
    pub y_max: S43,
}

impl Default for PlotView {
    fn default() -> Self {
        Self {
            x_min: S43::NEG_ONE,
            x_max: S43::ONE,
            y_min: S43::NEG_ONE,
            y_max: S43::ONE,
        }
    }
}

/// Number of halving levels per axis. Brightness goes 128 → 64 → ... → 1.
const LEVELS: usize = 8;
/// Brightest (level 0) line.
const TOP_BRIGHTNESS: u32 = 128;

pub fn draw_plot(
    pixels: &mut [u32],
    text_renderer: &mut TextRenderer,
    damage: &mut Damage,
    window_width: usize,
    window_height: usize,
    rect: Rect,
    view: PlotView,
    label_font_size: f32,
    formula: Option<&[Token]>,
    parse_failed: bool,
    base: u8,
) {
    // Parse error → blank black plot region. Skipping grid + labels + curve makes "your formula didn't parse" obvious at a glance, distinct from a valid expression that happens to evaluate offscreen.
    if parse_failed {
        fill_bg(pixels, window_width, rect);
        return;
    }
    // Everything composites in fluor's native darkness convention via `under` (topmost
    // paints first wins), so we draw front-to-back: axis labels on top, then the grid,
    // then the curve fill, then the black background fills whatever is still transparent
    // beneath them. No visible-RGB working buffer, no whole-rect XOR.
    draw_axis_labels(pixels, text_renderer, damage, window_width, window_height, rect, view, label_font_size, base);
    draw_dyadic_grid(pixels, window_width, rect, view);
    if let Some(tokens) = formula {
        // Eval errors (stack underflow, etc.) shouldn't fire on a token vector that
        // tokenize() accepted — but if they do, fall back to INFINITY so the column
        // shows as a yellow stripe (visible, not crashy).
        let curve = |x: S43| formula::evaluate(tokens, x, base).unwrap_or(S43::INFINITY);
        draw_curve(pixels, window_width, rect, view, curve);
    }
    fill_bg(pixels, window_width, rect);
}

/// Same as [`draw_plot`] but plots an arbitrary `curve: Fn(S43) -> S43` instead of a
/// parsed formula — used by the Photon notification view, which feeds the synth's S43
/// `voice(t)` straight in (world-x is time in seconds). Background, grid, and axis
/// labels are identical to the formula path so the two views are visually consistent.
pub fn draw_plot_curve(
    pixels: &mut [u32],
    text_renderer: &mut TextRenderer,
    damage: &mut Damage,
    window_width: usize,
    window_height: usize,
    rect: Rect,
    view: PlotView,
    label_font_size: f32,
    base: u8,
    curve: impl Fn(S43) -> S43,
) {
    // Front-to-back, all darkness + `under` (see `draw_plot`): labels, grid, curve, bg.
    draw_axis_labels(pixels, text_renderer, damage, window_width, window_height, rect, view, label_font_size, base);
    draw_dyadic_grid(pixels, window_width, rect, view);
    draw_curve(pixels, window_width, rect, view, curve);
    fill_bg(pixels, window_width, rect);
}

/// Fill the plot rect with opaque black, in darkness convention, via `under` — so it lands
/// only on pixels still transparent after the labels / grid / curve drew on top. Opaque
/// black in darkness is `0xFF_FF_FF_FF` (α opaque, RGB = max darkness).
fn fill_bg(pixels: &mut [u32], window_width: usize, rect: Rect) {
    for y in rect.y..rect.y + rect.h {
        let row = y * window_width;
        for x in rect.x..rect.x + rect.w {
            let idx = row + x;
            pixels[idx] = pixels[idx].under(0xFF_FF_FF_FF, BlendMode::Normal);
        }
    }
}

/// 32×-supersampled area-chart renderer. Each column fills from the curve's pixel-y down to the bottom of the plot rect — no line, no baseline strip, just a filled region whose top edge is the curve.
///
/// For each subsample (32 per column):
/// - `curve_frac < 0` (curve below the view): contributes nothing this subsample.
/// - `curve_frac > 1` (curve above the view, off-screen top): fills the entire on-screen column at full strength.
/// - else: drops to f32 to compute `curve_pix_y` and AAs the top row by its sub-pixel fraction; rows below get full coverage.
///
/// The case split runs in S43 — `curve_frac` can be far outside `[0, 1]` (curve evaluator can return any Spirix state), and dropping to f32 first would saturate the test. Cost is `32 * rect.w` curve evaluations per frame.
pub fn draw_curve(
    pixels: &mut [u32],
    window_width: usize,
    rect: Rect,
    view: PlotView,
    curve: impl Fn(S43) -> S43,
) {
    const SUBSAMPLES: usize = 32;
    // Opaque fills in darkness convention (complement of the visible magenta/yellow/green).
    const FILL_UNDEFINED: u32 = 0xFF00_0000 | darken(0x00_E0_00_E0); // magenta
    const FILL_INFINITY: u32 = 0xFF00_0000 | darken(0x00_E0_E0_00); // yellow
    const FILL_ZERO: u32 = 0xFF00_0000 | darken(0x00_00_E0_00); // green
    // Darkness RGB of the normal pink (positive) / blue (negative) fills.
    let pos_rgb = darken(state_colour(S43::ONE)); // FF8080 → darkness
    let neg_rgb = darken(state_colour(S43::NEG_ONE)); // 8080FF → darkness

    let x_range = view.x_max - view.x_min;
    let y_range = view.y_max - view.y_min;
    let x_scale = x_range / rect.w;
    let frac_scale = x_scale >> 5;

    let rect_y = rect.y as f32;
    let rect_h = rect.h as f32;
    let bottom_row = rect.y + rect.h;

    // Per-row supersample coverage for this column, split by sign so a zero-crossing
    // column blends pink + blue correctly. Each subsample contributes up to 1.0 per row
    // (fractional on the curve's sub-pixel top row); the column's alpha is coverage/32.
    let mut cov_pos = vec![0f32; rect.h];
    let mut cov_neg = vec![0f32; rect.h];

    for px in 0..rect.w {
        cov_pos.iter_mut().for_each(|c| *c = 0.0);
        cov_neg.iter_mut().for_each(|c| *c = 0.0);
        let abs_px = rect.x + px;
        let x = px * x_scale;
        let mut fill: Option<u32> = None;
        // Whether the column fill runs from the y=0 line down (true) or full-height (false).
        let mut fill_from_zero = false;

        for ss in 0..SUBSAMPLES {
            let frac_x = x + ss * frac_scale;
            let world_x = view.x_min + frac_x;
            let world_y = curve(world_x);

            // Off-scale / singular states fill the column as a solid marker rather than
            // joining the area strip. Full-height: undefined (magenta), infinity (yellow),
            // exploded (phase-scaled red/blue) — blow past the view. From y=0 down: zero
            // (green), vanished (phase-scaled red/blue) — ≈0, so they fill what a
            // zero-valued curve would. `0xFF00_0000 |` makes the phase colour opaque.
            if world_y.is_undefined() {
                fill = Some(FILL_UNDEFINED);
                break;
            }
            if world_y.is_infinite() {
                fill = Some(FILL_INFINITY);
                break;
            }
            if world_y.is_zero() {
                fill = Some(FILL_ZERO);
                fill_from_zero = true;
                break;
            }
            if world_y.is_exploded() {
                fill = Some(0xFF00_0000 | darken(state_colour(world_y)));
                break;
            }
            if world_y.is_vanished() {
                fill = Some(0xFF00_0000 | darken(state_colour(world_y)));
                fill_from_zero = true;
                break;
            }

            // Normal value: accumulate area coverage from the curve's pixel-y down.
            // `curve_frac` can be far outside [0, 1], so the case test runs in S43 before
            // we drop to f32.
            let curve_frac = (world_y - view.y_min) / y_range;
            if curve_frac < 0 {
                continue; // curve below the view → no fill this subsample
            }
            let cov = if world_y.is_positive() {
                &mut cov_pos
            } else {
                &mut cov_neg
            };
            if curve_frac > 1 {
                // Curve above the view → fills the whole on-screen column.
                cov.iter_mut().for_each(|c| *c += 1.0);
            } else {
                let frac_f = curve_frac.to_f32();
                let curve_pix_y = rect_y + (1. - frac_f) * rect_h;
                let top_row = curve_pix_y as usize;
                let top_cov = 1. - (curve_pix_y - curve_pix_y.floor());
                if top_row >= rect.y && top_row < bottom_row {
                    cov[top_row - rect.y] += top_cov;
                }
                for r in (top_row + 1).max(rect.y)..bottom_row {
                    cov[r - rect.y] += 1.0;
                }
            }
        }

        if let Some(colour) = fill {
            // `fill_from_zero` (zero + vanished) → from the y=0 line down, the area a ≈0
            // curve covers. Otherwise (undefined / infinity / exploded) → full column.
            let top = if fill_from_zero {
                let zero_py = map_y(view, rect, S43::ZERO);
                zero_py.clamp(rect.y as i32, bottom_row as i32) as usize
            } else {
                rect.y
            };
            for r in top..bottom_row {
                let idx = r * window_width + abs_px;
                pixels[idx] = pixels[idx].under(colour, BlendMode::Normal);
            }
        } else {
            // Normal area-fill: under-blend pink / blue per row at α = coverage/32 (so the
            // 32 supersamples antialias the curve's sloped top edge). The plot is drawn
            // topmost-first, so this composites UNDER the labels + grid already in place.
            let inv = 255.0 / SUBSAMPLES as f32;
            for i in 0..rect.h {
                let idx = (rect.y + i) * window_width + abs_px;
                let cp = cov_pos[i];
                if cp > 0.0 {
                    let a = (cp * inv).min(255.0) as u32;
                    pixels[idx] = pixels[idx].under((a << 24) | pos_rgb, BlendMode::Normal);
                }
                let cn = cov_neg[i];
                if cn > 0.0 {
                    let a = (cn * inv).min(255.0) as u32;
                    pixels[idx] = pixels[idx].under((a << 24) | neg_rgb, BlendMode::Normal);
                }
            }
        }
    }
}

/// Colour (24-bit `0x00RRGGBB`, visible-RGB) for a finite, non-undefined, non-infinite,
/// non-zero scalar, by magnitude class. Positive → red channel, negative → blue channel:
///   - **normal**:   fixed `FF8080` (pos) / `8080FF` (neg) — a definite, on-scale value.
///   - **vanished**: phase-scaled single channel `40..80` (pos R / neg B) — `[↓]`, dwarfed past the exponent floor.
///   - **exploded**: phase-scaled single channel `B0..FF` (pos R / neg B) — `[↑]`, past the exponent ceiling.
///
/// The phase is the value's position *within* its class, read from the fraction prefix
/// (top byte of `Scalar.fraction`). Class bit shapes (Spirix `class_xor`): positive
/// `01xxxxxx` exploded / `001xxxxx` vanished; negatives are the bitwise-NOT shapes
/// (`10xxxxxx` / `110xxxxx`), so `!prefix` maps a negative back onto the positive shape
/// and one extraction serves both signs. Spirix's `prefix()` is `pub(crate)`, so we read
/// the top byte directly via `(fraction >> 8) as i8` — adjust the shift to `(F_BITS − 8)`
/// for other Scalar widths.
fn state_colour(world_y: S43) -> u32 {
    let prefix: i8 = (world_y.fraction >> 8) as i8;
    let pos = world_y.is_positive();
    // Fold negatives onto the positive class shape so one phase extraction works for both.
    let p: u8 = if pos { prefix as u8 } else { !(prefix as u8) };

    if world_y.is_exploded() {
        // 01xxxxxx → phase ∈ [0,0x3F]; map to channel [0xB0, 0xFF].
        let phase = p.wrapping_sub(0x40) as u32 & 0x3F;
        let c = 0xB0 + phase * 0x4F / 0x3F;
        if pos { c << 16 } else { c }
    } else if world_y.is_vanished() {
        // 001xxxxx → phase ∈ [0,0x1F]; map to channel [0x40, 0x80].
        let phase = p.wrapping_sub(0x20) as u32 & 0x1F;
        let c = 0x40 + phase * 0x40 / 0x1F;
        if pos { c << 16 } else { c }
    } else {
        // Normal: definite magnitude, fixed pink (pos) / blue (neg).
        if pos { 0x00_FF_80_80 } else { 0x00_80_80_FF }
    }
}

/// World coords under a screen point inside the plot rect. Screen coords arrive from winit as f32 and convert to the plot's S43 world coords here.
pub fn screen_to_world(rect: Rect, view: PlotView, sx: f32, sy: f32) -> (S43, S43) {
    let fx = (sx - rect.x as f32) / rect.w as f32;
    let fy = (sy - rect.y as f32) / rect.h as f32;
    let wx = view.x_min + fx * (view.x_max - view.x_min);
    let wy = view.y_min + (1.0 - fy) * (view.y_max - view.y_min);
    (wx, wy)
}


#[inline]
fn map_x(view: PlotView, rect: Rect, vx: S43) -> i32 {
    let frac = ((vx - view.x_min) / (view.x_max - view.x_min)).to_f32();
    (rect.x as f32 + frac * rect.w as f32) as i32
}

#[inline]
fn map_y(view: PlotView, rect: Rect, vy: S43) -> i32 {
    let frac = ((vy - view.y_min) / (view.y_max - view.y_min)).to_f32();
    (rect.y as f32 + (1.0 - frac) * rect.h as f32) as i32
}

/// Scale-aware dyadic grid. Each axis derives its own primary spacing directly from the visible range — `primary = 2^floor(log2(width))` — and 8 halving levels descend from there with brightness 128, 64, 32, ..., 1. Level 0 draws lines at every multiple of `primary`; deeper levels add new lines at odd multiples of `primary / 2^level`, i.e. the midpoints of the previous level. No depth search and no density cutoff: line counts per axis are bounded by construction (~2× per level), so zoom-out can't wash out and zoom-in can't go blank — the grid pattern stays consistent at every scale. Lines are drawn dim-first so brighter levels overwrite dimmer at crossings.
fn draw_dyadic_grid(pixels: &mut [u32], window_width: usize, rect: Rect, view: PlotView) {

    let x_range = view.x_max - view.x_min;
    let y_range = view.y_max - view.y_min;
    if x_range <= 0 || y_range <= 0 || rect.w < 2 || rect.h < 2 {
        return;
    }

    let x_primary = primary_spacing(x_range);
    let y_primary = primary_spacing(y_range);

    // 8 levels × 2 axes = 16 draws, ordered ascending brightness so brighter levels land last and overwrite dimmer ones at intersections. Levels are already produced in descending brightness, so a simple reverse suffices. Tuple: (brightness, is_horizontal, spacing, level_zero).
    let mut queue: [(u32, bool, S43, bool); LEVELS * 2] =
        [(0, false, S43::ZERO, false); LEVELS * 2];
    let mut idx = 0;
    for level in 0..LEVELS {
        let factor = 1u32 << level;
        let brightness = TOP_BRIGHTNESS >> level;
        let level_zero = level == 0;
        queue[idx] = (brightness, false, x_primary / factor, level_zero);
        idx += 1;
        queue[idx] = (brightness, true, y_primary / factor, level_zero);
        idx += 1;
    }
    // Sort by brightness DESCENDING — under-blend is topmost-first, so the brightest lines
    // must draw first to win at crossings (the inverse of the old overwrite ordering).
    queue.sort_by_key(|&(b, _, _, _)| core::cmp::Reverse(b));

    for &(brightness, is_horizontal, spacing, level_zero) in queue.iter() {
        // Translucent white grid line in darkness convention: α = brightness, RGB = 0
        // (white). Under-blends over whatever's beneath (black bg or the curve fill), so
        // the grid reads through the fill the way the old additive version did.
        let colour = (brightness << 24) | 0x00_00_00_00;
        if is_horizontal {
            draw_y_lines(
                pixels,
                window_width,
                rect,
                view,
                spacing,
                level_zero,
                colour,
            );
        } else {
            draw_x_lines(
                pixels,
                window_width,
                rect,
                view,
                spacing,
                level_zero,
                colour,
            );
        }
    }
}

/// Tick labels at three nested grid levels (primary, primary/2, primary/4), each font 1/√2 smaller than the previous so the deepest labels are exactly half the size of the brightest. Labels anchor to the world axes (x=0 column, y=0 row) with edge fallback when the axis is offscreen. Skips the origin (k=0) on level 0 since the two label series would overlap there. Levels 1 and 2 use odd-multiple positions and so never collide with level 0.
fn draw_axis_labels(
    pixels: &mut [u32],
    text_renderer: &mut TextRenderer,
    damage: &mut Damage,
    window_width: usize,
    window_height: usize,
    rect: Rect,
    view: PlotView,
    font_size: f32,
    base: u8,
) {
    let x_range = view.x_max - view.x_min;
    let y_range = view.y_max - view.y_min;
    if x_range <= 0 || y_range <= 0 || rect.w < 8 || rect.h < 8 {
        return;
    }

    // One Canvas over the present buffer for the whole label pass. Constructed here — after all raw grid/curve pixel writes are done — so the mutable borrow of `pixels` doesn't overlap them.
    let mut canvas = Canvas::new(pixels, window_width, window_height, damage);

    let x_primary = primary_spacing(x_range);
    let y_primary = primary_spacing(y_range);

    let plot_top = rect.y as i32;
    let plot_bot = (rect.y + rect.h) as i32;
    let plot_lft = rect.x as i32;
    let plot_rgt = (rect.x + rect.w) as i32;

    // x-label baseline anchored to world-y=0 row (or nearest plot edge).
    let zero_py = map_y(view, rect, S43::ZERO);
    let x_label_baseline = if zero_py < plot_top {
        plot_top as f32 + font_size + 2.0
    } else if zero_py > plot_bot {
        plot_bot as f32 - font_size * 0.5 - 2.0
    } else {
        zero_py as f32 + font_size + 2.0
    };

    // y-label x position anchored to world-x=0 column (or nearest plot edge). When the axis is past the right edge, right-align into the plot.
    let zero_px = map_x(view, rect, S43::ZERO);
    let (y_label_x, align_left) = if zero_px < plot_lft {
        (plot_lft as f32 + 4.0, true)
    } else if zero_px > plot_rgt {
        (plot_rgt as f32 - 4.0, false)
    } else {
        (zero_px as f32 + 4.0, true)
    };

    // Three font tiers, each 1/√2 smaller — top-to-bottom is exactly 2×.
    let scale = 1.4159f32;
    let s0 = font_size;
    let s1 = s0 / scale;
    let s2 = s1 / scale;

    // Level 0 (primary), level 1 (primary/2 odd), level 2 (primary/4 odd).
    draw_x_labels_at_level(
        &mut canvas, text_renderer, rect, view, x_primary, true,
        x_label_baseline, s0, base,
    );
    draw_x_labels_at_level(
        &mut canvas, text_renderer, rect, view, x_primary >> 1u8, false,
        x_label_baseline, s1, base,
    );
    draw_x_labels_at_level(
        &mut canvas, text_renderer, rect, view, x_primary >> 2u8, false,
        x_label_baseline, s2, base,
    );

    draw_y_labels_at_level(
        &mut canvas, text_renderer, rect, view, y_primary, true,
        y_label_x, align_left, s0, base,
    );
    draw_y_labels_at_level(
        &mut canvas, text_renderer, rect, view, y_primary >> 1u8, false,
        y_label_x, align_left, s1, base,
    );
    draw_y_labels_at_level(
        &mut canvas, text_renderer, rect, view, y_primary >> 2u8, false,
        y_label_x, align_left, s2, base,
    );
}

fn draw_x_labels_at_level(
    canvas: &mut Canvas,
    text_renderer: &mut TextRenderer,
    rect: Rect,
    view: PlotView,
    spacing: S43,
    level_zero: bool,
    baseline_y: f32,
    font_size: f32,
    base: u8,
) {
    let plot_lft = rect.x as i32;
    let plot_rgt = (rect.x + rect.w) as i32;
    let (k_min, k_max, odd_only) = line_ks(view.x_min, view.x_max, spacing, level_zero);
    for k in k_min..=k_max {
        if !odd_only && k == 0 {
            continue;
        }
        let v = if odd_only { (2 * k + 1) * spacing } else { k * spacing };
        let px = map_x(view, rect, v);
        if px <= plot_lft + 4 || px >= plot_rgt - 4 {
            continue;
        }
        // Spirix Display takes the format precision as the output base — `{:4.base$}`
        // renders `v` in `base` with up to 4 digits, matching the parsing base.
        let label = format!("{:4.base$}", v, base = base as usize);
        // Plain under-blend: the plot is drawn topmost-first, so labels are painted FIRST
        // onto the transparent rect and the grid/curve/bg compose beneath them — they end
        // up on top with no hacks. Darkness-convention colour; clip to the plot rect.
        let clip = Some(Clip::new(rect.x, rect.y, rect.x + rect.w, rect.y + rect.h));
        text_renderer.draw_text_center_u32(
            canvas,
            &label,
            px as f32,
            baseline_y,
            font_size,
            400,
            theme::TEXT_COLOUR ^ 0x00FF_FFFF,
            theme::FONT_UI,
            clip,
            None,
            None,
        );
    }
}

fn draw_y_labels_at_level(
    canvas: &mut Canvas,
    text_renderer: &mut TextRenderer,
    rect: Rect,
    view: PlotView,
    spacing: S43,
    level_zero: bool,
    label_x: f32,
    align_left: bool,
    font_size: f32,
    base: u8,
) {
    let plot_top = rect.y as i32;
    let plot_bot = (rect.y + rect.h) as i32;
    let (k_min, k_max, odd_only) = line_ks(view.y_min, view.y_max, spacing, level_zero);
    for k in k_min..=k_max {
        if !odd_only && k == 0 {
            continue;
        }
        let v = if odd_only { (2 * k + 1) * spacing } else { k * spacing };
        let py = map_y(view, rect, v);
        if py <= plot_top + (font_size as i32) || py >= plot_bot - 4 {
            continue;
        }
        let label = format!("{:4.base$}", v, base = base as usize);
        let baseline_y = py as f32 - 2.0;
        // Under-blend on top (labels drawn first); darkness colour, clipped. See x-labels.
        let clip = Some(Clip::new(rect.x, rect.y, rect.x + rect.w, rect.y + rect.h));
        let colour = theme::TEXT_COLOUR ^ 0x00FF_FFFF;
        if align_left {
            text_renderer.draw_text_left_u32(
                canvas, &label, label_x, baseline_y, font_size, 400, colour,
                theme::FONT_UI, clip, None, None,
            );
        } else {
            text_renderer.draw_text_right_u32(
                canvas, &label, label_x, baseline_y, font_size, 400, colour,
                theme::FONT_UI, clip, None, None,
            );
        }
    }
}

/// Largest power of 2 not exceeding `range`. For range = 2 → 2; range = 20 → 16; range = 0.002 → 2^-9. Acts purely on the f32 exponent so it's exact for any finite positive `range`.
fn primary_spacing(range: S43) -> S43 {
    if range.is_normal() {
        S43::new(S43::ONE.fraction, range.exponent)
    } else {
        range
    }
}

fn draw_x_lines(
    pixels: &mut [u32],
    window_width: usize,
    rect: Rect,
    view: PlotView,
    spacing: S43,
    level_zero: bool,
    colour: u32,
) {
    let (k_min, k_max, odd_only) = line_ks(view.x_min, view.x_max, spacing, level_zero);
    for k in k_min..=k_max {
        let v = if odd_only {
            (2 * k + 1) * spacing
        } else {
            k * spacing
        };
        let px = map_x(view, rect, v);
        if px > rect.x as i32 && px < (rect.x + rect.w - 1) as i32 {
            draw_v_line(pixels, window_width, rect, px as usize, colour);
        }
    }
}

fn draw_y_lines(
    pixels: &mut [u32],
    window_width: usize,
    rect: Rect,
    view: PlotView,
    spacing: S43,
    level_zero: bool,
    colour: u32,
) {
    let (k_min, k_max, odd_only) = line_ks(view.y_min, view.y_max, spacing, level_zero);
    for k in k_min..=k_max {
        let v = if odd_only {
            (2 * k + 1) * spacing
        } else {
            k * spacing
        };
        let py = map_y(view, rect, v);
        if py > rect.y as i32 && py < (rect.y + rect.h - 1) as i32 {
            draw_h_line(pixels, window_width, rect, py as usize, colour);
        }
    }
}

/// Range of integer `k` whose dyadic line falls in `[lo, hi]`. `level_zero` → lines at `k * spacing` (every multiple); deeper levels → new lines at `(2k+1) * spacing` (odd multiples). Returned as `(k_min, k_max, odd_only)`.
fn line_ks(lo: S43, hi: S43, spacing: S43, level_zero: bool) -> (i64, i64, bool) {
    if level_zero {
        (
            (lo / spacing).ceil().to_i64(),
            (hi / spacing).to_i64(),
            false,
        )
    } else {
        (
            ((lo / spacing - 1u8) >> 1u8).ceil().to_i64(),
            ((hi / spacing - 1u8) >> 1u8).to_i64(),
            true,
        )
    }
}

fn draw_v_line(pixels: &mut [u32], window_width: usize, rect: Rect, x: usize, colour: u32) {
    for y in (rect.y + 1)..(rect.y + rect.h - 1) {
        let idx = y * window_width + x;
        pixels[idx] = pixels[idx].under(colour, BlendMode::Normal);
    }
}

fn draw_h_line(pixels: &mut [u32], window_width: usize, rect: Rect, y: usize, colour: u32) {
    let row = y * window_width;
    for x in (rect.x + 1)..(rect.x + rect.w - 1) {
        let idx = row + x;
        pixels[idx] = pixels[idx].under(colour, BlendMode::Normal);
    }
}
