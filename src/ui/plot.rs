//! Plot region: frame, axes, and grid. No plot data yet — that lands once the
//! Spirix-backed evaluator is wired in. View bounds are kept in `PlotView` so
//! pan/zoom can mutate them without touching the rendering code.

use crate::ui::compositing::HIT_PLOT_AREA;
use crate::ui::theme;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct PlotView {
    pub x_min: f32,
    pub x_max: f32,
    pub y_min: f32,
    pub y_max: f32,
}

impl Default for PlotView {
    fn default() -> Self {
        Self { x_min: -1., x_max: 1., y_min: -1., y_max: 1. }
    }
}

/// Number of halving levels per axis. Brightness goes 128 → 64 → ... → 1.
const LEVELS: usize = 8;
/// Brightest (level 0) line.
const TOP_BRIGHTNESS: u32 = 128;

pub fn draw_plot(
    pixels: &mut [u32],
    hit_test_map: &mut [u8],
    window_width: usize,
    rect: Rect,
    view: PlotView,
) {
    fill_rect(pixels, hit_test_map, window_width, rect, theme::PLOT_BG, HIT_PLOT_AREA);
    draw_dyadic_grid(pixels, window_width, rect, view);
    draw_frame(pixels, window_width, rect, theme::PLOT_FRAME);
}

/// World coords under a screen point inside the plot rect.
pub fn screen_to_world(rect: Rect, view: PlotView, sx: f32, sy: f32) -> (f32, f32) {
    let fx = (sx - rect.x as f32) / rect.w as f32;
    let fy = (sy - rect.y as f32) / rect.h as f32;
    let wx = view.x_min + fx * (view.x_max - view.x_min);
    let wy = view.y_min + (1.0 - fy) * (view.y_max - view.y_min);
    (wx, wy)
}

fn fill_rect(
    pixels: &mut [u32],
    hit_test_map: &mut [u8],
    window_width: usize,
    r: Rect,
    colour: u32,
    hit_id: u8,
) {
    for y in r.y..r.y + r.h {
        let row = y * window_width;
        for x in r.x..r.x + r.w {
            pixels[row + x] = colour;
            hit_test_map[row + x] = hit_id;
        }
    }
}

fn draw_frame(pixels: &mut [u32], window_width: usize, r: Rect, colour: u32) {
    let top = r.y * window_width;
    let bot = (r.y + r.h - 1) * window_width;
    for x in r.x..r.x + r.w {
        pixels[top + x] = colour;
        pixels[bot + x] = colour;
    }
    for y in r.y..r.y + r.h {
        pixels[y * window_width + r.x] = colour;
        pixels[y * window_width + r.x + r.w - 1] = colour;
    }
}

#[inline]
fn map_x(view: PlotView, rect: Rect, vx: f32) -> i32 {
    (rect.x as f32 + (vx - view.x_min) / (view.x_max - view.x_min) * rect.w as f32) as i32
}

#[inline]
fn map_y(view: PlotView, rect: Rect, vy: f32) -> i32 {
    (rect.y as f32 + (1.0 - (vy - view.y_min) / (view.y_max - view.y_min)) * rect.h as f32) as i32
}

/// Scale-aware dyadic grid. Each axis derives its own primary spacing directly
/// from the visible range — `primary = 2^floor(log2(width))` — and 8 halving
/// levels descend from there with brightness 128, 64, 32, ..., 1. Level 0
/// draws lines at every multiple of `primary`; deeper levels add new lines at
/// odd multiples of `primary / 2^level`, i.e. the midpoints of the previous
/// level. No depth search and no density cutoff: line counts per axis are
/// bounded by construction (~2× per level), so zoom-out can't wash out and
/// zoom-in can't go blank — the grid pattern stays consistent at every scale.
/// Lines are drawn dim-first so brighter levels overwrite dimmer at crossings.
fn draw_dyadic_grid(pixels: &mut [u32], window_width: usize, rect: Rect, view: PlotView) {
    let bg = theme::PLOT_BG;

    let x_range = view.x_max - view.x_min;
    let y_range = view.y_max - view.y_min;
    if x_range <= 0.0 || y_range <= 0.0 || rect.w < 2 || rect.h < 2 {
        return;
    }

    let x_primary = primary_spacing(x_range);
    let y_primary = primary_spacing(y_range);

    // 8 levels × 2 axes = 16 draws, ordered ascending brightness so brighter
    // levels land last and overwrite dimmer ones at intersections. Levels are
    // already produced in descending brightness, so a simple reverse suffices.
    // Tuple: (brightness, is_horizontal, spacing, level_zero).
    let mut queue: [(u32, bool, f32, bool); LEVELS * 2] = [(0, false, 0.0, false); LEVELS * 2];
    let mut idx = 0;
    for level in 0..LEVELS {
        let factor = (1u32 << level) as f32;
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
            draw_y_lines(pixels, window_width, rect, view, spacing, level_zero, colour);
        } else {
            draw_x_lines(pixels, window_width, rect, view, spacing, level_zero, colour);
        }
    }
}

/// Largest power of 2 not exceeding `range`. For range = 2 → 2; range = 20 →
/// 16; range = 0.002 → 2^-9. Acts purely on the f32 exponent so it's exact for
/// any finite positive `range`.
fn primary_spacing(range: f32) -> f32 {
    range.log2().floor().exp2()
}

fn draw_x_lines(
    pixels: &mut [u32],
    window_width: usize,
    rect: Rect,
    view: PlotView,
    spacing: f32,
    level_zero: bool,
    colour: u32,
) {
    let (k_min, k_max, odd_only) = line_ks(view.x_min, view.x_max, spacing, level_zero);
    for k in k_min..=k_max {
        let v = if odd_only { (2 * k + 1) as f32 * spacing } else { k as f32 * spacing };
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
    spacing: f32,
    level_zero: bool,
    colour: u32,
) {
    let (k_min, k_max, odd_only) = line_ks(view.y_min, view.y_max, spacing, level_zero);
    for k in k_min..=k_max {
        let v = if odd_only { (2 * k + 1) as f32 * spacing } else { k as f32 * spacing };
        let py = map_y(view, rect, v);
        if py > rect.y as i32 && py < (rect.y + rect.h - 1) as i32 {
            draw_h_line(pixels, window_width, rect, py as usize, colour);
        }
    }
}

/// Range of integer `k` whose dyadic line falls in `[lo, hi]`. `level_zero` →
/// lines at `k * spacing` (every multiple); deeper levels → new lines at
/// `(2k+1) * spacing` (odd multiples). Returned as `(k_min, k_max, odd_only)`.
fn line_ks(lo: f32, hi: f32, spacing: f32, level_zero: bool) -> (i64, i64, bool) {
    if level_zero {
        ((lo / spacing).ceil() as i64, (hi / spacing).floor() as i64, false)
    } else {
        (
            ((lo / spacing - 1.0) * 0.5).ceil() as i64,
            ((hi / spacing - 1.0) * 0.5).floor() as i64,
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
