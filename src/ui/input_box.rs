//! Formula input box, lifted from Photon's textbox pattern.
//!
//! `TextState` caches per-char pixel widths so the blinkey position and horizontal layout never disagree with cosmic-text shaping. Glyphs and the blinkey wave are composited additively (`wrapping_add` / `wrapping_sub`), which lets the diff renderer subtract the previous frame's text+blinkey and add the new ones without re-drawing the input box bg or frame.

use crate::ui::compositing::HIT_INPUT_BOX;
use crate::ui::plot::Rect;
use crate::ui::text_rasterizing::TextRenderer;
use crate::ui::theme;

pub const PROMPT: &str = "y = ";

#[derive(Clone, Default)]
pub struct TextState {
    pub chars: Vec<char>,
    pub widths: Vec<usize>,
    pub width: usize,
    pub blinkey_index: usize,
    pub focused: bool,
}

impl TextState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cursor_offset(&self) -> usize {
        self.widths[..self.blinkey_index].iter().sum()
    }

    pub fn insert(&mut self, ch: char, width: usize) {
        self.chars.insert(self.blinkey_index, ch);
        self.widths.insert(self.blinkey_index, width);
        self.width += width;
        self.blinkey_index += 1;
    }

    pub fn delete_forward(&mut self) -> bool {
        if self.blinkey_index >= self.chars.len() {
            return false;
        }
        let w = self.widths[self.blinkey_index];
        self.chars.remove(self.blinkey_index);
        self.widths.remove(self.blinkey_index);
        self.width -= w;
        true
    }

    pub fn delete_backward(&mut self) -> bool {
        if self.blinkey_index == 0 {
            return false;
        }
        self.blinkey_index -= 1;
        let w = self.widths[self.blinkey_index];
        self.chars.remove(self.blinkey_index);
        self.widths.remove(self.blinkey_index);
        self.width -= w;
        true
    }

    pub fn move_left(&mut self) -> bool {
        if self.blinkey_index == 0 {
            return false;
        }
        self.blinkey_index -= 1;
        true
    }

    pub fn move_right(&mut self) -> bool {
        if self.blinkey_index >= self.chars.len() {
            return false;
        }
        self.blinkey_index += 1;
        true
    }

    pub fn home(&mut self) -> bool {
        if self.blinkey_index == 0 {
            return false;
        }
        self.blinkey_index = 0;
        true
    }

    pub fn end(&mut self) -> bool {
        if self.blinkey_index == self.chars.len() {
            return false;
        }
        self.blinkey_index = self.chars.len();
        true
    }
}

#[derive(Clone, Copy)]
pub struct InputLayout {
    pub rect: Rect,
    pub font_size: f32,
    pub baseline_y: f32,
    pub text_start_x: f32,
    pub text_clip_right: f32,
    pub blinkey_top: usize,
    pub blinkey_height: usize,
}

impl InputLayout {
    pub fn new(rect: Rect, prompt_w: f32, font_size: f32) -> Self {
        let pad_x = (rect.h as f32 * 0.4).max(8.0);
        let baseline_y = rect.y as f32 + rect.h as f32 / 2.0;
        let text_start_x = rect.x as f32 + pad_x + prompt_w;
        let text_clip_right = (rect.x + rect.w) as f32 - pad_x;
        let blinkey_height = (font_size * 1.1) as usize;
        let blinkey_top = (baseline_y - blinkey_height as f32 / 2.0) as usize;
        Self {
            rect,
            font_size,
            baseline_y,
            text_start_x,
            text_clip_right,
            blinkey_top,
            blinkey_height,
        }
    }

    pub fn cursor_x(&self, text: &TextState) -> usize {
        (self.text_start_x as i32 + text.cursor_offset() as i32).max(self.rect.x as i32) as usize
    }
}

pub fn measure_char_width(tr: &mut TextRenderer, ch: char, font_size: f32) -> usize {
    let mut s = String::new();
    s.push(ch);
    tr.measure_text_width(
        &s,
        font_size,
        theme::FONT_WEIGHT_USER_CONTENT,
        theme::FONT_USER_CONTENT,
    ) as usize
}

/// Re-measure every char's width (call after font_size changes — e.g., on resize).
pub fn recompute_widths(text: &mut TextState, tr: &mut TextRenderer, font_size: f32) {
    let widths: Vec<usize> = text
        .chars
        .iter()
        .map(|&c| measure_char_width(tr, c, font_size))
        .collect();
    text.width = widths.iter().sum();
    text.widths = widths;
    if text.blinkey_index > text.chars.len() {
        text.blinkey_index = text.chars.len();
    }
}

/// Char index nearest a click x within the textbox.
pub fn index_from_x(text: &TextState, layout: &InputLayout, click_x: f32) -> usize {
    if text.chars.is_empty() {
        return 0;
    }
    let mut x = layout.text_start_x;
    for (i, &w) in text.widths.iter().enumerate() {
        let mid = x + w as f32 / 2.0;
        if click_x < mid {
            return i;
        }
        x += w as f32;
    }
    text.chars.len()
}

/// Draw bg + frame + prompt and (re)write the textbox mask in the input rect. Called on full redraws only. Does NOT draw text or blinkey — those are added additively afterwards.
pub fn draw_chrome(
    pixels: &mut [u32],
    hit_test_map: &mut [u8],
    mask: &mut [u8],
    window_width: usize,
    rect: Rect,
    focused: bool,
    layout: &InputLayout,
    text_renderer: &mut TextRenderer,
) {
    fill(
        pixels,
        hit_test_map,
        mask,
        window_width,
        rect,
        theme::TEXTBOX_FILL,
    );
    let (light, shadow) = if focused {
        (0xFF_8A_82_6B, theme::TEXTBOX_SHADOW_EDGE)
    } else {
        (theme::TEXTBOX_LIGHT_EDGE, theme::TEXTBOX_SHADOW_EDGE)
    };
    frame(pixels, window_width, rect, light, shadow);

    let pad_x = (rect.h as f32 * 0.4).max(8.0);
    text_renderer.draw_text_left_u32(
        pixels,
        window_width,
        PROMPT,
        rect.x as f32 + pad_x,
        layout.baseline_y,
        layout.font_size,
        500,
        theme::LABEL_COLOUR,
        theme::FONT_UI,
    );
}

/// Pre-measure the prompt's pixel width for layout calculation.
pub fn measure_prompt_width(tr: &mut TextRenderer, font_size: f32) -> f32 {
    tr.measure_text_width(PROMPT, font_size, 500, theme::FONT_UI)
}

/// Add (or subtract) every char of `text` additively. Reversible — calling once with `add_mode = true` and once with `add_mode = false` cancels out exactly.
pub fn render_text(
    pixels: &mut [u32],
    text_renderer: &mut TextRenderer,
    window_width: usize,
    text: &TextState,
    layout: &InputLayout,
    mask: &[u8],
    add_mode: bool,
) {
    let mut x = layout.text_start_x;
    for (i, &ch) in text.chars.iter().enumerate() {
        let w = text.widths[i] as f32;
        if x > layout.text_clip_right {
            break;
        }
        text_renderer.render_char_additive_u32(
            pixels,
            window_width,
            ch,
            x,
            layout.baseline_y,
            layout.font_size,
            theme::FONT_WEIGHT_USER_CONTENT,
            theme::FONT_USER_CONTENT,
            theme::TEXT_COLOUR,
            mask,
            add_mode,
        );
        x += w;
    }
}

/// Photon's blinkey: a vertical "comet" of brightness with a horizontal glow
/// falling off as `1 / 2^|x|`. Two variants — `top_bright` true means the bright half is at the top of the wave, false at the bottom — picked at random per blink so the cursor visually shimmers.
pub fn render_blinkey(
    pixels: &mut [u32],
    window_width: usize,
    blinkey_x: usize,
    blinkey_top: usize,
    blinkey_height: usize,
    top_bright: bool,
    add_mode: bool,
) {
    if blinkey_height < 2 {
        return;
    }
    let half = (blinkey_height / 2) as isize;
    let buffer_len = pixels.len();
    for y in blinkey_top..(blinkey_top + blinkey_height) {
        let t = (y as isize - blinkey_top as isize - half) as f32 / half as f32;
        let wave_top = (1.0 - t * t) * (1.0 - t) * (1.0 - t);
        let wave_bot = (1.0 - t * t) * (1.0 + t) * (1.0 + t);
        let wave = (if top_bright { wave_top } else { wave_bot }) * theme::CURSOR_BRIGHTNESS;
        if wave <= 0.0 {
            continue;
        }
        let wave_u = wave as u32;
        let row = y * window_width;
        for dx in -7isize..=7 {
            let xa = dx.unsigned_abs();
            let v = 0x00_01_01_01u32.wrapping_mul(wave_u >> xa);
            let xpos = blinkey_x as isize + dx;
            if xpos < 0 || (xpos as usize) >= window_width {
                continue;
            }
            let idx = row + xpos as usize;
            if idx >= buffer_len {
                continue;
            }
            if add_mode {
                pixels[idx] = pixels[idx].wrapping_add(v);
            } else {
                pixels[idx] = pixels[idx].wrapping_sub(v);
            }
        }
    }
}

fn fill(pixels: &mut [u32], hit: &mut [u8], mask: &mut [u8], width: usize, r: Rect, colour: u32) {
    for y in r.y..r.y + r.h {
        let row = y * width;
        for x in r.x..r.x + r.w {
            pixels[row + x] = colour;
            hit[row + x] = HIT_INPUT_BOX;
            mask[row + x] = 255;
        }
    }
}

fn frame(pixels: &mut [u32], window_width: usize, r: Rect, light: u32, shadow: u32) {
    let top = r.y * window_width;
    let bot = (r.y + r.h - 1) * window_width;
    for x in r.x..r.x + r.w {
        pixels[top + x] = light;
        pixels[bot + x] = shadow;
    }
    for y in r.y..r.y + r.h {
        pixels[y * window_width + r.x] = light;
        pixels[y * window_width + r.x + r.w - 1] = shadow;
    }
}
