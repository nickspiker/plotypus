//! Chrome rendering: window edges, squircle corner mask, top-right window controls.
//! Lifted from Photon's compositing.rs (functions only — no PhotonApp coupling).

use crate::ui::app::PlotypusApp;
use crate::ui::theme;

pub const HIT_NONE: u8 = 0;
pub const HIT_MINIMIZE_BUTTON: u8 = 1;
pub const HIT_MAXIMIZE_BUTTON: u8 = 2;
pub const HIT_CLOSE_BUTTON: u8 = 3;
pub const HIT_BODY: u8 = 4;
pub const HIT_INPUT_BOX: u8 = 5;
pub const HIT_PLOT_AREA: u8 = 6;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub const PREMULTIPLIED: bool = true;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub const PREMULTIPLIED: bool = false;

impl PlotypusApp {
    /// Compute squircle corner geometry and button bounds without drawing.
    /// Returns (corner_start, crossings, button_x_start_with_offset, button_height).
    pub fn window_controls_bounds(
        window_width: u32,
        window_height: u32,
        ru: f32,
    ) -> (usize, Vec<(u16, u8, u8)>, usize, usize) {
        let window_width = window_width as usize;
        let window_height = window_height as usize;

        let span = 2.0 * window_width as f32 * window_height as f32
            / (window_width as f32 + window_height as f32);
        let button_height = (span / 32.0 * ru).ceil() as usize;
        let button_width = button_height;
        let total_width = button_width * 7 / 2;

        let x_start = window_width - total_width;

        let radius = span * ru / 4.;
        let squirdleyness = 24;

        let mut crossings: Vec<(u16, u8, u8)> = Vec::new();
        let mut y = 1f32;
        loop {
            let y_norm = y / radius;
            let x_norm = (1.0 - y_norm.powi(squirdleyness)).powf(1.0 / squirdleyness as f32);
            let x = x_norm * radius;
            let inset = radius - x;
            if inset > 0. {
                crossings.push((
                    inset as u16,
                    (inset.fract().sqrt() * 256.) as u8,
                    ((1. - inset.fract()).sqrt() * 256.) as u8,
                ));
            }
            if x < y {
                break;
            }
            y += 1.;
        }
        let start = (radius - y) as usize;
        let crossings: Vec<(u16, u8, u8)> = crossings.into_iter().rev().collect();

        (start, crossings, x_start + button_width / 4, button_height)
    }

    /// Draw the three top-right window controls (minimize, maximize, close) and
    /// populate hit_test_map for them.
    pub fn draw_window_controls(
        pixels: &mut [u32],
        hit_test_map: &mut [u8],
        window_width: u32,
        window_height: u32,
        ru: f32,
    ) -> (usize, Vec<(u16, u8, u8)>, usize, usize) {
        let window_width = window_width as usize;
        let window_height = window_height as usize;

        let span = 2.0 * window_width as f32 * window_height as f32
            / (window_width as f32 + window_height as f32);
        let button_height = (span / 32.0 * ru).ceil() as usize;
        let button_width = button_height;
        let total_width = button_width * 7 / 2;

        let mut x_start = window_width - total_width;
        let y_start = 0;

        let radius = span * ru / 4.;
        let squirdleyness = 24;

        let mut crossings: Vec<(u16, u8, u8)> = Vec::new();
        let mut y = 1f32;
        loop {
            let y_norm = y / radius;
            let x_norm = (1.0 - y_norm.powi(squirdleyness)).powf(1.0 / squirdleyness as f32);
            let x = x_norm * radius;
            let inset = radius - x;
            if inset > 0. {
                crossings.push((
                    inset as u16,
                    (inset.fract().sqrt() * 256.) as u8,
                    ((1. - inset.fract()).sqrt() * 256.) as u8,
                ));
            }
            if x < y {
                break;
            }
            y += 1.;
        }
        let start = (radius - y) as usize;
        let crossings: Vec<(u16, u8, u8)> = crossings.into_iter().rev().collect();

        let edge_colour = theme::WINDOW_LIGHT_EDGE;
        let bg_colour = theme::WINDOW_CONTROLS_BG;

        // Left squircle edge of the controls strip
        let mut y_offset = start;
        for (inset, l, h) in &crossings {
            if y_offset >= button_height {
                break;
            }
            let py = y_start + button_height - 1 - y_offset;

            let col_end = total_width.min(window_width - x_start);
            for col in (*inset as usize + 2)..col_end - 1 {
                let px = x_start + col;
                let pixel_idx = py * window_width + px;
                pixels[pixel_idx] = bg_colour;

                let button_area_x_start = x_start + button_width / 4;
                let button_id = if px < button_area_x_start {
                    HIT_MINIMIZE_BUTTON
                } else {
                    let x_in_button_area = px - button_area_x_start;
                    if x_in_button_area < button_width {
                        HIT_MINIMIZE_BUTTON
                    } else if x_in_button_area < button_width * 2 {
                        HIT_MAXIMIZE_BUTTON
                    } else {
                        HIT_CLOSE_BUTTON
                    }
                };
                hit_test_map[pixel_idx] = button_id;
            }

            let px = x_start + *inset as usize;
            let pixel_idx = py * window_width + px;
            pixels[pixel_idx] = blend_rgb_only(pixels[pixel_idx], edge_colour, *l, *h);

            let px = x_start + *inset as usize + 1;
            let pixel_idx = py * window_width + px;
            pixels[pixel_idx] = blend_rgb_only(bg_colour, edge_colour, *h, *l);

            let button_area_x_start = x_start + button_width / 4;
            let button_id = if px < button_area_x_start {
                HIT_MINIMIZE_BUTTON
            } else {
                let x_in_button_area = px - button_area_x_start;
                if x_in_button_area < button_width {
                    HIT_MINIMIZE_BUTTON
                } else if x_in_button_area < button_width * 2 {
                    HIT_MAXIMIZE_BUTTON
                } else {
                    HIT_CLOSE_BUTTON
                }
            };
            hit_test_map[pixel_idx] = button_id;

            y_offset += 1;
        }

        // Bottom edge of the controls strip
        let mut x_offset = start;
        let crossing_limit = crossings.len().min(window_width - (x_start + start));
        for &(inset, l, h) in &crossings[..crossing_limit] {
            let i = inset as usize;
            let px = x_start + x_offset;

            let py = y_start + button_height - 1 - i;
            let pixel_idx = py * window_width + px;
            pixels[pixel_idx] = blend_rgb_only(pixels[pixel_idx], edge_colour, l, h);

            for row in (i + 2)..start {
                let py = y_start + button_height - 1 - row;
                let pixel_idx = py * window_width + px;
                pixels[pixel_idx] = bg_colour;

                let button_area_x_start = x_start + button_width / 4;
                let button_id = if px < button_area_x_start {
                    HIT_MINIMIZE_BUTTON
                } else {
                    let x_in_button_area = px - button_area_x_start;
                    if x_in_button_area < button_width {
                        HIT_MINIMIZE_BUTTON
                    } else if x_in_button_area < button_width * 2 {
                        HIT_MAXIMIZE_BUTTON
                    } else {
                        HIT_CLOSE_BUTTON
                    }
                };
                hit_test_map[pixel_idx] = button_id;
            }

            let py = y_start + button_height - 1 - (i + 1);
            let pixel_idx = py * window_width + px;
            pixels[pixel_idx] = blend_rgb_only(bg_colour, edge_colour, h, l);

            let button_area_x_start = x_start + button_width / 4;
            let button_id = if px < button_area_x_start {
                HIT_MINIMIZE_BUTTON
            } else {
                let x_in_button_area = px - button_area_x_start;
                if x_in_button_area < button_width {
                    HIT_MINIMIZE_BUTTON
                } else if x_in_button_area < button_width * 2 {
                    HIT_MAXIMIZE_BUTTON
                } else {
                    HIT_CLOSE_BUTTON
                }
            };
            hit_test_map[pixel_idx] = button_id;

            x_offset += 1;
        }

        // Linear extension of bottom edge to the right window border
        let linear_start_x = x_start + start + crossings.len();
        let edge_y = y_start + button_height - 1;
        for px in linear_start_x..window_width {
            let pixel_idx = edge_y * window_width + px;
            pixels[pixel_idx] = edge_colour;

            for row in 1..start {
                let py = edge_y - row;
                let pixel_idx = py * window_width + px;
                pixels[pixel_idx] = bg_colour;
                hit_test_map[pixel_idx] = HIT_CLOSE_BUTTON;
            }
        }

        x_start += button_width / 4;

        let (r, g, b, _a) = unpack_argb(theme::MINIMIZE_GLYPH);
        let minimize_colour = (r, g, b);
        Self::draw_minimize_symbol(
            pixels,
            window_width,
            x_start + button_width / 2,
            y_start + button_width / 2,
            button_width / 4,
            minimize_colour,
        );

        let (r, g, b, _a) = unpack_argb(theme::MAXIMIZE_GLYPH);
        let maximize_colour = (r, g, b);
        let (r, g, b, _a) = unpack_argb(theme::MAXIMIZE_GLYPH_INTERIOR);
        let maximize_interior = (r, g, b);
        Self::draw_maximize_symbol(
            pixels,
            window_width,
            x_start + button_width + button_width / 2,
            y_start + button_width / 2,
            button_width / 4,
            maximize_colour,
            maximize_interior,
        );

        let (r, g, b, _a) = unpack_argb(theme::CLOSE_GLYPH);
        let close_colour = (r, g, b);
        Self::draw_close_symbol(
            pixels,
            window_width,
            x_start + button_width * 2 + button_width / 2,
            y_start + button_width / 2,
            button_width / 4,
            close_colour,
        );
        (start, crossings, x_start, button_height)
    }

    pub fn draw_minimize_symbol(
        pixels: &mut [u32],
        width: usize,
        x: usize,
        y: usize,
        r: usize,
        stroke_colour: (u8, u8, u8),
    ) {
        let r = r + 1;
        let r_render = r / 4 + 1;
        let r_2 = r_render * r_render;
        let r_4 = r_2 * r_2;
        let r_3 = r_render * r_render * r_render;

        let stroke_packed = pack_argb(stroke_colour.0, stroke_colour.1, stroke_colour.2, 255);

        for h in -(r_render as isize)..=(r_render as isize) {
            for w in -(r as isize)..=(r as isize) {
                let h2 = h * h;
                let h4 = h2 * h2;
                let a = (w.abs() - (r * 3 / 4) as isize).max(0);
                let w2 = a * a;
                let w4 = w2 * w2;
                let dist_4 = (h4 + w4) as usize;

                if dist_4 <= r_4 {
                    let px = (x as isize + w) as usize;
                    let py = (y as isize + h + (r / 2) as isize) as usize;
                    let idx = py * width + px;
                    let gradient = ((r_4 - dist_4) << 8) / (r_3 << 2);
                    if gradient > 255 {
                        pixels[idx] = stroke_packed;
                    } else {
                        let alpha = gradient as u64;
                        let inv_alpha = 256 - alpha;

                        let mut bg = pixels[idx] as u64;
                        bg = (bg | (bg << 16)) & 0x0000FFFF0000FFFF;
                        bg = (bg | (bg << 8)) & 0x00FF00FF00FF00FF;

                        let mut stroke = stroke_packed as u64;
                        stroke = (stroke | (stroke << 16)) & 0x0000FFFF0000FFFF;
                        stroke = (stroke | (stroke << 8)) & 0x00FF00FF00FF00FF;

                        let mut blended = bg * inv_alpha + stroke * alpha;
                        blended = (blended >> 8) & 0x00FF00FF00FF00FF;
                        blended = (blended | (blended >> 8)) & 0x0000FFFF0000FFFF;
                        blended = blended | (blended >> 16);
                        pixels[idx] = blended as u32;
                    }
                }
            }
        }
    }

    pub fn draw_maximize_symbol(
        pixels: &mut [u32],
        width: usize,
        x: usize,
        y: usize,
        r: usize,
        stroke_colour: (u8, u8, u8),
        fill_colour: (u8, u8, u8),
    ) {
        let r = r + 1;
        let mut r_4 = r * r;
        r_4 *= r_4;
        let r_3 = r * r * r;

        let r_inner = r * 4 / 5;
        let mut r_inner_4 = r_inner * r_inner;
        r_inner_4 *= r_inner_4;
        let r_inner_3 = r_inner * r_inner * r_inner;

        let outer_edge_threshold = r_3 << 2;
        let inner_edge_threshold = r_inner_3 << 2;

        let stroke_packed = pack_argb(stroke_colour.0, stroke_colour.1, stroke_colour.2, 255);
        let fill_packed = pack_argb(fill_colour.0, fill_colour.1, fill_colour.2, 255);

        for h in -(r as isize)..=r as isize {
            for w in -(r as isize)..=r as isize {
                let h2 = h * h;
                let h4 = h2 * h2;
                let w2 = w * w;
                let w4 = w2 * w2;
                let dist_4 = (h4 + w4) as usize;

                if dist_4 <= r_4 {
                    let px = (x as isize + w) as usize;
                    let py = (y as isize + h) as usize;
                    let idx = py * width + px;

                    let dist_from_outer = r_4 - dist_4;

                    if dist_4 <= r_inner_4 {
                        let dist_from_inner = r_inner_4 - dist_4;
                        if dist_from_inner <= inner_edge_threshold {
                            let gradient = ((dist_from_inner) << 8) / inner_edge_threshold;
                            let alpha = gradient as u64;
                            let inv_alpha = 256 - alpha;

                            let mut stroke = stroke_packed as u64;
                            stroke = (stroke | (stroke << 16)) & 0x0000FFFF0000FFFF;
                            stroke = (stroke | (stroke << 8)) & 0x00FF00FF00FF00FF;

                            let mut fill = fill_packed as u64;
                            fill = (fill | (fill << 16)) & 0x0000FFFF0000FFFF;
                            fill = (fill | (fill << 8)) & 0x00FF00FF00FF00FF;

                            let mut blended = stroke * inv_alpha + fill * alpha;
                            blended = (blended >> 8) & 0x00FF00FF00FF00FF;
                            blended = (blended | (blended >> 8)) & 0x0000FFFF0000FFFF;
                            blended = blended | (blended >> 16);
                            pixels[idx] = blended as u32;
                        } else {
                            pixels[idx] = fill_packed;
                        }
                    } else {
                        if dist_from_outer <= outer_edge_threshold {
                            let gradient = ((dist_from_outer) << 8) / outer_edge_threshold;
                            let alpha = gradient as u64;
                            let inv_alpha = 256 - alpha;

                            let mut bg = pixels[idx] as u64;
                            bg = (bg | (bg << 16)) & 0x0000FFFF0000FFFF;
                            bg = (bg | (bg << 8)) & 0x00FF00FF00FF00FF;

                            let mut stroke = stroke_packed as u64;
                            stroke = (stroke | (stroke << 16)) & 0x0000FFFF0000FFFF;
                            stroke = (stroke | (stroke << 8)) & 0x00FF00FF00FF00FF;

                            let mut blended = bg * inv_alpha + stroke * alpha;
                            blended = (blended >> 8) & 0x00FF00FF00FF00FF;
                            blended = (blended | (blended >> 8)) & 0x0000FFFF0000FFFF;
                            blended = blended | (blended >> 16);
                            pixels[idx] = blended as u32;
                        } else {
                            pixels[idx] = stroke_packed;
                        }
                    }
                }
            }
        }
    }

    pub fn draw_close_symbol(
        pixels: &mut [u32],
        width: usize,
        x: usize,
        y: usize,
        r: usize,
        stroke_colour: (u8, u8, u8),
    ) {
        let r = r + 1;
        let thickness = (r / 3).max(1) as f32;
        let radius = thickness / 2.;
        let size = (r * 2) as f32;
        let cxf = x as f32;
        let cyf = y as f32;

        let end = size / 3.;

        let x1_start = cxf - end;
        let y1_start = cyf - end;
        let x1_end = cxf + end;
        let y1_end = cyf + end;

        let x2_start = cxf + end;
        let y2_start = cyf - end;
        let x2_end = cxf - end;
        let y2_end = cyf + end;

        let stroke_packed = pack_argb(stroke_colour.0, stroke_colour.1, stroke_colour.2, 255);

        let min_x = ((x as i32) - (r as i32)).max(0);
        let max_x = ((x as i32) + (r as i32)).min(width as i32);
        let min_y = ((y as i32) - (r as i32)).max(0);
        let max_y = ((y as i32) + (r as i32)).min(width as i32);

        let cxi = x as i32;
        let cyi = y as i32;

        let mut blend_at = |px: i32, py: i32, x1s: f32, y1s: f32, x1e: f32, y1e: f32| {
            let px_f = px as f32 + 0.5;
            let py_f = py as f32 + 0.5;
            let dist = Self::distance_to_capsule(px_f, py_f, x1s, y1s, x1e, y1e, radius);
            let alpha_f = if dist < -0.5 {
                1.
            } else if dist < 0.5 {
                0.5 - dist
            } else {
                0.
            };
            if alpha_f > 0. {
                let idx = py as usize * width + px as usize;
                let alpha = (alpha_f * 256.0) as u64;
                let inv_alpha = 256 - alpha;

                let mut bg = pixels[idx] as u64;
                bg = (bg | (bg << 16)) & 0x0000FFFF0000FFFF;
                bg = (bg | (bg << 8)) & 0x00FF00FF00FF00FF;

                let mut stroke = stroke_packed as u64;
                stroke = (stroke | (stroke << 16)) & 0x0000FFFF0000FFFF;
                stroke = (stroke | (stroke << 8)) & 0x00FF00FF00FF00FF;

                let mut blended = bg * inv_alpha + stroke * alpha;
                blended = (blended >> 8) & 0x00FF00FF00FF00FF;
                blended = (blended | (blended >> 8)) & 0x0000FFFF0000FFFF;
                blended = blended | (blended >> 16);
                pixels[idx] = blended as u32;
            }
        };

        for py in min_y..cyi {
            for px in min_x..cxi {
                blend_at(px, py, x1_start, y1_start, x1_end, y1_end);
            }
        }
        for py in min_y..cyi {
            for px in cxi..max_x {
                blend_at(px, py, x2_start, y2_start, x2_end, y2_end);
            }
        }
        for py in cyi..max_y {
            for px in min_x..cxi {
                blend_at(px, py, x2_start, y2_start, x2_end, y2_end);
            }
        }
        for py in cyi..max_y {
            for px in cxi..max_x {
                blend_at(px, py, x1_start, y1_start, x1_end, y1_end);
            }
        }
    }

    pub fn distance_to_capsule(
        px: f32,
        py: f32,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        radius: f32,
    ) -> f32 {
        let dx = x2 - x1;
        let dy = y2 - y1;
        let len_sq = dx * dx + dy * dy;
        let t = ((px - x1) * dx + (py - y1) * dy) / len_sq;
        let t_clamped = t.clamp(0., 1.);
        let closest_x = x1 + t_clamped * dx;
        let closest_y = y1 + t_clamped * dy;
        let dist_x = px - closest_x;
        let dist_y = py - closest_y;
        (dist_x * dist_x + dist_y * dist_y).sqrt() - radius
    }

    /// Vertical hairline separators between the three top-right control buttons.
    /// Two lines: at `button_x_start + button_width` (minimize|maximize) and
    /// at `button_x_start + button_width * 2` (maximize|close). Each line walks
    /// from vertical centre toward the squircle and stops when the pixel under
    /// it changes colour (i.e. when it hits the corner mask).
    pub fn draw_button_hairlines(
        pixels: &mut [u32],
        hit_test_map: &mut [u8],
        window_width: u32,
        button_x_start: usize,
        button_height: usize,
    ) {
        let width = window_width as usize;
        let y_start = 0usize;
        let button_width = button_height;

        let left_px = button_x_start + button_width;
        let right_px = button_x_start + button_width * 2;

        let center_y = y_start + button_height / 2;
        let edge_colour = theme::WINDOW_CONTROLS_HAIRLINE;

        // Left hairline
        let center_colour = pixels[center_y * width + left_px];
        for py in (y_start..=center_y).rev() {
            let idx = py * width + left_px;
            let diff = pixels[idx] != center_colour;
            pixels[idx] = edge_colour;
            hit_test_map[idx] = HIT_NONE;
            if diff {
                break;
            }
        }
        for py in (center_y + 1)..(y_start + button_height) {
            let idx = py * width + left_px;
            let diff = pixels[idx] != center_colour;
            pixels[idx] = edge_colour;
            hit_test_map[idx] = HIT_NONE;
            if diff {
                break;
            }
        }

        // Right hairline
        let center_colour_right = pixels[center_y * width + right_px];
        for py in (y_start..=center_y).rev() {
            let idx = py * width + right_px;
            let diff = pixels[idx] != center_colour_right;
            pixels[idx] = edge_colour;
            hit_test_map[idx] = HIT_NONE;
            if diff {
                break;
            }
        }
        for py in (center_y + 1)..(y_start + button_height) {
            let idx = py * width + right_px;
            let diff = pixels[idx] != center_colour_right;
            pixels[idx] = edge_colour;
            hit_test_map[idx] = HIT_NONE;
            if diff {
                break;
            }
        }
    }

    /// Apply hover tint to every pixel of the hovered window-control button by
    /// scanning the hit-test map and `wrapping_add`'ing the theme delta. Photon's
    /// `draw_button_hover_by_pixels` algorithm — deltas are tuned to produce
    /// the right brightening when each channel wraps, applied uniformly to bg,
    /// edges, and glyph alike.
    pub fn apply_window_control_hover(
        pixels: &mut [u32],
        hit_test_map: &[u8],
        hit_id: u8,
        hover_delta: u32,
    ) {
        if hit_id == HIT_NONE {
            return;
        }
        for (idx, &id) in hit_test_map.iter().enumerate() {
            if id == hit_id {
                pixels[idx] = pixels[idx].wrapping_add(hover_delta);
            }
        }
    }

    /// Draw window edge hairlines and apply squircle alpha mask to the four corners.
    pub fn draw_window_edges_and_mask(
        pixels: &mut [u32],
        hit_test_map: &mut [u8],
        width: u32,
        height: u32,
        start: usize,
        crossings: &[(u16, u8, u8)],
    ) {
        let light_colour = theme::WINDOW_LIGHT_EDGE;
        let shadow_colour = theme::WINDOW_SHADOW_EDGE;
        let w = width as usize;
        let h = height as usize;

        for x in 0..w {
            pixels[x] = light_colour;
        }
        for x in 0..w {
            pixels[(h - 1) * w + x] = shadow_colour;
        }
        for y in 0..h {
            pixels[y * w] = light_colour;
        }
        for y in 0..h {
            pixels[y * w + (w - 1)] = shadow_colour;
        }

        for row in 0..start {
            for col in 0..start {
                let idx = row * w + col;
                pixels[idx] = 0;
                hit_test_map[idx] = HIT_NONE;
            }
        }
        for row in 0..start {
            for col in (w - start)..w {
                let idx = row * w + col;
                pixels[idx] = 0;
                hit_test_map[idx] = HIT_NONE;
            }
        }
        for row in (h - start)..h {
            for col in 0..start {
                let idx = row * w + col;
                pixels[idx] = 0;
                hit_test_map[idx] = HIT_NONE;
            }
        }
        for row in (h - start)..h {
            for col in (w - start)..w {
                let idx = row * w + col;
                pixels[idx] = 0;
                hit_test_map[idx] = HIT_NONE;
            }
        }

        let mut y_top = start;
        for crossing in 0..crossings.len() {
            let (inset, l, hh) = crossings[crossing];
            for idx in y_top * w..y_top * w + inset as usize {
                pixels[idx] = 0;
                hit_test_map[idx] = HIT_NONE;
            }

            let pixel_idx = y_top * w + inset as usize;
            if PREMULTIPLIED {
                pixels[pixel_idx] = scale_alpha(light_colour, hh);
            } else {
                pixels[pixel_idx] = (light_colour & 0x00FFFFFF) | ((hh as u32) << 24);
            }
            if hh < 255 {
                hit_test_map[pixel_idx] = HIT_NONE;
            }

            let pixel_idx = pixel_idx + 1;
            pixels[pixel_idx] = blend_rgb_only(pixels[pixel_idx], light_colour, hh, l);

            let pixel_idx = y_top * w + w - 2 - inset as usize;
            pixels[pixel_idx] = blend_rgb_only(pixels[pixel_idx], shadow_colour, hh, l);

            let pixel_idx = pixel_idx + 1;
            if PREMULTIPLIED {
                pixels[pixel_idx] = scale_alpha(shadow_colour, hh);
            } else {
                pixels[pixel_idx] = (shadow_colour & 0x00FFFFFF) | ((hh as u32) << 24);
            }
            if hh < 255 {
                hit_test_map[pixel_idx] = HIT_NONE;
            }

            for idx in (y_top * w + w - inset as usize)..((y_top + 1) * w) {
                pixels[idx] = 0;
                hit_test_map[idx] = HIT_NONE;
            }
            y_top += 1;
        }

        let mut y_bottom = h - start - 1;
        for crossing in 0..crossings.len() {
            let (inset, l, hh) = crossings[crossing];

            for idx in y_bottom * w..y_bottom * w + inset as usize {
                pixels[idx] = 0;
                hit_test_map[idx] = HIT_NONE;
            }

            let pixel_idx = y_bottom * w + inset as usize;
            if PREMULTIPLIED {
                pixels[pixel_idx] = scale_alpha(light_colour, hh);
            } else {
                pixels[pixel_idx] = (light_colour & 0x00FFFFFF) | ((hh as u32) << 24);
            }
            if hh < 255 {
                hit_test_map[pixel_idx] = HIT_NONE;
            }

            let pixel_idx = pixel_idx + 1;
            pixels[pixel_idx] = blend_rgb_only(pixels[pixel_idx], light_colour, hh, l);

            let pixel_idx = y_bottom * w + w - 2 - inset as usize;
            pixels[pixel_idx] = blend_rgb_only(pixels[pixel_idx], shadow_colour, hh, l);

            let pixel_idx = pixel_idx + 1;
            if PREMULTIPLIED {
                pixels[pixel_idx] = scale_alpha(shadow_colour, hh);
            } else {
                pixels[pixel_idx] = (shadow_colour & 0x00FFFFFF) | ((hh as u32) << 24);
            }
            if hh < 255 {
                hit_test_map[pixel_idx] = HIT_NONE;
            }

            for idx in (y_bottom * w + w - inset as usize)..((y_bottom + 1) * w) {
                pixels[idx] = 0;
                hit_test_map[idx] = HIT_NONE;
            }
            y_bottom -= 1;
        }

        let mut x_left = start;
        for crossing in 0..crossings.len() {
            let (inset, l, hh) = crossings[crossing];

            for row in 0..inset as usize {
                let idx = row * w + x_left;
                pixels[idx] = 0;
                hit_test_map[idx] = HIT_NONE;
            }

            let pixel_idx = inset as usize * w + x_left;
            if PREMULTIPLIED {
                pixels[pixel_idx] = scale_alpha(light_colour, hh);
            } else {
                pixels[pixel_idx] = (light_colour & 0x00FFFFFF) | ((hh as u32) << 24);
            }
            if hh < 255 {
                hit_test_map[pixel_idx] = HIT_NONE;
            }

            let pixel_idx = (inset as usize + 1) * w + x_left;
            pixels[pixel_idx] = blend_rgb_only(pixels[pixel_idx], light_colour, hh, l);

            let pixel_idx = (h - 1 - inset as usize) * w + x_left;
            if PREMULTIPLIED {
                pixels[pixel_idx] = scale_alpha(shadow_colour, hh);
            } else {
                pixels[pixel_idx] = (shadow_colour & 0x00FFFFFF) | ((hh as u32) << 24);
            }
            if hh < 255 {
                hit_test_map[pixel_idx] = HIT_NONE;
            }

            let pixel_idx = (h - 2 - inset as usize) * w + x_left;
            pixels[pixel_idx] = blend_rgb_only(pixels[pixel_idx], shadow_colour, hh, l);

            for row in (h - inset as usize)..h {
                let idx = row * w + x_left;
                pixels[idx] = 0;
                hit_test_map[idx] = HIT_NONE;
            }

            x_left += 1;
        }

        let mut x_right = w - start - 1;
        for crossing in 0..crossings.len() {
            let (inset, l, hh) = crossings[crossing];

            for row in 0..inset as usize {
                let idx = row * w + x_right;
                pixels[idx] = 0;
                hit_test_map[idx] = HIT_NONE;
            }

            let pixel_idx = inset as usize * w + x_right;
            if PREMULTIPLIED {
                pixels[pixel_idx] = scale_alpha(light_colour, hh);
            } else {
                pixels[pixel_idx] = (light_colour & 0x00FFFFFF) | ((hh as u32) << 24);
            }
            if hh < 255 {
                hit_test_map[pixel_idx] = HIT_NONE;
            }

            let pixel_idx = (inset as usize + 1) * w + x_right;
            pixels[pixel_idx] = blend_rgb_only(pixels[pixel_idx], light_colour, hh, l);

            let pixel_idx = (h - 1 - inset as usize) * w + x_right;
            if PREMULTIPLIED {
                pixels[pixel_idx] = scale_alpha(shadow_colour, hh);
            } else {
                pixels[pixel_idx] = (shadow_colour & 0x00FFFFFF) | ((hh as u32) << 24);
            }
            if hh < 255 {
                hit_test_map[pixel_idx] = HIT_NONE;
            }

            let pixel_idx = (h - 2 - inset as usize) * w + x_right;
            pixels[pixel_idx] = blend_rgb_only(pixels[pixel_idx], shadow_colour, hh, l);

            for row in (h - inset as usize)..h {
                let idx = row * w + x_right;
                pixels[idx] = 0;
                hit_test_map[idx] = HIT_NONE;
            }

            x_right -= 1;
        }
    }
}

#[inline]
pub fn pack_argb(r: u8, g: u8, b: u8, a: u8) -> u32 {
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

#[inline]
pub fn unpack_argb(pixel: u32) -> (u8, u8, u8, u8) {
    let a = (pixel >> 24) as u8;
    let r = (pixel >> 16) as u8;
    let g = (pixel >> 8) as u8;
    let b = pixel as u8;
    (r, g, b, a)
}

pub fn scale_alpha(colour: u32, alpha: u8) -> u32 {
    let mut c = colour as u64;
    c = (c | (c << 16)) & 0x0000FFFF0000FFFF;
    c = (c | (c << 8)) & 0x00FF00FF00FF00FF;
    let mut scaled = c * alpha as u64;
    scaled = (scaled >> 8) & 0x00FF00FF00FF00FF;
    scaled = (scaled | (scaled >> 8)) & 0x0000FFFF0000FFFF;
    scaled = scaled | (scaled >> 16);
    scaled as u32
}

#[inline]
pub fn blend_rgb_only(bg_colour: u32, fg_colour: u32, weight_bg: u8, weight_fg: u8) -> u32 {
    let mut bg = bg_colour as u64;
    bg = (bg | (bg << 16)) & 0x0000FFFF0000FFFF;
    bg = (bg | (bg << 8)) & 0x00FF00FF00FF00FF;

    let mut fg = fg_colour as u64;
    fg = (fg | (fg << 16)) & 0x0000FFFF0000FFFF;
    fg = (fg | (fg << 8)) & 0x00FF00FF00FF00FF;

    let mut blended = bg * weight_bg as u64 + fg * weight_fg as u64;
    blended = (blended >> 8) & 0x00FF00FF00FF00FF;
    blended = (blended | (blended >> 8)) & 0x0000FFFF0000FFFF;
    blended = blended | (blended >> 16) | 0xFF000000;

    blended as u32
}
