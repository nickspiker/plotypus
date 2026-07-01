//! `PlotypusApp` — the `fluor::host::app::FluorApp` that drives Plotypus.
//!
//! Fluor owns the window, event loop, CPU present buffer, chrome (titlebar / window
//! controls / drag / resize edges), text rendering, hit-testing, and the blink timer.
//! Plotypus owns the domain: a formula `Textbox`, a "play" `Button`, the plot region,
//! pan/zoom on the plot, and the Photon-notification synth/audio.
//!
//! Layout (top → bottom): chrome bar, then a row holding the formula textbox with the
//! play button to its right, then the plot fills the rest. The plot draws straight into
//! the host's `target: &mut [u32]` in `render`.

use crate::formula::{self, ParseError, Token};
use crate::synth::{self, PhotonVoice};
use crate::ui::plot::{self, PlotView, Rect};

use fluor::canvas::{Canvas, PixelRect};
use fluor::coord::Coord;
use fluor::event::{
    CursorIcon, ElementState, Event, Key, KeyEvent, ModifiersState, MouseButton, NamedKey,
};
use fluor::geom::Viewport;
use fluor::host::app::{Context, EventResponse, FluorApp};
use fluor::host::chrome::{self, HIT_NONE, HitId, ResizeEdge};
use fluor::host::chrome_widget::DefaultChrome;
use fluor::host::widget::{self as widget, Container, TabDir, Widget};
use fluor::widgets::{BlinkTimer, Button, Textbox};
use spirix::ScalarF4E3 as S43;
use std::time::Instant;

const DEFAULT_FORMULA: &str = "#sin(x*@pi*2)*0.7";
/// Default play duration in seconds (decimal, base 10 — this box is plain seconds, not a
/// dozenal formula value). Seeds the duration box.
const DEFAULT_DURATION: &str = "1.05";

/// Pan or zoom drag in progress on the plot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlotDragMode {
    Pan,
    Zoom,
    /// Right-drag: adjust the x/y aspect ratio — horizontal motion scales the x-extent,
    /// vertical motion scales the y-extent, each anchored at the cursor.
    Aspect,
}

#[derive(Debug, Clone, Copy)]
struct PlotDrag {
    mode: PlotDragMode,
    /// Cursor position when the drag started (zoom anchors here).
    start_x: Coord,
    start_y: Coord,
    /// World coords under the cursor at drag start (zoom anchoring).
    anchor_world_x: S43,
    anchor_world_y: S43,
    /// View bounds when the drag started.
    start_view: PlotView,
    /// Last-frame cursor position for incremental pan deltas.
    last_x: Coord,
    last_y: Coord,
}

pub struct PlotypusApp {
    title: String,
    chrome: DefaultChrome,
    /// Formula input. Its `chars` are the source of truth for the plotted expression.
    formula_box: Textbox,
    /// Numeric base for parsing literals AND rendering axis labels (2..=36). Type a single
    /// digit/letter into `base_box` to change it; `c` = dozenal (12), the default.
    base: u8,
    /// Base selector — a one-char box showing `formula::base_to_char(self.base)`.
    base_box: Textbox,
    /// Duration input (seconds) — how long the play sweep takes to cross the viewed
    /// x-range. Parsed as a plain decimal; falls back to `DEFAULT_DURATION` if invalid.
    duration_box: Textbox,
    /// "Play" button — evaluates + plays the typed formula as audio.
    play_button: Button,
    /// Monotonic dense hit-id counter shared by chrome + widgets.
    hit_counter: HitId,
    /// Currently focused widget id (keyboard target), or None.
    current_focus: Option<HitId>,
    blink: BlinkTimer,

    // --- Plot domain state ---
    plot_view: PlotView,
    plot_rect: Rect,
    plot_drag: Option<PlotDrag>,
    /// Last parse of the formula text. `None` = empty box (resting state); `Some(Err)` =
    /// parse error (plot blanks); `Some(Ok)` = the plotted token vector.
    formula: Option<Result<Vec<Token>, ParseError>>,

    /// When `Some`, the plot shows this notification waveform (same S43 `voice(t)` the
    /// speaker plays) instead of the typed formula. Cleared when the formula is edited.
    notification: Option<PhotonVoice>,
    /// Rotating seed so each play demos a different per-user sound.
    notification_seed: u64,

    modifiers: ModifiersState,
    is_maximized: bool,
    /// True while a left-drag is extending a textbox selection (armed on press inside a
    /// focused textbox, released on mouse-up).
    is_dragging_select: bool,

    /// `[`+`]` debug-chord tracker (see `ChordTracker`).
    chord: ChordTracker,
    /// `true` while the hit-map debug overlay (`[]h`) is on. Painted at the end of
    /// `render` from `debug_hit_colours`.
    show_hitmask: bool,
    /// 256-entry random palette indexed by hit-id byte, regenerated each time the hitmask
    /// overlay toggles on so distinct ids get visibly-distinct colours.
    debug_hit_colours: Vec<u32>,
}

/// Grace after a bracket Release before we treat it as truly released. X11 fires a
/// synthetic Release for a held key the instant another key is pressed; without this
/// grace the chord would disarm a millisecond before the action key registers.
const CHORD_RELEASE_GRACE: std::time::Duration = std::time::Duration::from_millis(40);

/// Tracks whether `[` and `]` are simultaneously held for the debug chord. A bracket is
/// "held" if its last press is more recent than its last release, OR the release was
/// within `CHORD_RELEASE_GRACE` (absorbs X11's synthetic-release-on-other-keypress).
#[derive(Default)]
struct ChordTracker {
    lb_press: Option<Instant>,
    lb_release: Option<Instant>,
    rb_press: Option<Instant>,
    rb_release: Option<Instant>,
}

impl ChordTracker {
    fn note(&mut self, bracket: char, state: ElementState, now: Instant) {
        match (bracket, state) {
            ('[', ElementState::Pressed) => self.lb_press = Some(now),
            ('[', ElementState::Released) => self.lb_release = Some(now),
            (']', ElementState::Pressed) => self.rb_press = Some(now),
            (']', ElementState::Released) => self.rb_release = Some(now),
            _ => {}
        }
    }

    fn both_held(&self, now: Instant) -> bool {
        fn held(press: Option<Instant>, release: Option<Instant>, now: Instant) -> bool {
            match (press, release) {
                (Some(p), Some(r)) => p > r || now.duration_since(r) < CHORD_RELEASE_GRACE,
                (Some(_), None) => true,
                _ => false,
            }
        }
        held(self.lb_press, self.lb_release, now) && held(self.rb_press, self.rb_release, now)
    }
}

impl PlotypusApp {
    pub fn new() -> Self {
        // Placeholder viewport — real geometry lands in `init` once the host opens the
        // window. Chrome claims hit ids 1..=N first; the widgets that follow get the rest.
        let viewport = Viewport::new(1280, 800);
        let mut hit_counter: HitId = HIT_NONE;
        let chrome = DefaultChrome::new(
            viewport,
            "Plotypus".to_string(),
            load_orb(),
            Some("ready".to_string()),
            &mut hit_counter,
        );

        let mut formula_box = Textbox::new(&mut hit_counter, 0.0, 0.0, 1.0, 1.0, 12.0);
        formula_box.stroke_ru = 1.0 / 12.0;
        // Seed the default formula. Widths get measured on the first `set_font_size`.
        for c in DEFAULT_FORMULA.chars() {
            formula_box.chars.push(c);
        }
        formula_box.cursor = formula_box.chars.len();

        // Base selector: one char (`c` = dozenal). Typing a digit/letter sets the base.
        let base = formula::DEFAULT_BASE;
        let mut base_box = Textbox::new(&mut hit_counter, 0.0, 0.0, 1.0, 1.0, 12.0);
        base_box.stroke_ru = 1.0 / 12.0;
        base_box.chars.push(formula::base_to_char(base));
        base_box.cursor = base_box.chars.len();

        // Duration box: how many seconds the play sweep takes to cross the viewed x-range.
        let mut duration_box = Textbox::new(&mut hit_counter, 0.0, 0.0, 1.0, 1.0, 12.0);
        duration_box.stroke_ru = 1.0 / 12.0;
        for c in DEFAULT_DURATION.chars() {
            duration_box.chars.push(c);
        }
        duration_box.cursor = duration_box.chars.len();

        let mut play_button =
            Button::new(&mut hit_counter, 0.0, 0.0, 1.0, 1.0, 12.0, "▶ play");
        play_button.stroke_ru = 1.0 / 12.0;

        Self {
            title: "Plotypus".to_string(),
            chrome,
            formula_box,
            base,
            base_box,
            duration_box,
            play_button,
            hit_counter,
            current_focus: None,
            blink: BlinkTimer::new(),
            plot_view: PlotView::default(),
            plot_rect: Rect { x: 0, y: 0, w: 0, h: 0 },
            plot_drag: None,
            formula: None,
            notification: None,
            notification_seed: 0,
            modifiers: ModifiersState::empty(),
            is_maximized: false,
            is_dragging_select: false,
            chord: ChordTracker::default(),
            show_hitmask: false,
            debug_hit_colours: Vec::new(),
        }
    }

    /// Recompute widget + plot geometry from the viewport. Chrome reserves the top bar;
    /// the formula box + play button sit on the next row; the plot fills the rest.
    fn update_layout(&mut self, ctx: &mut Context) {
        let vp = ctx.viewport;
        let span = vp.effective_span();
        let bw = span / 32.0; // chrome button-width unit; drives margins + row heights
        let w = vp.width_px as Coord;

        let margin = bw;
        let row_h = bw * 2.0;
        let gap = bw * 0.5;
        // Chrome occupies roughly the top button row; start content below it.
        let chrome_bar = bw * 2.0;
        let row_cy = chrome_bar + gap + row_h * 0.5;

        // Row layout, left→right: formula (flexible) · base (one char) · duration (narrow)
        // · play button (fixed). The formula box absorbs whatever width the rest leave.
        let button_w = (bw * 8.0).min(w * 0.25);
        let dur_w = (bw * 5.0).min(w * 0.15);
        let base_w = bw * 2.5;
        let content_w = w - margin * 2.0;
        let tb_w = (content_w - base_w - dur_w - button_w - gap * 3.0).max(bw * 4.0);

        let tb_cx = margin + tb_w * 0.5;
        let base_cx = margin + tb_w + gap + base_w * 0.5;
        let dur_cx = margin + tb_w + gap + base_w + gap + dur_w * 0.5;
        let btn_cx = margin + tb_w + gap + base_w + gap + dur_w + gap + button_w * 0.5;
        let font_size = bw;

        self.formula_box.set_rect(tb_cx, row_cy, tb_w, row_h);
        self.formula_box.set_font_size(font_size, ctx.text);
        self.base_box.set_rect(base_cx, row_cy, base_w, row_h);
        self.base_box.set_font_size(font_size, ctx.text);
        self.duration_box.set_rect(dur_cx, row_cy, dur_w, row_h);
        self.duration_box.set_font_size(font_size, ctx.text);
        self.play_button.set_rect(btn_cx, row_cy, button_w, row_h);
        self.play_button.set_font_size(font_size);

        // Plot fills from below the content row to the bottom margin.
        let plot_top = (row_cy + row_h * 0.5 + gap) as usize;
        let plot_x = margin as usize;
        let plot_w = (content_w) as usize;
        let plot_bottom = (vp.height_px as Coord - margin) as usize;
        let plot_h = plot_bottom.saturating_sub(plot_top);
        self.plot_rect = Rect { x: plot_x, y: plot_top, w: plot_w, h: plot_h };
    }

    /// Re-parse the formula box into `self.formula`. Empty box → resting state (None),
    /// so the grid + labels stay visible. Editing also exits notification view.
    fn reparse_formula(&mut self) {
        let text: String = self.formula_box.chars.iter().collect();
        self.formula = if text.trim().is_empty() {
            None
        } else {
            Some(formula::parse(&text, self.base))
        };
        self.notification = None;
    }

    /// True if `(x, y)` is inside the plot rect.
    fn point_in_plot(&self, x: Coord, y: Coord) -> bool {
        let r = self.plot_rect;
        let xi = x as i32;
        let yi = y as i32;
        xi >= r.x as i32
            && xi < (r.x + r.w) as i32
            && yi >= r.y as i32
            && yi < (r.y + r.h) as i32
    }

    /// World coords under a plot-pixel position.
    fn plot_screen_to_world(&self, sx: Coord, sy: Coord) -> (S43, S43) {
        plot::screen_to_world(self.plot_rect, self.plot_view, sx, sy)
    }

    /// Drop color-emoji faces from the shared font DB so Spirix's Display glyphs render as
    /// crisp monochrome outlines. cosmic-text's per-glyph fallback otherwise routes the
    /// heavy transfinite/negligible arrows ⬆ (U+2B06) / ⬇ (U+2B07) — which appear in
    /// undefined-state tags like `℘⬇/⬇` — to Noto Color Emoji, whose color-bitmap glyphs come
    /// out shredded (and double-width) through fluor's grayscale swash rasterizer. With the
    /// color-emoji faces gone, those codepoints fall to a normal monochrome font (Adwaita
    /// Mono / Symbola / Noto Emoji, whatever the system has) and render correctly. Plotypus
    /// uses no emoji anywhere, so removing them is free. Note: ⬆/⬇ are NOT ↑/↓ — the heavy
    /// arrows mean *transfinite* / *negligible* (a class group), so we must render the real
    /// glyphs rather than substitute the escape arrows.
    fn drop_color_emoji_fonts(ctx: &mut Context) {
        let db = ctx.text.font_system_mut().db_mut();
        let ids: Vec<_> = db
            .faces()
            .filter(|f| {
                f.families
                    .iter()
                    .any(|(name, _)| name.to_lowercase().contains("color emoji"))
            })
            .map(|f| f.id)
            .collect();
        for id in ids {
            db.remove_face(id);
        }
    }

    /// Show the world x under the cursor AND the function value f(x) there, in the chrome
    /// status line, formatted in the current base (Spirix Display takes the format precision
    /// as the output radix, like the axis labels). f(x) is the actual evaluated result — so
    /// it carries Spirix's class tags (⦉∞⦊, ⦉±↑⦊, ⦉±↓⦊, ℘…) rather than the meaningless
    /// screen-height the old readout printed for the cursor's y. Requests a redraw so the
    /// readout updates immediately.
    fn show_click_coords(&mut self, sx: Coord, sy: Coord, ctx: &mut Context) {
        let (wx, _wy) = self.plot_screen_to_world(sx, sy);
        let b = self.base as usize;
        let fx = match self.formula.as_ref() {
            // Evaluate the plotted expression at the cursor's x. This is the same call the
            // curve renderer makes per pixel-column, so the readout matches what's drawn.
            Some(Ok(tokens)) => match formula::evaluate(tokens, wx, self.base) {
                Ok(v) => format!("{:6.b$}", v, b = b),
                // evaluate() only errors on a malformed token stream (already caught at parse
                // time); undefined *math* comes back as an Ok(℘…) value, so this is rare.
                Err(_) => "℘".to_string(),
            },
            // Empty box or parse error → no curve to sample.
            _ => "—".to_string(),
        };
        let text = format!("x {:6.b$}   f(x) {}", wx, fx, b = b);
        if self.chrome.set_status_text(Some(text)) {
            ctx.window.request_redraw();
        }
    }

    fn start_plot_drag(&mut self, mode: PlotDragMode, x: Coord, y: Coord) {
        let (anchor_world_x, anchor_world_y) = self.plot_screen_to_world(x, y);
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
    }

    /// Apply the cursor's current position to the active drag. Returns true if the view changed.
    fn update_plot_drag(&mut self, x: Coord, y: Coord) -> bool {
        let r = self.plot_rect;
        let Some(drag) = self.plot_drag.as_mut() else {
            return false;
        };
        match drag.mode {
            PlotDragMode::Pan => {
                let dx = x - drag.last_x;
                let dy = y - drag.last_y;
                let view = self.plot_view;
                let wpx = (view.x_max - view.x_min) / r.w.max(1);
                let wpy = (view.y_max - view.y_min) / r.h.max(1);
                self.plot_view.x_min -= dx * wpx;
                self.plot_view.x_max -= dx * wpx;
                self.plot_view.y_min += dy * wpy;
                self.plot_view.y_max += dy * wpy;
                drag.last_x = x;
                drag.last_y = y;
            }
            // Zoom (Ctrl+left-drag) and Aspect (right-drag) share the same cursor-anchored
            // independent-axis scale: horizontal motion scales the x-extent, vertical the
            // y-extent. They differ only in trigger — Aspect is the natural "stretch x vs
            // y" gesture, Zoom keeps the modifier path for users on a left-button-only mouse.
            PlotDragMode::Zoom | PlotDragMode::Aspect => {
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
        true
    }

    /// Cursor-anchored uniform zoom by `factor` (scales both x and y extents by the same
    /// amount, keeping the world point under `(sx, sy)` fixed on screen). Used by the
    /// scroll wheel: `factor = 31/32` per notch in, `33/32` per notch out — deliberately
    /// asymmetric so you can creep onto an exact value rather than overshoot symmetrically.
    fn zoom_at(&mut self, sx: Coord, sy: Coord, factor: f32) {
        let (ax, ay) = self.plot_screen_to_world(sx, sy);
        let v = self.plot_view;
        self.plot_view.x_min = ax - (ax - v.x_min) * factor;
        self.plot_view.x_max = ax + (v.x_max - ax) * factor;
        self.plot_view.y_min = ay - (ay - v.y_min) * factor;
        self.plot_view.y_max = ay + (v.y_max - ay) * factor;
    }

    /// Synthesize a Photon notification, frame the plot to its waveform, and play it.
    /// The plot updates even if audio fails so the waveform is still visible.
    fn play_notification(&mut self, seed: u64) {
        let voice = PhotonVoice::from_seed(seed);
        self.plot_view = PlotView {
            x_min: S43::ZERO,
            x_max: S43::from(synth::DURATION_SECS),
            y_min: S43::from(-1.05f32),
            y_max: S43::from(1.05f32),
        };
        self.notification = Some(voice);
        let samples = voice.render();
        crate::audio::play(samples);
    }

    /// Commit the typed formula: re-parse it (updating the plotted curve) and, if it
    /// parses, play it as audio — `x` sweeps the visible plot x-range over the duration
    /// set in the duration box, `y = f(x)` is the waveform. This is the Enter /
    /// play-button action: "evaluate and hear what I typed." Leaves notification mode.
    fn play_formula(&mut self) {
        self.reparse_formula();
        let Some(Ok(tokens)) = &self.formula else {
            return; // empty box or parse error — nothing to play
        };
        let tokens = tokens.clone();
        let base = self.base;
        let x_min = self.plot_view.x_min.to_f32();
        let x_max = self.plot_view.x_max.to_f32();
        let y_min = self.plot_view.y_min.to_f32();
        let y_max = self.plot_view.y_max.to_f32();
        let duration = self.play_duration();
        // Map y through the visible y-range to audio full-scale, 1:1 — the vertical axis
        // IS the volume, so a curve that runs off the top/bottom clips and distorts.
        let samples = synth::render_formula(
            |x: S43| formula::evaluate(&tokens, x, base).unwrap_or(S43::ZERO),
            x_min,
            x_max,
            y_min,
            y_max,
            duration,
        );
        crate::audio::play(samples);
    }

    /// Parse the duration box as plain decimal seconds. Falls back to the default on an
    /// unparseable or non-positive value, and clamps to a sane range so a fat-fingered
    /// "1000" doesn't queue a 16-minute buffer.
    fn play_duration(&self) -> f32 {
        let text: String = self.duration_box.chars.iter().collect();
        text.trim()
            .parse::<f32>()
            .ok()
            .filter(|d| d.is_finite() && *d > 0.0)
            .unwrap_or(synth::DURATION_SECS)
            .clamp(0.05, 30.0)
    }

    /// Borrow whichever textbox currently holds focus, or `None` if focus is on a
    /// non-textbox widget (or nothing).
    fn focused_textbox_mut(&mut self) -> Option<&mut Textbox> {
        let focus = self.current_focus?;
        if focus == self.formula_box.hit_id() {
            Some(&mut self.formula_box)
        } else if focus == self.base_box.hit_id() {
            Some(&mut self.base_box)
        } else if focus == self.duration_box.hit_id() {
            Some(&mut self.duration_box)
        } else {
            None
        }
    }

    fn change_focus(&mut self, new_focus: Option<HitId>, ctx: &mut Context) {
        if new_focus == self.current_focus {
            return;
        }
        let prior = self.current_focus;
        widget::apply_focus_change(self as &mut dyn Container, prior, new_focus);
        self.current_focus = new_focus;
        if new_focus.is_some() {
            self.blink.start(Instant::now());
        } else {
            self.blink.stop();
        }
        ctx.window.request_redraw();
    }
}

impl Container for PlotypusApp {
    fn visit(&mut self, f: &mut dyn FnMut(&mut dyn Widget)) {
        f(&mut self.formula_box);
        f(&mut self.base_box);
        f(&mut self.duration_box);
        f(&mut self.play_button);
        self.chrome.visit(f);
    }
}

impl FluorApp for PlotypusApp {
    type UserEvent = ();

    fn title(&self) -> &str {
        &self.title
    }

    fn window_icon(&self) -> Option<&fluor::host::icon::Icon> {
        // Same orb the chrome paints in its top-left slot, so the OS taskbar / alt-tab icon
        // matches the in-window orb (on the platforms winit honours — Windows + X11).
        self.chrome.app_icon.as_ref()
    }

    fn init(&mut self, ctx: &mut Context) {
        Self::drop_color_emoji_fonts(ctx);
        self.chrome.resize(ctx.viewport);
        self.update_layout(ctx);
        self.reparse_formula();
    }

    fn on_resize(&mut self, _w: u32, _h: u32, ctx: &mut Context) {
        self.chrome.resize(ctx.viewport);
        self.is_maximized = ctx.is_maximized;
        self.chrome.set_full_edge(ctx.is_maximized);
        self.update_layout(ctx);
    }

    fn on_event(&mut self, event: &Event, ctx: &mut Context) -> EventResponse {
        match event {
            Event::ModifiersChanged(m) => {
                self.modifiers = *m;
                EventResponse::Pass
            }

            Event::CursorMoved { .. } => {
                // Use ctx.cursor_x/y (window-local) NOT the event's x/y. In Fluor's
                // fullscreen-compositor model the CursorMoved event carries raw SCREEN
                // coords, while ctx and every other geometry value is window-local — mixing
                // them makes the first pan delta jump by the window's screen offset.
                let x = ctx.cursor_x;
                let y = ctx.cursor_y;
                if self.plot_drag.is_some() {
                    if self.update_plot_drag(x, y) {
                        ctx.window.request_redraw();
                    }
                    return EventResponse::Handled;
                }
                // Drag-select: extend the focused textbox's selection to the cursor. The
                // anchor is set on the first move (so a click-drag selects from the press
                // caret), and `cursor` tracks the pixel under the mouse each frame.
                if self.is_dragging_select {
                    if let Some(tb) = self.focused_textbox_mut() {
                        let cx = x.clamp(tb.text_left(), tb.text_right());
                        if tb.selection_anchor.is_none() {
                            tb.selection_anchor = Some(tb.cursor);
                        }
                        tb.cursor = tb.cursor_index_from_x(cx);
                    }
                    ctx.window.request_redraw();
                    return EventResponse::Handled;
                }
                // Hover bookkeeping for chrome + widgets.
                let new_hit = self.chrome.hit_at(x, y);
                let mut changed = self.chrome.set_hover(new_hit);
                let want_tb = new_hit == self.formula_box.hit_id();
                if self.formula_box.is_hovered() != want_tb {
                    self.formula_box.set_hovered(want_tb);
                    changed = true;
                }
                let want_base = new_hit == self.base_box.hit_id();
                if self.base_box.is_hovered() != want_base {
                    self.base_box.set_hovered(want_base);
                    changed = true;
                }
                let want_dur = new_hit == self.duration_box.hit_id();
                if self.duration_box.is_hovered() != want_dur {
                    self.duration_box.set_hovered(want_dur);
                    changed = true;
                }
                let want_btn = new_hit == self.play_button.hit_id();
                if self.play_button.is_hovered() != want_btn {
                    self.play_button.set_hovered(want_btn);
                    changed = true;
                }
                if changed {
                    ctx.window.request_redraw();
                }
                EventResponse::Pass
            }

            Event::CursorLeft => {
                if self.chrome.set_hover(HIT_NONE) {
                    ctx.window.request_redraw();
                }
                EventResponse::Pass
            }

            Event::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
            } => {
                let x = ctx.cursor_x;
                let y = ctx.cursor_y;
                let hit_id = self.chrome.hit_at(x, y);

                // Widget hit takes precedence over resize edges + plot.
                if hit_id != HIT_NONE {
                    let mods = self.modifiers;
                    let response = widget::dispatch_click(self as &mut dyn Container, hit_id, x, y, mods);
                    // Focus the widget if it's focusable.
                    let mut focusable = false;
                    self.visit(&mut |w| {
                        if w.id() == hit_id && w.focus().is_some() {
                            focusable = true;
                        }
                    });
                    self.change_focus(if focusable { Some(hit_id) } else { None }, ctx);
                    // `dispatch_click` above already set the caret (Textbox::on_click);
                    // arm drag-select so a press-and-drag extends a selection.
                    if self.focused_textbox_mut().is_some() {
                        self.is_dragging_select = true;
                    }
                    ctx.window.request_redraw();
                    return response;
                }

                // Plot: plain drag = pan, zoom-modifier drag = zoom.
                if self.point_in_plot(x, y) {
                    self.change_focus(None, ctx);
                    self.show_click_coords(x, y, ctx);
                    let mode = if self.modifiers.control_key() || self.modifiers.super_key() {
                        PlotDragMode::Zoom
                    } else {
                        PlotDragMode::Pan
                    };
                    self.start_plot_drag(mode, x, y);
                    return EventResponse::Handled;
                }

                // Resize edge, else window drag.
                let edge = chrome::get_resize_edge(
                    ctx.viewport.width_px,
                    ctx.viewport.height_px,
                    x,
                    y,
                );
                if edge != ResizeEdge::None {
                    return EventResponse::StartResize(edge);
                }
                self.change_focus(None, ctx);
                EventResponse::StartWindowDrag
            }

            // Right-button press in the plot starts an aspect (x/y) drag.
            Event::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
            } => {
                let x = ctx.cursor_x;
                let y = ctx.cursor_y;
                if self.point_in_plot(x, y) {
                    self.show_click_coords(x, y, ctx);
                    self.start_plot_drag(PlotDragMode::Aspect, x, y);
                    return EventResponse::Handled;
                }
                EventResponse::Pass
            }

            // Any button release ends an active plot drag or selection drag.
            Event::MouseInput {
                state: ElementState::Released,
                ..
            } => {
                let mut handled = self.plot_drag.take().is_some();
                if self.is_dragging_select {
                    self.is_dragging_select = false;
                    // A zero-width selection (click without drag) collapses to a caret.
                    if let Some(tb) = self.focused_textbox_mut() {
                        if tb.selection_anchor == Some(tb.cursor) {
                            tb.selection_anchor = None;
                        }
                    }
                    handled = true;
                }
                if handled {
                    return EventResponse::Handled;
                }
                EventResponse::Pass
            }

            // Scroll wheel: cursor-anchored zoom. Up = in (31/32), down = out (33/32) —
            // asymmetric so repeated notches can weasel onto an exact value.
            Event::MouseWheel { delta } => {
                let x = ctx.cursor_x;
                let y = ctx.cursor_y;
                if !self.point_in_plot(x, y) {
                    return EventResponse::Pass;
                }
                let steps: f32 = match delta {
                    fluor::event::MouseScrollDelta::Lines(_, y) => *y,
                    fluor::event::MouseScrollDelta::Pixels(_, y) => y / 32.0,
                };
                if steps == 0.0 {
                    return EventResponse::Pass;
                }
                // Per-notch factor, raised to |steps| so trackpad fractional deltas scale
                // smoothly. steps > 0 (scroll up) zooms in (31/32 < 1 shrinks the view).
                let per_notch: f32 = if steps > 0.0 { 31.0 / 32.0 } else { 33.0 / 32.0 };
                let factor = per_notch.powf(steps.abs());
                self.zoom_at(x, y, factor);
                ctx.window.request_redraw();
                EventResponse::Handled
            }

            Event::KeyboardInput { event: kev } => self.handle_key(kev, ctx),

            Event::Focused(focused) => {
                if self.chrome.set_focused(*focused) {
                    ctx.window.request_redraw();
                }
                EventResponse::Pass
            }

            _ => EventResponse::Pass,
        }
    }

    fn damage_rect(&self, viewport: Viewport) -> Option<PixelRect> {
        // Conservative first cut: full viewport every frame. Matches the pre-migration
        // full-redraw behaviour; differential damage is a later optimization.
        let w = viewport.width_px as usize;
        let h = viewport.height_px as usize;
        Some(PixelRect::new(0, 0, w, h))
    }

    fn hit_test_map(&self) -> Option<(&[HitId], usize, usize)> {
        let (w, h) = self.chrome.dims();
        Some((self.chrome.hit_test_map(), w, h))
    }

    fn overlay_deltas(&mut self) -> Vec<u32> {
        let count = self.hit_counter as usize + 1;
        widget::build_overlay_deltas(self, count)
    }

    fn render(&mut self, target: &mut [u32], ctx: &mut Context) {
        let buf_w = ctx.viewport.width_px as usize;
        let buf_h = ctx.viewport.height_px as usize;

        // Chrome background + perimeter + controls. The bg closure must fully cover the
        // window-shaped background; we paint a night-sky starfield (see `draw_starfield`)
        // — on-theme for the Photon notification (each star is a little signal), fully
        // deterministic, single-pass, no per-frame animation cost.
        self.chrome.rasterize_bg(ctx.damage, |canvas| draw_starfield(canvas));
        self.chrome
            .rasterize_perimeter(target, buf_w, buf_h, ctx.clip_mask);
        self.chrome
            .rasterize_chrome(ctx.damage, ctx.text, ctx.clip_mask);

        // Plot region — straight into the present buffer. Notification mode plots the
        // synth's S43 voice(t); otherwise the parsed formula curve.
        let label_font_size = (ctx.viewport.effective_span() / 64.0).max(10.0);
        let base = self.base;
        if self.plot_rect.w > 4 && self.plot_rect.h > 4 {
            if let Some(voice) = self.notification {
                plot::draw_plot_curve(
                    target,
                    ctx.text,
                    ctx.damage,
                    buf_w,
                    buf_h,
                    self.plot_rect,
                    self.plot_view,
                    label_font_size,
                    base,
                    |x: S43| voice.voice(x.to_f32()),
                );
            } else {
                let formula = self
                    .formula
                    .as_ref()
                    .and_then(|r| r.as_ref().ok())
                    .map(|v| v.as_slice());
                let parse_failed = matches!(&self.formula, Some(Err(_)));
                plot::draw_plot(
                    target,
                    ctx.text,
                    ctx.damage,
                    buf_w,
                    buf_h,
                    self.plot_rect,
                    self.plot_view,
                    label_font_size,
                    formula,
                    parse_failed,
                    base,
                );
            }
        }

        // Widgets paint on top, stamping their hit ids into the chrome's hit map.
        {
            let mut canvas = Canvas::new(target, buf_w, buf_h, ctx.damage);
            let id = self.formula_box.hit_id();
            self.formula_box.render_content_into(
                &mut canvas,
                0.0,
                0.0,
                ctx.text,
                None,
                None,
                Some(&mut self.chrome.hit_test_map),
                id,
            );
            let base_id = self.base_box.hit_id();
            self.base_box.render_content_into(
                &mut canvas,
                0.0,
                0.0,
                ctx.text,
                None,
                None,
                Some(&mut self.chrome.hit_test_map),
                base_id,
            );
            let did = self.duration_box.hit_id();
            self.duration_box.render_content_into(
                &mut canvas,
                0.0,
                0.0,
                ctx.text,
                None,
                None,
                Some(&mut self.chrome.hit_test_map),
                did,
            );
            let bid = self.play_button.hit_id();
            self.play_button.render_content_into(
                &mut canvas,
                0.0,
                0.0,
                ctx.text,
                None,
                Some(&mut self.chrome.hit_test_map),
                bid,
            );
        }

        self.chrome.flatten_into(target, buf_w, buf_h, None);

        // Blinkey for whichever textbox is focused, painted on top.
        if self.current_focus == Some(self.formula_box.hit_id()) {
            let mut canvas = Canvas::new(target, buf_w, buf_h, ctx.damage);
            self.formula_box.render_blinkey_into(&mut canvas, 0.0, 0.0);
        } else if self.current_focus == Some(self.base_box.hit_id()) {
            let mut canvas = Canvas::new(target, buf_w, buf_h, ctx.damage);
            self.base_box.render_blinkey_into(&mut canvas, 0.0, 0.0);
        } else if self.current_focus == Some(self.duration_box.hit_id()) {
            let mut canvas = Canvas::new(target, buf_w, buf_h, ctx.damage);
            self.duration_box.render_blinkey_into(&mut canvas, 0.0, 0.0);
        }

        // Debug hit-map overlay (`[]h`): replace every pixel with its hit-id's flat colour
        // so interactive zones are unmistakable. Drawn last, over everything.
        if self.show_hitmask && !self.debug_hit_colours.is_empty() {
            let map = &self.chrome.hit_test_map;
            let n = map.len().min(target.len());
            for i in 0..n {
                target[i] = self
                    .debug_hit_colours
                    .get(map[i] as usize)
                    .copied()
                    .unwrap_or(0);
            }
        }

        // Debug-chord hint panel — visible while both `[` and `]` are held.
        if self.chord.both_held(Instant::now()) {
            let span = ctx.viewport.effective_span();
            let mut canvas = Canvas::new(target, buf_w, buf_h, ctx.damage);
            fluor::paint::draw_chord_hint(&mut canvas, ctx.text, DEBUG_CHORDS, span);
        }
    }

    fn cursor_for(&self, x: Coord, y: Coord, ctx: &Context) -> CursorIcon {
        let hit = self.chrome.hit_at(x, y);
        if self.chrome.owns_hit(hit) {
            return CursorIcon::Pointer;
        }
        if hit == self.play_button.hit_id() {
            return CursorIcon::Pointer;
        }
        if hit == self.formula_box.hit_id()
            || hit == self.base_box.hit_id()
            || hit == self.duration_box.hit_id()
        {
            return CursorIcon::Text;
        }
        match chrome::get_resize_edge(ctx.viewport.width_px, ctx.viewport.height_px, x, y) {
            ResizeEdge::Top | ResizeEdge::Bottom => CursorIcon::NsResize,
            ResizeEdge::Left | ResizeEdge::Right => CursorIcon::EwResize,
            ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorIcon::NwseResize,
            ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorIcon::NeswResize,
            ResizeEdge::None => {
                if self.point_in_plot(x, y) {
                    CursorIcon::Default
                } else {
                    CursorIcon::Default
                }
            }
        }
    }

    fn initial_size(&self, monitor: (u32, u32)) -> (u32, u32) {
        // Match pre-migration launch geometry: 3/4 of the smaller screen dimension tall,
        // 5:4 landscape. Gives the plot a comfortable aspect on first open.
        let win_h = monitor.0.min(monitor.1) * 3 / 4;
        let win_w = win_h * 5 / 4;
        (win_w.max(1), win_h.max(1))
    }

    fn wake_at(&self) -> Option<Instant> {
        self.blink.next_tick()
    }

    fn tick(&mut self, _ctx: &mut Context) -> bool {
        let mut needs_redraw = false;
        if self.blink.poll(Instant::now()) {
            // flip_blinkey is a no-op on an unfocused box, so flipping all is safe and
            // covers whichever currently holds focus.
            let a = self.formula_box.flip_blinkey();
            let b = self.base_box.flip_blinkey();
            let c = self.duration_box.flip_blinkey();
            if a || b || c {
                needs_redraw = true;
            }
        }
        // Play button → evaluate + play the typed formula (same as Enter).
        if self.play_button.take_click() {
            self.play_formula();
            needs_redraw = true;
        }
        needs_redraw
    }
}

impl PlotypusApp {
    /// Keyboard handling: Tab/Esc focus control, Enter/Ctrl+P play, else deliver to the
    /// focused widget and re-parse on textbox edits.
    fn handle_key(&mut self, kev: &KeyEvent, ctx: &mut Context) -> EventResponse {
        // --- Debug chord: hold `[` AND `]`, then press an action key. Bracket tracking
        // must run on BOTH press and release (before the key-down early-return) so the
        // held-state is current. Brackets still type into the focused box as normal.
        if let Key::Character(c) = &kev.logical_key {
            let now = Instant::now();
            let s = c.as_str();
            if s == "[" || s == "]" {
                self.chord.note(s.chars().next().unwrap(), kev.state, now);
                ctx.window.request_redraw();
            } else if kev.state == ElementState::Pressed
                && !kev.repeat
                && self.chord.both_held(now)
            {
                if let Some(ac) = c.to_ascii_lowercase().chars().next() {
                    if self.handle_debug_chord(ac, ctx) {
                        return EventResponse::Handled;
                    }
                }
            }
        }

        if kev.state != ElementState::Pressed {
            return EventResponse::Pass;
        }
        let shift = self.modifiers.shift_key();
        let zoom_mod = self.modifiers.control_key() || self.modifiers.super_key();

        // Tab / Shift+Tab focus cycle.
        if matches!(kev.logical_key, Key::Named(NamedKey::Tab)) {
            let dir = if shift { TabDir::Backward } else { TabDir::Forward };
            let current = self.current_focus;
            let next = widget::linear_tab_next(self as &mut dyn Container, current, dir);
            self.change_focus(next, ctx);
            return EventResponse::Handled;
        }
        // Escape clears focus.
        if matches!(kev.logical_key, Key::Named(NamedKey::Escape)) {
            self.change_focus(None, ctx);
            return EventResponse::Handled;
        }
        // Ctrl/Cmd+P → play the Photon notification (the deterministic demo sound).
        if zoom_mod {
            if let Key::Character(c) = &kev.logical_key {
                if c.eq_ignore_ascii_case("p") {
                    self.notification_seed = self.notification_seed.wrapping_add(0x9E37_79B9);
                    let seed = self.notification_seed;
                    self.play_notification(seed);
                    ctx.window.request_redraw();
                    return EventResponse::Handled;
                }
            }
        }
        // Enter → commit the typed formula: evaluate, replot, and play it as audio.
        if matches!(kev.logical_key, Key::Named(NamedKey::Enter)) {
            self.play_formula();
            ctx.window.request_redraw();
            return EventResponse::Handled;
        }

        // Deliver to the focused widget. Edits update the box's text but the plot does
        // NOT re-render until Enter (plot-on-commit, not live) — so half-typed
        // expressions don't flash broken curves.
        let Some(focus_id) = self.current_focus else {
            return EventResponse::Pass;
        };
        let mods = self.modifiers;
        let text = &mut *ctx.text;
        let response = widget::dispatch_key(self as &mut dyn Container, focus_id, kev, mods, text);

        if matches!(response, EventResponse::Handled) {
            if focus_id == self.base_box.hit_id() {
                // The base box holds a single char. Adopt the last char typed as the new
                // base (if it maps to 2..=36), collapse the box back to one char, and
                // re-parse + relabel immediately so the plot reflects the new base.
                let typed = self.base_box.chars.last().copied();
                self.base = typed.and_then(formula::char_to_base).unwrap_or(self.base);
                self.base_box.clear();
                self.base_box.insert_char(formula::base_to_char(self.base), &mut *ctx.text);
                self.reparse_formula();
            }
            if focus_id == self.formula_box.hit_id()
                || focus_id == self.base_box.hit_id()
                || focus_id == self.duration_box.hit_id()
            {
                // Keep the cursor solid + blink-restarted while actively typing.
                self.blink.start(Instant::now());
                ctx.window.request_redraw();
            }
        }
        response
    }

    /// Apply a `[]`+key debug toggle. Returns true if the key was a known debug action.
    /// Most toggles just flip a Fluor atomic that its finalize / host already honors;
    /// the hitmask is the one Plotypus paints itself (see `render`). Debug chords are
    /// listed in `DEBUG_CHORDS` for the on-screen hint.
    fn handle_debug_chord(&mut self, ac: char, ctx: &mut Context) -> bool {
        use fluor::paint as fp;
        use std::sync::atomic::Ordering::Relaxed;
        let flip = |a: &std::sync::atomic::AtomicBool| {
            let v = !a.load(Relaxed);
            a.store(v, Relaxed);
            v
        };
        match ac {
            'h' => {
                self.show_hitmask = !self.show_hitmask;
                fp::DEBUG_SHOW_HITMASK.store(self.show_hitmask, Relaxed);
                if self.show_hitmask {
                    self.regen_debug_hit_colours();
                }
            }
            'a' => {
                // Cycle off → grayscale → force-opaque → off.
                let next = (fp::DEBUG_SHOW_ALPHA.load(Relaxed) + 1) % 3;
                fp::DEBUG_SHOW_ALPHA.store(next, Relaxed);
            }
            'p' => {
                flip(&fp::DEBUG_SKIP_PREMULT);
            }
            'c' => {
                flip(&fp::DEBUG_SKIP_CHROME);
                self.chrome.invalidate_chrome();
            }
            'l' => {
                flip(&fp::DEBUG_SKIP_CONTROLS);
                self.chrome.invalidate_chrome();
            }
            'f' => {
                flip(&fp::DEBUG_SHOW_FPS);
            }
            'w' => {
                flip(&fp::DEBUG_SHOW_DAMAGE);
            }
            'd' => {
                flip(&fp::DEBUG_SHOW_FADE);
            }
            'b' => {
                flip(&fp::DEBUG_SHOW_OPAQUE_SCAN);
            }
            _ => return false,
        }
        ctx.window.request_redraw();
        true
    }

    /// Fill `debug_hit_colours` with 256 distinct opaque colours (darkness convention) so
    /// the hitmask overlay shows each hit-id as a distinct flat colour. xorshift32 seeded
    /// from the frame-independent address of `self` — debug-quality, no determinism need.
    fn regen_debug_hit_colours(&mut self) {
        let mut s: u32 = (self as *const _ as usize as u32) | 1;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s
        };
        self.debug_hit_colours.clear();
        self.debug_hit_colours.reserve(256);
        for _ in 0..256 {
            let r = (next() >> 16) as u8;
            let g = (next() >> 16) as u8;
            let b = (next() >> 16) as u8;
            self.debug_hit_colours
                .push(fluor::paint::pack_argb(r, g, b, 255));
        }
    }
}

/// `[]`-chord debug bindings, for the on-screen hint (`draw_chord_hint`) shown while both
/// brackets are held. Keys match the dispatch in `handle_debug_chord`.
const DEBUG_CHORDS: &[(&str, &str)] = &[
    ("H", "Hit-map overlay"),
    ("A", "Alpha view (cycle)"),
    ("P", "Skip premultiply"),
    ("C", "Skip chrome"),
    ("L", "Skip controls"),
    ("F", "FPS strip"),
    ("W", "Damage outline"),
    ("D", "Screen decay"),
    ("B", "Opaque-scan tint"),
];

/// Decode the bundled plotypus PNG into a Fluor app-icon orb. Resized to 256×256 (Fluor's
/// canonical orb size; the chrome scales it to the slot via nearest-neighbour) and packed
/// into Fluor's α + darkness convention (α = `0xFF` opaque, RGB = `255 − visible`). The
/// chrome masks the square image to a disc at draw time. Returns `None` if decoding fails —
/// the chrome then simply renders no orb.
fn load_orb() -> Option<fluor::host::icon::Icon> {
    const ORB_PNG: &[u8] = include_bytes!("../../assets/plotypus.png");
    let img = image::load_from_memory(ORB_PNG).ok()?;
    let resized = img.resize_exact(256, 256, image::imageops::FilterType::Lanczos3);
    let rgb = resized.to_rgb8();
    let (width, height) = (rgb.width(), rgb.height());
    let pixels = rgb
        .pixels()
        .map(|p| {
            let [r, g, b] = p.0;
            0xFF00_0000
                | (((255 - r) as u32) << 16)
                | (((255 - g) as u32) << 8)
                | ((255 - b) as u32)
        })
        .collect();
    Some(fluor::host::icon::Icon {
        width,
        height,
        pixels,
    })
}

/// SplitMix64 finalizer — same well-mixed hash the synth uses for per-user seeds. Maps an
/// arbitrary `u64` key to a uniformly-spread `u64`. Used to scatter stars deterministically.
#[inline]
fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Night-sky starfield background. A vertical gradient (deep space-blue at the top fading
/// to near-black at the bottom) seeds the canvas; a sparse, deterministic star field is
/// stamped on top — each star's position + brightness comes from `splitmix64`, the same
/// hash that derives the Photon-notification voices, so the background and the sound are
/// cut from the same cloth. A handful of the brightest stars get a faint cross-glint.
///
/// Everything is written in Fluor's darkness convention via `pack_argb`. The gradient runs
/// row-parallel (`par_rows`); the star pass is a single sparse walk. Static — no animation,
/// so it only repaints when the bg layer is marked dirty.
fn draw_starfield(canvas: &mut Canvas) {
    use fluor::paint::pack_argb;
    let w = canvas.width;
    let h = canvas.height;
    if w < 2 || h < 2 {
        return;
    }
    let pixels: &mut [u32] = canvas.pixels;

    // --- Vertical gradient. Top deep-blue → bottom near-black. One precomputed colour
    // per row (the gradient varies only in y), then a flat row fill. ---
    for y in 0..h {
        let t = y as f32 / (h - 1).max(1) as f32;
        let px = pack_argb(lerp_u8(10, 2, t), lerp_u8(16, 4, t), lerp_u8(38, 9, t), 255);
        let row = &mut pixels[y * w..y * w + w];
        row.fill(px);
    }

    // --- Stars. Walk a coarse grid of cells; each cell deterministically may hold one
    // star, jittered within the cell. Density + brightness from the hash. ---
    const CELL: usize = 14; // avg one candidate star per 14×14 px
    for cy in 0..(h / CELL) {
        for cx in 0..(w / CELL) {
            let key = (cy as u64) << 32 | cx as u64;
            let hsh = splitmix64(key ^ 0x5060_7080_90A0_B0C0);
            // ~38% of cells actually get a star — sparse, not a checkerboard.
            if (hsh & 0xFF) > 97 {
                continue;
            }
            // Jitter position inside the cell.
            let jx = ((hsh >> 8) as usize) % CELL;
            let jy = ((hsh >> 16) as usize) % CELL;
            let x = cx * CELL + jx;
            let y = cy * CELL + jy;
            if x >= w || y >= h {
                continue;
            }
            // Brightness class from more hash bits: most stars dim, a few bright.
            let lvl = (hsh >> 24) & 0xFF;
            let bright: u8 = if lvl < 12 {
                235 // rare brilliant star
            } else if lvl < 60 {
                150
            } else {
                85
            };
            // Slight cool tint — stars are faintly blue-white.
            put_star(pixels, w, h, x, y, bright, bright, (bright as u16 + 20).min(255) as u8);
            // The brilliant ones get a 1-px cross-glint at half brightness.
            if bright == 235 {
                let glint = 110u8;
                put_star(pixels, w, h, x.wrapping_sub(1), y, glint, glint, glint);
                put_star(pixels, w, h, x + 1, y, glint, glint, glint);
                put_star(pixels, w, h, x, y.wrapping_sub(1), glint, glint, glint);
                put_star(pixels, w, h, x, y + 1, glint, glint, glint);
            }
        }
    }
}

/// Write a single opaque star pixel (darkness convention) if `(x, y)` is in bounds.
#[inline]
fn put_star(pixels: &mut [u32], w: usize, h: usize, x: usize, y: usize, r: u8, g: u8, b: u8) {
    if x < w && y < h {
        pixels[y * w + x] = fluor::paint::pack_argb(r, g, b, 255);
    }
}

/// Integer-channel linear interpolation `a → b` by `t ∈ [0,1]`.
#[inline]
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8
}
