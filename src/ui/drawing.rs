//! Shared drawing primitives used across platforms
//!
//! These functions are platform-independent and work on any &mut [u32] pixel buffer.

use super::theme;

/// Photon's signature animated background texture
///
/// Creates a symmetric procedural texture with organic noise patterns.
/// The `fullscreen` parameter controls whether edge pixels are drawn
/// (false = leave 1px border for window edges on desktop).
///
/// # Arguments
/// * `pixels` - ARGB pixel buffer (0xAARRGGBB format)
/// * `width` - Buffer width in pixels
/// * `height` - Buffer height in pixels
/// * `speckle` - Animation counter for speckle effect (0 for static)
/// * `fullscreen` - If true, draw all pixels including edges
#[cfg(not(target_os = "android"))]
pub fn draw_background_texture(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    speckle: usize,
    fullscreen: bool,
    scroll_offset: isize,
) {
    use rayon::prelude::*;

    // When fullscreen, fill all pixels including edges
    let (row_start, row_end, x_start, x_end) = if fullscreen {
        (0, height, 0, width)
    } else {
        (1, height - 1, 1, width - 1)
    };

    let rows = &mut pixels[row_start * width..row_end * width];

    rows.par_chunks_mut(width)
        .enumerate()
        .for_each(|(row_idx, row_pixels)| {
            // WHY: Scroll offset shifts which logical row this screen row represents
            // PROOF: Adding scroll_offset to row index makes texture scroll with content
            // PREVENTS: Background staying static while content scrolls
            let logical_row = (row_start + row_idx) as isize - scroll_offset;
            draw_background_row(
                row_pixels,
                width,
                logical_row,
                height,
                x_start,
                x_end,
                speckle,
            );
        });
}

/// Android version - always fullscreen, no rayon (sequential for now)
#[cfg(target_os = "android")]
pub fn draw_background_texture(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    speckle: usize,
    _fullscreen: bool, // Android is always fullscreen
    scroll_offset: isize,
) {
    for row_idx in 0..height {
        let row_start = row_idx * width;
        let row_end = row_start + width;
        let row_pixels = &mut pixels[row_start..row_end];
        let logical_row = row_idx as isize - scroll_offset;
        draw_background_row(row_pixels, width, logical_row, height, 0, width, speckle);
    }
}

/// Draw a single row of the background texture
/// This is the core algorithm shared between platforms
#[inline]
fn draw_background_row(
    row_pixels: &mut [u32],
    width: usize,
    logical_row: isize,
    height: usize,
    x_start: usize,
    x_end: usize,
    speckle: usize,
) {
    // WHY: logical_row can be negative when scrolled, use wrapping for RNG seed
    // PROOF: wrapping_sub produces consistent hash for any scroll position
    // PREVENTS: Different behavior for negative vs positive row indices
    let mut rng: usize = (0xDEADBEEF01234567)
        ^ ((logical_row as usize).wrapping_sub(height / 2).wrapping_mul(0x9E3779B94517B397));
    let mask = theme::BG_MASK;
    let alpha = theme::BG_ALPHA;
    let ones = 0x00010101;
    let base = theme::BG_BASE;
    let speckle_colour = theme::BG_SPECKLE;
    let mut colour = rng as u32 & mask | alpha;

    // Right half: left-to-right
    for x in width / 2..x_end {
        rng ^= rng.rotate_left(13).wrapping_add(12345678942);
        let adder = rng as u32 & ones;
        if rng.wrapping_add(speckle) < usize::MAX / 256 {
            colour = rng as u32 >> 8 & speckle_colour | alpha;
        } else {
            colour = colour.wrapping_add(adder) & mask;
            let subtractor = (rng >> 5) as u32 & ones;
            colour = colour.wrapping_sub(subtractor) & mask;
        }
        row_pixels[x] = colour.wrapping_add(base) | alpha;
    }

    // Left half: right-to-left (mirror)
    rng = 0xDEADBEEF01234567
        ^ ((logical_row as usize).wrapping_sub(height / 2).wrapping_mul(0x9E3779B94517B397));
    colour = rng as u32 & mask | alpha;

    for x in (x_start..width / 2).rev() {
        rng ^= rng.rotate_left(13).wrapping_sub(12345678942);
        let adder = rng as u32 & ones;
        if rng.wrapping_add(speckle) < usize::MAX / 256 {
            colour = rng as u32 >> 8 & speckle_colour | alpha;
        } else {
            colour = colour.wrapping_add(adder) & mask;
            let subtractor = (rng >> 5) as u32 & ones;
            colour = colour.wrapping_sub(subtractor) & mask;
        }
        row_pixels[x] = colour.wrapping_add(base) | alpha;
    }
}
