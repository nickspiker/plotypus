//! Plot region: axes, grid, and curve. View bounds are kept in `PlotView` so pan/zoom can mutate them without touching the rendering code.
//!
//! The plot draws straight into the host's CPU present buffer (`&mut [u32]`) that Fluor hands `FluorApp::render`. Grid + curve are raw per-pixel writes; axis labels go through Fluor's `TextRenderer`, which draws into a `Canvas` over the same buffer (constructed after the raw writes finish, so the borrows don't overlap). The plot does no hit-testing — the app routes plot interaction by rect containment, so no per-pixel hit map is threaded here.
use crate::formula::{self, Token};
use crate::ui::theme;
use fluor::canvas::{Canvas, Damage};
use fluor::text::TextRenderer;
use spirix::ScalarF4E3 as S43;

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
) {
    fill_rect(pixels, window_width, rect, 0xFF_00_00_00);
    // Parse error → blank black plot region. Skipping grid + labels + curve makes "your formula didn't parse" obvious at a glance, distinct from a valid expression that happens to evaluate offscreen.
    if parse_failed {
        visible_to_darkness_rect(pixels, window_width, rect);
        return;
    }
    draw_dyadic_grid(pixels, window_width, rect, view);
    if let Some(tokens) = formula {
        // Eval errors (stack underflow, etc.) shouldn't fire on a token vector that
        // tokenize() accepted — but if they do, fall back to INFINITY so the column
        // shows as a yellow stripe (visible, not crashy).
        let curve = |x: S43| formula::evaluate(tokens, x).unwrap_or(S43::INFINITY);
        draw_curve(pixels, window_width, rect, view, curve);
    }
    // All the grid/curve/fill math above runs in plain visible-RGB (black base, additive brightness). Flip the rect to Fluor's darkness convention once here — the single import-boundary flip the pixel-format contract expects — before the label pass (which goes through Fluor's text renderer and already speaks darkness).
    visible_to_darkness_rect(pixels, window_width, rect);
    draw_axis_labels(pixels, text_renderer, damage, window_width, window_height, rect, view, label_font_size);
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
    curve: impl Fn(S43) -> S43,
) {
    fill_rect(pixels, window_width, rect, 0xFF_00_00_00);
    draw_dyadic_grid(pixels, window_width, rect, view);
    draw_curve(pixels, window_width, rect, view, curve);
    // Flip visible-RGB → Fluor darkness convention once, before the (already-darkness) label pass. See `draw_plot`.
    visible_to_darkness_rect(pixels, window_width, rect);
    draw_axis_labels(pixels, text_renderer, damage, window_width, window_height, rect, view, label_font_size);
}

/// Complement the RGB bytes of every pixel in `rect` in place (`pixel ^= 0x00FFFFFF`), keeping α. Converts the plot's visible-RGB working buffer into Fluor's darkness-convention buffer (`0 = white potential`, `255 = black ink`) at the plot's import boundary. Called once per frame after all raw pixel writes, before text.
fn visible_to_darkness_rect(pixels: &mut [u32], window_width: usize, rect: Rect) {
    for y in rect.y..rect.y + rect.h {
        let row = y * window_width;
        for x in rect.x..rect.x + rect.w {
            pixels[row + x] ^= 0x00FF_FFFF;
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
    // Fill colours overwrite the entire column; high alpha so the result is opaque on top of the bg.
    const FILL_UNDEFINED: u32 = 0xFF_E0_00_E0; // magenta
    const FILL_INFINITY: u32 = 0xFF_E0_E0_00; // yellow
    const FILL_ZERO: u32 = 0xFF_00_E0_00; // green

    let x_range = view.x_max - view.x_min;
    let y_range = view.y_max - view.y_min;
    let x_scale = x_range / rect.w;
    let frac_scale = x_scale >> 5;

    let rect_y = rect.y as f32;
    let rect_h = rect.h as f32;
    let bottom_row = rect.y + rect.h;

    for px in 0..rect.w {
        let abs_px = rect.x + px;
        let x = px * x_scale;
        let mut fill: Option<u32> = None;

        for ss in 0..SUBSAMPLES {
            let frac_x = x + ss * frac_scale;
            let world_x = view.x_min + frac_x;
            let world_y = curve(world_x);

            // Three states overwrite the entire column rather than contribute to the strip: undefined (magenta), infinity (yellow), exact zero (green) — they're singularities the eye should catch instantly.
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
                break;
            }

            let colour = state_colour(world_y);
            let r_inc = ((colour >> 16) & 0xFF) / SUBSAMPLES as u32;
            let g_inc = ((colour >> 8) & 0xFF) / SUBSAMPLES as u32;
            let b_inc = (colour & 0xFF) / SUBSAMPLES as u32;

            // Decide the curve's relation to the view in S43 — `curve_frac` can be far outside [0, 1], so the case test must run in scalar space before we drop to f32.
            let curve_frac = (world_y - view.y_min) / y_range;
            if curve_frac < 0 {
                continue;
            }

            let (r_first, top_cov) = if curve_frac > 1 {
                (rect.y, 1.0_f32)
            } else {
                let frac_f = curve_frac.to_f32();
                let curve_pix_y = rect_y + (1. - frac_f) * rect_h;
                let top_row = curve_pix_y as usize;
                let cov = 1. - (curve_pix_y - curve_pix_y.floor());
                (top_row, cov)
            };

            for r in r_first..bottom_row {
                let coverage = if r == r_first { top_cov } else { 1. };
                let cov = (coverage * 256.) as u32;
                let contribution = (((r_inc * cov) >> 8) << 16)
                    | (((g_inc * cov) >> 8) << 8)
                    | ((b_inc * cov) >> 8);
                let idx = r * window_width + abs_px;
                pixels[idx] = pixels[idx].wrapping_add(contribution);
            }
        }

        if let Some(colour) = fill {
            for r in rect.y..bottom_row {
                let idx = r * window_width + abs_px;
                pixels[idx] = colour;
            }
        }
    }
}

/// Additive contribution colour for a finite, non-undefined, non-zero scalar. Channel goes into R for positive states, B for negative states. Brightness encodes magnitude class:
///   - vanished:  0x20..=0x5E  (faint — values dwarfed past the exponent floor)
///   - normal:    0x70         (mid — definite magnitude)
///   - exploded:  0x80..=0xFE  (bright — values past the exponent ceiling)
///
/// Within vanished/exploded the gradient is read from the fraction prefix (top byte of `Scalar.fraction`). Positive prefix bits `01xxxxxx` (exploded) or `001xxxxx` (vanished); shifting left by 1 maps those into the desired top-channel byte. Negative prefix bits `10xxxxxx` / `110xxxxx`; bit-not flips them to the positive shape, then the same shift produces the gradient. Spirix's `prefix()` is `pub(crate)`, so we read the top byte directly via `(fraction >> 8) as i8` — adjust the shift to `(F_BITS − 8)` for other Scalar widths.
fn state_colour(world_y: S43) -> u32 {
    let prefix: i8 = (world_y.fraction >> 8) as i8;
    let pos = world_y.is_positive();
    let mag: u8 = if world_y.is_exploded() {
        if pos {
            (prefix as u8) << 1
        } else {
            (!prefix as u8) << 1
        }
    } else if world_y.is_vanished() {
        if pos {
            ((prefix as u8) << 1).wrapping_sub(0x20)
        } else {
            ((!prefix as u8) << 1).wrapping_sub(0x20)
        }
    } else {
        0x70
    };
    if pos { (mag as u32) << 16 } else { mag as u32 }
}

/// World coords under a screen point inside the plot rect. Screen coords arrive from winit as f32 and convert to the plot's S43 world coords here.
pub fn screen_to_world(rect: Rect, view: PlotView, sx: f32, sy: f32) -> (S43, S43) {
    let fx = (sx - rect.x as f32) / rect.w as f32;
    let fy = (sy - rect.y as f32) / rect.h as f32;
    let wx = view.x_min + fx * (view.x_max - view.x_min);
    let wy = view.y_min + (1.0 - fy) * (view.y_max - view.y_min);
    (wx, wy)
}

fn fill_rect(pixels: &mut [u32], window_width: usize, r: Rect, colour: u32) {
    for y in r.y..r.y + r.h {
        let row = y * window_width;
        for x in r.x..r.x + r.w {
            pixels[row + x] = colour;
        }
    }
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
    let bg = 0;

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
    // Stable sort by brightness ascending. queue is short (16) so cost is trivial.
    queue.sort_by_key(|&(b, _, _, _)| b);

    for &(brightness, is_horizontal, spacing, level_zero) in queue.iter() {
        let colour = blend_white_onto(bg, brightness);
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
        x_label_baseline, s0,
    );
    draw_x_labels_at_level(
        &mut canvas, text_renderer, rect, view, x_primary >> 1u8, false,
        x_label_baseline, s1,
    );
    draw_x_labels_at_level(
        &mut canvas, text_renderer, rect, view, x_primary >> 2u8, false,
        x_label_baseline, s2,
    );

    draw_y_labels_at_level(
        &mut canvas, text_renderer, rect, view, y_primary, true,
        y_label_x, align_left, s0,
    );
    draw_y_labels_at_level(
        &mut canvas, text_renderer, rect, view, y_primary >> 1u8, false,
        y_label_x, align_left, s1,
    );
    draw_y_labels_at_level(
        &mut canvas, text_renderer, rect, view, y_primary >> 2u8, false,
        y_label_x, align_left, s2,
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
        let label = format!("{:4.12}", v);
        text_renderer.draw_text_center_u32(
            canvas,
            &label,
            px as f32,
            baseline_y,
            font_size,
            400,
            theme::TEXT_COLOUR ^ 0x00FF_FFFF,
            theme::FONT_UI,
            None,
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
        let label = format!("{:4.12}", v);
        let baseline_y = py as f32 - 2.0;
        if align_left {
            text_renderer.draw_text_left_u32(
                canvas,
                &label,
                label_x,
                baseline_y,
                font_size,
                400,
                theme::TEXT_COLOUR ^ 0x00FF_FFFF,
                theme::FONT_UI,
                None,
                None,
                None,
            );
        } else {
            text_renderer.draw_text_right_u32(
                canvas,
                &label,
                label_x,
                baseline_y,
                font_size,
                400,
                theme::TEXT_COLOUR ^ 0x00FF_FFFF,
                theme::FONT_UI,
                None,
                None,
                None,
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
        pixels[y * window_width + x] = colour;
    }
}

fn draw_h_line(pixels: &mut [u32], window_width: usize, rect: Rect, y: usize, colour: u32) {
    let row = y * window_width;
    for x in (rect.x + 1)..(rect.x + rect.w - 1) {
        pixels[row + x] = colour;
    }
}

/// Alpha-blend opaque white onto `bg` with `intensity` (0..=255 alpha).
fn blend_white_onto(bg: u32, intensity: u32) -> u32 {
    let bg_r = (bg >> 16) & 0xFF;
    let bg_g = (bg >> 8) & 0xFF;
    let bg_b = bg & 0xFF;
    let inv = 256 - intensity;
    let r = (bg_r * inv + 255 * intensity) >> 8;
    let g = (bg_g * inv + 255 * intensity) >> 8;
    let b = (bg_b * inv + 255 * intensity) >> 8;
    0xFF000000 | (r << 16) | (g << 8) | b
}
