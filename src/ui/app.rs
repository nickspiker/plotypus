use crate::ui::compositing::{HIT_CLOSE_BUTTON, HIT_MAXIMIZE_BUTTON, HIT_MINIMIZE_BUTTON};
use crate::ui::input_box::{
    self, InputLayout, TextState, draw_chrome, recompute_widths, render_blinkey, render_text,
};
use crate::ui::plot::{PlotView, Rect, draw_plot, screen_to_world};
use crate::ui::renderer::Renderer;
use crate::ui::text_rasterizing::TextRenderer;
use crate::ui::{compositing, drawing, theme};
use rand::Rng;
use spirix::ScalarF4E3 as S43;
use std::time::{Duration, Instant};
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
pub enum PlotDragMode {
    Pan,
    Zoom,
}

#[derive(Debug, Clone, Copy)]
pub struct PlotDrag {
    pub mode: PlotDragMode,
    /// Screen-space cursor position when the drag started.
    pub start_x: f32,
    pub start_y: f32,
    /// World coords under the cursor when the drag started (for zoom anchoring).
    pub anchor_world_x: S43,
    pub anchor_world_y: S43,
    /// View bounds at the moment the drag started (for zoom — pan integrates incrementally).
    pub start_view: PlotView,
    /// Last-frame cursor position for incremental pan deltas.
    pub last_x: f32,
    pub last_y: f32,
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

    // Formula input state — the source of truth for what the user has typed.
    pub text_state: TextState,
    /// Snapshot of `text_state` reflecting what is currently composited into `cpu_buffer`. Differential render subtracts using this then adds using `text_state`, then assigns this := text_state.
    pub last_text_state: TextState,
    /// Snapshot of the InputLayout used to draw `last_text_state` — needed so a resize can subtract from the *old* coordinates before the new layout is applied.
    pub last_layout: Option<InputLayout>,
    /// Single-channel mask (window-sized) marking the inside of the input box, used by `render_char_additive_u32` to clip glyphs to the textbox.
    pub textbox_mask: Vec<u8>,
    /// Set when text_state diverges from last_text_state (insert/delete/move).
    pub text_dirty: bool,
    /// Whether the blinkey is currently composited in `cpu_buffer`.
    pub blinkey_visible: bool,
    /// Random per-blink orientation: true = bright at the top of the wave. Set fresh on every ON-event, kept stable across the matching OFF-event so subtraction cancels the prior addition exactly.
    pub blinkey_top_bright: bool,
    /// Last blinkey position composited into `cpu_buffer` (for subtraction).
    pub last_blinkey_x: usize,
    pub last_blinkey_top: usize,
    pub last_blinkey_height: usize,
    /// Wall-clock instant of the next blink toggle. Drives the wakeup loop.
    pub next_blink_time: Instant,

    // Plot
    pub plot_view: PlotView,
    pub plot_drag: Option<PlotDrag>,
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
            mouse_x: f32::NAN,
            mouse_y: f32::NAN,
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
            text_state: {
                let mut t = TextState::new();
                t.focused = true;
                t
            },
            last_text_state: TextState::new(),
            last_layout: None,
            textbox_mask: vec![0u8; (width * height) as usize],
            text_dirty: false,
            blinkey_visible: false,
            blinkey_top_bright: true,
            last_blinkey_x: 0,
            last_blinkey_top: 0,
            last_blinkey_height: 0,
            next_blink_time: Instant::now() + Duration::from_millis(300),
            plot_view: PlotView::default(),
            plot_drag: None,
        }
    }

    /// Begin a pan or zoom drag anchored at the given screen position. Returns false if the position is outside the plot rect (caller should not start).
    pub fn start_plot_drag(&mut self, mode: PlotDragMode, x: f32, y: f32) -> bool {
        let (_, plot_rect) = Self::compute_layout(self.width, self.height, self.button_height());
        if !point_in_rect(plot_rect, x, y) {
            return false;
        }
        let (anchor_world_x, anchor_world_y) = screen_to_world(plot_rect, self.plot_view, x, y);
        self.plot_drag = Some(PlotDrag {
            mode,
            start_x: x,
            start_y: y,
            anchor_world_x,
            anchor_world_y,
            start_view: self.plot_view,
            last_x: x,
            last_y: y,
        });
        true
    }

    /// Apply the cursor's current position to the active drag. Returns true if the view changed and a redraw is needed.
    pub fn update_plot_drag(&mut self, x: f32, y: f32) -> bool {
        let btn_h = self.button_height();
        let (_, plot_rect) = Self::compute_layout(self.width, self.height, btn_h);
        let Some(drag) = self.plot_drag.as_mut() else {
            return false;
        };
        match drag.mode {
            PlotDragMode::Pan => {
                let dx = x - drag.last_x;
                let dy = y - drag.last_y;
                let view = self.plot_view;
                let wpx = (view.x_max - view.x_min) / plot_rect.w;
                let wpy = (view.y_max - view.y_min) / plot_rect.h;
                self.plot_view.x_min -= dx * wpx;
                self.plot_view.x_max -= dx * wpx;
                self.plot_view.y_min += dy * wpy;
                self.plot_view.y_max += dy * wpy;
                drag.last_x = x;
                drag.last_y = y;
            }
            PlotDragMode::Zoom => {
                const SENS: f32 = 0.005;
                let dx = x - drag.start_x;
                let dy = y - drag.start_y;
                let factor_x = (-dx * SENS).exp();
                let factor_y = (dy * SENS).exp();
                let sv = drag.start_view;
                self.plot_view.x_min =
                    drag.anchor_world_x - (drag.anchor_world_x - sv.x_min) * factor_x;
                self.plot_view.x_max =
                    drag.anchor_world_x + (sv.x_max - drag.anchor_world_x) * factor_x;
                self.plot_view.y_min =
                    drag.anchor_world_y - (drag.anchor_world_y - sv.y_min) * factor_y;
                self.plot_view.y_max =
                    drag.anchor_world_y + (sv.y_max - drag.anchor_world_y) * factor_y;
            }
        }
        self.window_dirty = true;
        true
    }

    pub fn end_plot_drag(&mut self) {
        self.plot_drag = None;
    }

    pub fn button_height(&self) -> usize {
        (self.span / 32.0 * self.ru).ceil() as usize
    }

    /// Compute the input-box and plot rectangles from window dims and chrome bar height. Layout: chrome row at the top (where the window controls live), then a margin, then a one-line input row, then the plot fills the rest down to the bottom margin.
    pub fn compute_layout(width: u32, height: u32, button_height: usize) -> (Rect, Rect) {
        let w = width as usize;
        let h = height as usize;
        let margin = button_height;
        let gap = (button_height / 4).max(2);
        let input_h = button_height;

        let input_x = margin;
        let input_y = button_height + gap;
        let input_w = w.saturating_sub(margin * 2);

        let plot_x = margin;
        let plot_y = input_y + input_h + gap;
        let plot_w = w.saturating_sub(margin * 2);
        let plot_h = h.saturating_sub(plot_y + margin);

        (
            Rect {
                x: input_x,
                y: input_y,
                w: input_w,
                h: input_h,
            },
            Rect {
                x: plot_x,
                y: plot_y,
                w: plot_w,
                h: plot_h,
            },
        )
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.width = size.width;
        self.height = size.height;
        self.span = compute_span(size.width, size.height);
        self.renderer.resize(size.width, size.height);
        let n = (size.width * size.height) as usize;
        self.hit_test_map.resize(n, compositing::HIT_BODY);
        self.textbox_mask.resize(n, 0);
        self.window_dirty = true;
        self.last_layout = None;
        self.blinkey_visible = false;
    }

    pub fn set_fullscreen(&mut self, fullscreen: bool) {
        if self.is_fullscreen != fullscreen {
            self.is_fullscreen = fullscreen;
            self.window_dirty = true;
        }
    }

    /// Hit-test a pixel position. Returns `HIT_NONE` for non-finite coords (e.g. the `f32::NAN` sentinel before the first cursor event) and for any position outside the window.
    pub fn hit_test(&self, x: f32, y: f32) -> u8 {
        if !x.is_finite() || !y.is_finite() {
            return compositing::HIT_NONE;
        }
        let xi = x as i32;
        let yi = y as i32;
        if xi < 0 || yi < 0 || (xi as u32) >= self.width || (yi as u32) >= self.height {
            return compositing::HIT_NONE;
        }
        let idx = yi as usize * self.width as usize + xi as usize;
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
    /// Mirrors Photon's `window_dirty || debug_hit_test || show_textbox_mask` pattern. Buffer-guard mark methods are no-ops — real dirty tracking lives on `self.renderer`.
    fn needs_full_redraw(&self) -> bool {
        self.window_dirty || self.debug_hit_test || self.show_textbox_mask
    }

    pub fn render(&mut self) {
        self.frame_counter += 1;

        if !self.needs_full_redraw() {
            if self.text_dirty {
                self.render_input_diff();
            }
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

        // Pre-mark the renderer dirty BEFORE locking the buffer. The SoftbufferBuffer guard's mark_* methods are no-ops; only Renderer::mark_* updates the dirty_y_min/max range that present_frame uses to copy rows.
        self.renderer.mark_all();

        let mut buffer = self.renderer.lock_buffer();
        let pixels: &mut [u32] = &mut buffer;

        drawing::draw_background_texture(pixels, width, height, speckle, fullscreen, 0);

        let (start, crossings, btn_x, btn_h) =
            Self::draw_window_controls(pixels, &mut self.hit_test_map, self.width, self.height, ru);

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

        // Hairlines between min|max and max|close, walked from centre until each hits the squircle. Must come after edges_and_mask so the colour-change detection terminates at the right pixel.
        Self::draw_button_hairlines(pixels, &mut self.hit_test_map, self.width, btn_x, btn_h);

        // Hover tint: scan hit_test_map for the hovered button's id and wrapping_add the theme delta to every matching pixel. Photon's pattern.
        let (hover_id, hover_delta) = match hovered {
            HoveredButton::Close => (HIT_CLOSE_BUTTON, theme::CLOSE_HOVER),
            HoveredButton::Maximize => (HIT_MAXIMIZE_BUTTON, theme::MAXIMIZE_HOVER),
            HoveredButton::Minimize => (HIT_MINIMIZE_BUTTON, theme::MINIMIZE_HOVER),
            _ => (compositing::HIT_NONE, 0),
        };
        Self::apply_window_control_hover(pixels, &self.hit_test_map, hover_id, hover_delta);
        let _ = btn_x;

        let (input_rect, plot_rect) = Self::compute_layout(self.width, self.height, btn_h);
        if plot_rect.w > 4 && plot_rect.h > 4 {
            let label_font_size = (btn_h as f32 * 0.5).max(10.0);
            draw_plot(
                pixels,
                &mut self.hit_test_map,
                &mut self.text_renderer,
                width,
                plot_rect,
                self.plot_view,
                label_font_size,
            );
        }

        // Input box: draw chrome (bg + frame + prompt), then additively render text and blinkey. Snapshot text_state + layout + blinkey position so the diff path can subtract them on the next text-only update.
        if input_rect.w > 4 && input_rect.h > 4 {
            let font_size = (input_rect.h as f32 * 0.55).max(12.0);
            let prompt_w = input_box::measure_prompt_width(&mut self.text_renderer, font_size);
            let layout = InputLayout::new(input_rect, prompt_w, font_size);

            // Re-measure widths in case the font size changed (resize, ru change).
            recompute_widths(&mut self.text_state, &mut self.text_renderer, font_size);

            draw_chrome(
                pixels,
                &mut self.hit_test_map,
                &mut self.textbox_mask,
                width,
                input_rect,
                self.text_state.focused,
                &layout,
                &mut self.text_renderer,
            );

            render_text(
                pixels,
                &mut self.text_renderer,
                width,
                &self.text_state,
                &layout,
                &self.textbox_mask,
                true,
            );

            let blinkey_visible = self.text_state.focused;
            if blinkey_visible {
                self.blinkey_top_bright = rand::thread_rng().r#gen();
                let bx = layout.cursor_x(&self.text_state);
                render_blinkey(
                    pixels,
                    width,
                    bx,
                    layout.blinkey_top,
                    layout.blinkey_height,
                    self.blinkey_top_bright,
                    true,
                );
                self.last_blinkey_x = bx;
                self.last_blinkey_top = layout.blinkey_top;
                self.last_blinkey_height = layout.blinkey_height;
                self.next_blink_time = next_blink_wake();
            }
            self.blinkey_visible = blinkey_visible;

            self.last_text_state = self.text_state.clone();
            self.last_layout = Some(layout);
            self.text_dirty = false;
        }

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

    /// Differential input box render: subtract last frame's text+blinkey from `cpu_buffer`, then add the current ones. Marks only the input rect's rows dirty so the chrome and plot stay untouched in the compositor.
    fn render_input_diff(&mut self) {
        let Some(layout) = self.last_layout else {
            // No previous full draw to diff against — escalate.
            self.window_dirty = true;
            return;
        };
        let width = self.width as usize;
        let rect = layout.rect;

        self.renderer
            .mark_rows(rect.y as u32, (rect.y + rect.h) as u32);

        let mut buffer = self.renderer.lock_buffer();
        let pixels: &mut [u32] = &mut buffer;

        if self.blinkey_visible {
            render_blinkey(
                pixels,
                width,
                self.last_blinkey_x,
                self.last_blinkey_top,
                self.last_blinkey_height,
                self.blinkey_top_bright,
                false,
            );
            self.blinkey_visible = false;
        }

        render_text(
            pixels,
            &mut self.text_renderer,
            width,
            &self.last_text_state,
            &layout,
            &self.textbox_mask,
            false,
        );

        render_text(
            pixels,
            &mut self.text_renderer,
            width,
            &self.text_state,
            &layout,
            &self.textbox_mask,
            true,
        );

        if self.text_state.focused {
            self.blinkey_top_bright = rand::thread_rng().r#gen();
            let bx = layout.cursor_x(&self.text_state);
            render_blinkey(
                pixels,
                width,
                bx,
                layout.blinkey_top,
                layout.blinkey_height,
                self.blinkey_top_bright,
                true,
            );
            self.last_blinkey_x = bx;
            self.last_blinkey_top = layout.blinkey_top;
            self.last_blinkey_height = layout.blinkey_height;
            self.blinkey_visible = true;
            self.next_blink_time = next_blink_wake();
        }

        let _ = buffer.present();

        self.last_text_state = self.text_state.clone();
        self.text_dirty = false;
    }

    /// Toggle the blinkey on or off in `cpu_buffer`, schedule the next toggle. Called from the event loop when `next_blink_time` is reached.
    pub fn flip_blinkey(&mut self) {
        if !self.text_state.focused {
            return;
        }
        let Some(layout) = self.last_layout else {
            return;
        };
        let width = self.width as usize;
        let rect = layout.rect;

        self.renderer
            .mark_rows(rect.y as u32, (rect.y + rect.h) as u32);
        let mut buffer = self.renderer.lock_buffer();
        let pixels: &mut [u32] = &mut buffer;

        if self.blinkey_visible {
            // Subtract using the orientation chosen when this blinkey was added.
            render_blinkey(
                pixels,
                width,
                self.last_blinkey_x,
                self.last_blinkey_top,
                self.last_blinkey_height,
                self.blinkey_top_bright,
                false,
            );
            self.blinkey_visible = false;
        } else {
            // Pick a fresh random orientation for this ON-event.
            self.blinkey_top_bright = rand::thread_rng().r#gen();
            let bx = layout.cursor_x(&self.text_state);
            render_blinkey(
                pixels,
                width,
                bx,
                layout.blinkey_top,
                layout.blinkey_height,
                self.blinkey_top_bright,
                true,
            );
            self.last_blinkey_x = bx;
            self.last_blinkey_top = layout.blinkey_top;
            self.last_blinkey_height = layout.blinkey_height;
            self.blinkey_visible = true;
        }

        let _ = buffer.present();
        self.next_blink_time = next_blink_wake();
    }
}

/// Photon's blinkey blink rate: random 0–300 ms per toggle so neighbouring cursors in the same window don't sync up into a metronome.
fn next_blink_wake() -> Instant {
    let ms = rand::thread_rng().gen_range(0..=300);
    Instant::now() + Duration::from_millis(ms)
}


fn point_in_rect(r: Rect, x: f32, y: f32) -> bool {
    let xi = x as i32;
    let yi = y as i32;
    xi >= r.x as i32 && xi < (r.x + r.w) as i32 && yi >= r.y as i32 && yi < (r.y + r.h) as i32
}
