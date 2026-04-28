use crate::ui::compositing::{
    HIT_CLOSE_BUTTON, HIT_MAXIMIZE_BUTTON, HIT_MINIMIZE_BUTTON,
};
use crate::ui::renderer::Renderer;
use crate::ui::text_rasterizing::TextRenderer;
use crate::ui::{compositing, drawing, theme};
use rand::Rng;
use winit::dpi::PhysicalSize;
use winit::keyboard::ModifiersState;
use winit::window::Window;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoveredButton {
    None,
    Close,
    Maximize,
    Minimize,
    Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    None,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

pub struct PlotypusApp {
    pub width: u32,
    pub height: u32,
    pub renderer: Renderer,
    pub text_renderer: TextRenderer,
    pub is_fullscreen: bool,
    pub frame_counter: u64,
    pub window_dirty: bool,
    /// Per-pixel hit-test map, indexed [y * width + x]
    pub hit_test_map: Vec<u8>,
    /// Zoom multiplier for chrome scaling (Photon's `ru` — responsive unit)
    pub ru: f32,
    /// Cached harmonic-mean dimension: 2*w*h/(w+h). Drives chrome sizing and edge thresholds.
    pub span: f32,

    // Input state
    pub mouse_x: f32,
    pub mouse_y: f32,
    pub mouse_button_pressed: bool,
    pub modifiers: ModifiersState,
    pub hovered_button: HoveredButton,
    pub prev_hovered_button: HoveredButton,
    pub is_dragging_resize: bool,
    pub resize_edge: ResizeEdge,

    // Debug overlays
    pub debug: bool,
    pub debug_hit_test: bool,
    pub show_textbox_mask: bool,
    pub debug_hit_colours: Vec<(u8, u8, u8)>,
}

fn compute_span(width: u32, height: u32) -> f32 {
    let w = width as f32;
    let h = height as f32;
    2.0 * w * h / (w + h).max(1.0)
}

impl PlotypusApp {
    pub fn new(window: &Window) -> Self {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        let renderer = Renderer::new(window, width, height);
        Self {
            width,
            height,
            renderer,
            text_renderer: TextRenderer::new(),
            is_fullscreen: false,
            frame_counter: 0,
            window_dirty: true,
            hit_test_map: vec![compositing::HIT_BODY; (width * height) as usize],
            ru: 1.0,
            span: compute_span(width, height),
            mouse_x: -1.0,
            mouse_y: -1.0,
            mouse_button_pressed: false,
            modifiers: ModifiersState::empty(),
            hovered_button: HoveredButton::None,
            prev_hovered_button: HoveredButton::None,
            is_dragging_resize: false,
            resize_edge: ResizeEdge::None,
            debug: false,
            debug_hit_test: false,
            show_textbox_mask: false,
            debug_hit_colours: Vec::new(),
        }
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.width = size.width;
        self.height = size.height;
        self.span = compute_span(size.width, size.height);
        self.renderer.resize(size.width, size.height);
        self.hit_test_map
            .resize((size.width * size.height) as usize, compositing::HIT_BODY);
        self.window_dirty = true;
    }

    pub fn set_fullscreen(&mut self, fullscreen: bool) {
        if self.is_fullscreen != fullscreen {
            self.is_fullscreen = fullscreen;
            self.window_dirty = true;
        }
    }

    /// Hit-test a pixel position; out-of-bounds returns HIT_NONE.
    pub fn hit_test(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || (x as u32) >= self.width || (y as u32) >= self.height {
            return compositing::HIT_NONE;
        }
        let idx = y as usize * self.width as usize + x as usize;
        self.hit_test_map[idx]
    }

    /// Detect resize edge proximity. Returns ResizeEdge::None outside the threshold.
    pub fn get_resize_edge(&self, x: f32, y: f32) -> ResizeEdge {
        if self.is_fullscreen {
            return ResizeEdge::None;
        }
        let border = (self.span / 32.0).ceil();
        let at_left = x < border;
        let at_right = x > (self.width as f32 - border);
        let at_top = y < border;
        let at_bottom = y > (self.height as f32 - border);

        if at_top && at_left {
            ResizeEdge::TopLeft
        } else if at_top && at_right {
            ResizeEdge::TopRight
        } else if at_bottom && at_left {
            ResizeEdge::BottomLeft
        } else if at_bottom && at_right {
            ResizeEdge::BottomRight
        } else if at_top {
            ResizeEdge::Top
        } else if at_bottom {
            ResizeEdge::Bottom
        } else if at_left {
            ResizeEdge::Left
        } else if at_right {
            ResizeEdge::Right
        } else {
            ResizeEdge::None
        }
    }

    /// Adjust ru zoom by `steps` in 33/32-per-step increments (Photon's pattern).
    pub fn adjust_zoom(&mut self, steps: f32) {
        let factor = if steps.is_sign_negative() {
            (33f32 / 32.0).powf(steps)
        } else {
            (31f32 / 32.0).powf(-steps)
        };
        self.ru = (self.ru * factor).clamp(0.125, 4.0);
        self.window_dirty = true;
    }

    /// Populate debug_hit_colours with a deterministic-enough random palette indexed by element id.
    pub fn regenerate_debug_hit_colours(&mut self) {
        let mut rng = rand::thread_rng();
        self.debug_hit_colours.clear();
        for _ in 0..=255u8 {
            self.debug_hit_colours
                .push((rng.r#gen(), rng.r#gen(), rng.r#gen()));
        }
    }

    /// Three-flag gate: whether this frame needs a full-screen redraw.
    /// Mirrors Photon's `window_dirty || debug_hit_test || show_textbox_mask` pattern.
    /// Buffer-guard mark methods are no-ops — real dirty tracking lives on `self.renderer`.
    fn needs_full_redraw(&self) -> bool {
        self.window_dirty || self.debug_hit_test || self.show_textbox_mask
    }

    pub fn render(&mut self) {
        self.frame_counter += 1;

        if !self.needs_full_redraw() {
            return;
        }

        let width = self.width as usize;
        let height = self.height as usize;
        let speckle = (self.frame_counter % 1024) as usize;
        let fullscreen = self.is_fullscreen;
        let ru = self.ru;
        let hovered = self.hovered_button;
        let debug = self.debug;
        let debug_hit_test = self.debug_hit_test;
        let frame = self.frame_counter;

        for h in self.hit_test_map.iter_mut() {
            *h = compositing::HIT_BODY;
        }

        // Pre-mark the renderer dirty BEFORE locking the buffer.
        // The SoftbufferBuffer guard's mark_* methods are no-ops; only Renderer::mark_*
        // updates the dirty_y_min/max range that present_frame uses to copy rows.
        self.renderer.mark_all();

        let mut buffer = self.renderer.lock_buffer();
        let pixels: &mut [u32] = &mut buffer;

        drawing::draw_background_texture(pixels, width, height, speckle, fullscreen, 0);

        let (start, crossings, btn_x, btn_h) = Self::draw_window_controls(
            pixels,
            &mut self.hit_test_map,
            self.width,
            self.height,
            ru,
        );

        if !fullscreen {
            Self::draw_window_edges_and_mask(
                pixels,
                &mut self.hit_test_map,
                self.width,
                self.height,
                start,
                &crossings,
            );
        }

        // Hairlines between min|max and max|close, walked from centre until each
        // hits the squircle. Must come after edges_and_mask so the colour-change
        // detection terminates at the right pixel.
        Self::draw_button_hairlines(
            pixels,
            &mut self.hit_test_map,
            self.width,
            btn_x,
            btn_h,
        );

        // Hover tint: scan hit_test_map for the hovered button's id and
        // wrapping_add the theme delta to every matching pixel. Photon's pattern.
        let (hover_id, hover_delta) = match hovered {
            HoveredButton::Close => (HIT_CLOSE_BUTTON, theme::CLOSE_HOVER),
            HoveredButton::Maximize => (HIT_MAXIMIZE_BUTTON, theme::MAXIMIZE_HOVER),
            HoveredButton::Minimize => (HIT_MINIMIZE_BUTTON, theme::MINIMIZE_HOVER),
            _ => (compositing::HIT_NONE, 0),
        };
        Self::apply_window_control_hover(pixels, &self.hit_test_map, hover_id, hover_delta);
        let _ = (btn_x, btn_h); // silence unused warning if hover branches drop them later

        let title_y = height as f32 * 0.45;
        let center_x = width as f32 * 0.5;
        let font_size = (height as f32 * 0.08).max(24.0);
        self.text_renderer.draw_text_center_u32(
            pixels,
            width,
            "Plotypus",
            center_x,
            title_y,
            font_size,
            600,
            theme::TEXT_COLOUR,
            theme::FONT_LOGO,
        );

        let hint_y = height as f32 * 0.55;
        let hint_size = (height as f32 * 0.025).max(10.0);
        self.text_renderer.draw_text_center_u32(
            pixels,
            width,
            "y = <expression>  (Spirix-backed graphing calculator)",
            center_x,
            hint_y,
            hint_size,
            400,
            theme::LABEL_COLOUR,
            theme::FONT_UI,
        );

        if debug {
            let dbg = format!(
                "frame={}  ru={:.3}  size={}x{}  hov={:?}  edge={:?}",
                frame, ru, width, height, hovered, self.resize_edge
            );
            self.text_renderer.draw_text_left_u32(
                pixels,
                width,
                &dbg,
                12.0,
                height as f32 - 28.0,
                14.0,
                400,
                theme::COUNTER_TEXT,
                theme::FONT_UI,
            );
        }

        if debug_hit_test && !self.debug_hit_colours.is_empty() {
            for y in 0..height {
                for x in 0..width {
                    let idx = y * width + x;
                    let id = self.hit_test_map[idx] as usize;
                    if id < self.debug_hit_colours.len() {
                        let (r, g, b) = self.debug_hit_colours[id];
                        pixels[idx] = compositing::pack_argb(r, g, b, 255);
                    }
                }
            }
        }

        let _ = buffer.present();
        self.window_dirty = false;
    }

}
