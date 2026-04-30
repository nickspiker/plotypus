//! Mouse + keyboard handlers for chrome interactions. Lifted concept from Photon's mouse.rs/keyboard.rs but trimmed to what Plotypus needs:
//! - Window-control button click + hover
//! - Edge-resize detection + cursor shape feedback
//! - Body drag (delegates to winit::Window::drag_window)
//! - Ctrl+D / Ctrl+H / Ctrl+T debug toggles
//! - Ctrl/Cmd +/- zoom

use crate::DEBUG_ENABLED;
use crate::formula;
use crate::ui::app::{FocusedBox, HoveredButton, PlotDragMode, PlotypusApp, ResizeEdge};
use crate::ui::compositing::{
    HIT_BASE_BOX, HIT_BODY, HIT_CLOSE_BUTTON, HIT_INPUT_BOX, HIT_MAXIMIZE_BUTTON,
    HIT_MINIMIZE_BUTTON, HIT_PLOT_AREA, HIT_RANGE_XMIN, HIT_RANGE_XMAX, HIT_RANGE_YMIN,
    HIT_RANGE_YMAX,
};
use crate::ui::input_box::{index_from_x, measure_char_width};
use std::sync::atomic::Ordering;
use winit::event::{ElementState, KeyEvent, MouseButton};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window};

pub enum ClickAction {
    None,
    Exit,
    Minimize,
    ToggleFullscreen,
    DragWindow,
    DragResize(ResizeEdge),
}

/// Zoom-shortcut modifier: Cmd on macOS, Ctrl elsewhere. Same key used for `Ctrl+D/H/T/+/-/0`.
#[inline]
pub fn zoom_modifier(mods: &ModifiersState) -> bool {
    #[cfg(target_os = "macos")]
    {
        mods.super_key()
    }
    #[cfg(not(target_os = "macos"))]
    {
        mods.control_key()
    }
}

pub enum KeyAction {
    None,
    Exit,
    /// Redraw requested (state changed)
    Redraw,
}

impl PlotypusApp {
    pub fn update_modifiers(&mut self, mods: ModifiersState) {
        self.modifiers = mods;
    }

    /// Decide what a left-mouse-down at the current cursor position should do. Caller owns the window and performs the action; this keeps PlotypusApp independent of the winit::Window handle.
    pub fn handle_mouse_click(&mut self, state: ElementState, button: MouseButton) -> ClickAction {
        if button != MouseButton::Left {
            return ClickAction::None;
        }
        match state {
            ElementState::Pressed => {
                self.mouse_button_pressed = true;
                let hit = self.hit_test(self.mouse_x, self.mouse_y);
                match hit {
                    HIT_CLOSE_BUTTON => return ClickAction::Exit,
                    HIT_MINIMIZE_BUTTON => return ClickAction::Minimize,
                    HIT_MAXIMIZE_BUTTON => return ClickAction::ToggleFullscreen,
                    _ => {}
                }
                let edge = self.get_resize_edge(self.mouse_x, self.mouse_y);
                if edge != ResizeEdge::None {
                    self.resize_edge = edge;
                    return ClickAction::DragResize(edge);
                }
                if hit == HIT_INPUT_BOX {
                    self.focus_box(FocusedBox::Formula);
                    self.focus_textbox_at(self.mouse_x);
                    return ClickAction::None;
                }
                if hit == HIT_BASE_BOX {
                    self.focus_box(FocusedBox::Base);
                    return ClickAction::None;
                }
                if let Some(fb) = match hit {
                    HIT_RANGE_XMIN => Some(FocusedBox::RangeXMin),
                    HIT_RANGE_XMAX => Some(FocusedBox::RangeXMax),
                    HIT_RANGE_YMIN => Some(FocusedBox::RangeYMin),
                    HIT_RANGE_YMAX => Some(FocusedBox::RangeYMax),
                    _ => None,
                } {
                    self.focus_range_box(fb);
                    return ClickAction::None;
                }
                if hit == HIT_PLOT_AREA {
                    self.defocus_all();
                    if self.modifiers.alt_key() {
                        self.start_plot_drag(PlotDragMode::Pan, self.mouse_x, self.mouse_y);
                        return ClickAction::None;
                    }
                    if zoom_modifier(&self.modifiers) {
                        self.start_plot_drag(PlotDragMode::Zoom, self.mouse_x, self.mouse_y);
                        return ClickAction::None;
                    }
                    return ClickAction::DragWindow;
                }
                if hit == HIT_BODY {
                    self.defocus_all();
                    return ClickAction::DragWindow;
                }
                ClickAction::None
            }
            ElementState::Released => {
                self.mouse_button_pressed = false;
                self.is_dragging_resize = false;
                self.resize_edge = ResizeEdge::None;
                self.end_plot_drag();
                ClickAction::None
            }
        }
    }

    /// Update hover state and cursor icon in response to mouse motion. Returns true if a redraw is needed (hover changed).
    pub fn handle_mouse_move(&mut self, window: &Window, x: f32, y: f32) -> bool {
        self.mouse_x = x;
        self.mouse_y = y;

        if self.plot_drag.is_some() {
            let cursor = match self.plot_drag.unwrap().mode {
                PlotDragMode::Pan => CursorIcon::Grabbing,
                PlotDragMode::Zoom => CursorIcon::AllScroll,
            };
            window.set_cursor(cursor);
            return self.update_plot_drag(x, y);
        }

        if !self.mouse_button_pressed {
            self.is_dragging_resize = false;
        }

        let hit = self.hit_test(x, y);
        let on_button = matches!(
            hit,
            HIT_CLOSE_BUTTON | HIT_MAXIMIZE_BUTTON | HIT_MINIMIZE_BUTTON
        );

        let new_hovered = if on_button {
            match hit {
                HIT_CLOSE_BUTTON => HoveredButton::Close,
                HIT_MAXIMIZE_BUTTON => HoveredButton::Maximize,
                HIT_MINIMIZE_BUTTON => HoveredButton::Minimize,
                _ => HoveredButton::None,
            }
        } else if hit == HIT_BODY {
            HoveredButton::Body
        } else {
            HoveredButton::None
        };

        let edge = if on_button {
            ResizeEdge::None
        } else {
            self.get_resize_edge(x, y)
        };

        let cursor = if on_button {
            CursorIcon::Pointer
        } else if edge != ResizeEdge::None {
            match edge {
                ResizeEdge::Top | ResizeEdge::Bottom => CursorIcon::NsResize,
                ResizeEdge::Left | ResizeEdge::Right => CursorIcon::EwResize,
                ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorIcon::NwseResize,
                ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorIcon::NeswResize,
                ResizeEdge::None => CursorIcon::Default,
            }
        } else if hit == HIT_PLOT_AREA {
            if self.modifiers.alt_key() {
                CursorIcon::Grab
            } else if zoom_modifier(&self.modifiers) {
                CursorIcon::ZoomIn
            } else {
                CursorIcon::Crosshair
            }
        } else if hit == HIT_INPUT_BOX
            || hit == HIT_BASE_BOX
            || hit == HIT_RANGE_XMIN
            || hit == HIT_RANGE_XMAX
            || hit == HIT_RANGE_YMIN
            || hit == HIT_RANGE_YMAX
        {
            CursorIcon::Text
        } else {
            CursorIcon::Default
        };
        window.set_cursor(cursor);

        let edge_changed = self.resize_edge != edge && !self.mouse_button_pressed;
        if edge_changed {
            self.resize_edge = edge;
        }

        let hovered_changed = self.hovered_button != new_hovered;
        if hovered_changed {
            self.prev_hovered_button = self.hovered_button;
            self.hovered_button = new_hovered;
        }

        // Debug overlay shows live edge/hover state, so redraw on any motion when debug is on.
        let needs_redraw = hovered_changed || (self.debug && edge_changed) || self.debug;
        if needs_redraw {
            self.window_dirty = true;
        }
        needs_redraw
    }

    pub fn handle_keyboard(&mut self, event: KeyEvent) -> KeyAction {
        if event.state != ElementState::Pressed {
            return KeyAction::None;
        }

        if let Key::Named(NamedKey::Escape) = event.logical_key {
            return KeyAction::Exit;
        }

        let zoom_mod = zoom_modifier(&self.modifiers);
        // Any "command-class" modifier blocks text input so chord shortcuts (Ctrl+D, Alt+anything reserved for future bindings, Cmd on Mac) are never swallowed by the focused textbox. Shift is not a command modifier — Shift+letter still types a capital letter.
        let any_cmd_mod =
            self.modifiers.control_key() || self.modifiers.alt_key() || self.modifiers.super_key();

        if !any_cmd_mod {
            match self.focused_box {
                FocusedBox::Formula => {
                    if let Some(action) = self.handle_input_text(&event) {
                        return action;
                    }
                }
                FocusedBox::Base => {
                    if let Some(action) = self.handle_base_input(&event) {
                        return action;
                    }
                }
                FocusedBox::RangeXMin | FocusedBox::RangeXMax
                | FocusedBox::RangeYMin | FocusedBox::RangeYMax => {
                    if let Some(action) = self.handle_range_input(&event) {
                        return action;
                    }
                }
                FocusedBox::None => {}
            }
        }

        if zoom_mod {
            if let Key::Character(ref c) = event.logical_key {
                let s: &str = c.as_ref();
                if s.eq_ignore_ascii_case("d") {
                    self.debug = !self.debug;
                    DEBUG_ENABLED.store(self.debug, Ordering::Relaxed);
                    self.window_dirty = true;
                    return KeyAction::Redraw;
                }
                if s.eq_ignore_ascii_case("h") {
                    self.debug_hit_test = !self.debug_hit_test;
                    self.show_textbox_mask = false;
                    if self.debug_hit_test {
                        self.regenerate_debug_hit_colours();
                    }
                    self.window_dirty = true;
                    return KeyAction::Redraw;
                }
                if s.eq_ignore_ascii_case("t") {
                    self.show_textbox_mask = !self.show_textbox_mask;
                    self.debug_hit_test = false;
                    self.window_dirty = true;
                    return KeyAction::Redraw;
                }
                if s == "+" || s == "=" {
                    self.adjust_zoom(1.0);
                    return KeyAction::Redraw;
                }
                if s == "-" || s == "_" {
                    self.adjust_zoom(-1.0);
                    return KeyAction::Redraw;
                }
                if s == "0" {
                    self.ru = 1.0;
                    self.window_dirty = true;
                    return KeyAction::Redraw;
                }
            }
        }

        KeyAction::None
    }

    /// Handle a key as formula-input editing. Returns `Some(KeyAction)` if the key was consumed, `None` to let the outer handler keep dispatching.
    fn handle_input_text(&mut self, event: &KeyEvent) -> Option<KeyAction> {
        let mut changed = false;
        match &event.logical_key {
            Key::Named(NamedKey::Backspace) => {
                changed = self.text_state.delete_backward();
            }
            Key::Named(NamedKey::Delete) => {
                changed = self.text_state.delete_forward();
            }
            Key::Named(NamedKey::ArrowLeft) => {
                changed = self.text_state.move_left();
            }
            Key::Named(NamedKey::ArrowRight) => {
                changed = self.text_state.move_right();
            }
            Key::Named(NamedKey::Home) => {
                changed = self.text_state.home();
            }
            Key::Named(NamedKey::End) => {
                changed = self.text_state.end();
            }
            Key::Named(NamedKey::Enter) => {}
            Key::Named(NamedKey::Space) => {
                let font_size = self.input_font_size();
                let w = measure_char_width(&mut self.text_renderer, ' ', font_size);
                self.text_state.insert(' ', w);
                changed = true;
            }
            Key::Character(c) => {
                let font_size = self.input_font_size();
                for ch in c.chars() {
                    if ch.is_control() {
                        continue;
                    }
                    let w = measure_char_width(&mut self.text_renderer, ch, font_size);
                    self.text_state.insert(ch, w);
                    changed = true;
                }
            }
            _ => return None,
        }
        if changed {
            self.text_dirty = true;
            Some(KeyAction::Redraw)
        } else {
            Some(KeyAction::None)
        }
    }

    /// Focus the textbox and place the cursor at the click x. Reuses the cached last_layout from the most recent full draw so cursor placement matches the rendered layout exactly.
    pub fn focus_textbox_at(&mut self, click_x: f32) {
        let was_focused = self.text_state.focused;
        self.text_state.focused = true;
        if let Some(layout) = self.last_layout {
            let idx = index_from_x(&self.text_state, &layout, click_x);
            self.text_state.blinkey_index = idx;
        } else {
            self.text_state.blinkey_index = self.text_state.chars.len();
        }
        if !was_focused {
            // Frame colour changes — needs full redraw of input chrome.
            self.window_dirty = true;
        } else {
            self.text_dirty = true;
        }
    }

    /// Set focus to a specific box, defocusing any previously focused box.
    pub fn focus_box(&mut self, target: FocusedBox) {
        if self.focused_box == target {
            return;
        }
        // Commit range box edits when leaving a range box
        self.commit_range_edit();
        self.focused_box = target;
        self.text_state.focused = target == FocusedBox::Formula;
        self.window_dirty = true;
    }

    /// Focus a range box and prepare it for editing.
    pub fn focus_range_box(&mut self, target: FocusedBox) {
        self.commit_range_edit();
        self.focused_box = target;
        self.text_state.focused = false;
        let idx = self.range_box_index(target).unwrap();
        self.range_text_states[idx].focused = true;
        self.range_text_states[idx].blinkey_index = self.range_text_states[idx].chars.len();
        self.window_dirty = true;
    }

    /// Defocus all boxes, committing any pending range edits.
    pub fn defocus_all(&mut self) {
        self.commit_range_edit();
        self.focused_box = FocusedBox::None;
        self.text_state.focused = false;
        for ts in &mut self.range_text_states {
            ts.focused = false;
        }
        self.window_dirty = true;
    }

    /// If a range box is focused, parse its text and apply to plot_view.
    fn commit_range_edit(&mut self) {
        let idx = match self.focused_box {
            FocusedBox::RangeXMin => Some(0),
            FocusedBox::RangeXMax => Some(1),
            FocusedBox::RangeYMin => Some(2),
            FocusedBox::RangeYMax => Some(3),
            _ => None,
        };
        if let Some(i) = idx {
            let text: String = self.range_text_states[i].chars.iter().collect();
            if let Some(val) = formula::parse_value(&text, self.base) {
                match i {
                    0 => self.plot_view.x_min = val,
                    1 => self.plot_view.x_max = val,
                    2 => self.plot_view.y_min = val,
                    3 => self.plot_view.y_max = val,
                    _ => {}
                }
            }
        }
    }

    fn range_box_index(&self, fb: FocusedBox) -> Option<usize> {
        match fb {
            FocusedBox::RangeXMin => Some(0),
            FocusedBox::RangeXMax => Some(1),
            FocusedBox::RangeYMin => Some(2),
            FocusedBox::RangeYMax => Some(3),
            _ => None,
        }
    }

    /// Handle keyboard input for the base box.
    fn handle_base_input(&mut self, event: &KeyEvent) -> Option<KeyAction> {
        if let Key::Named(NamedKey::Escape) = event.logical_key {
            self.defocus_all();
            return Some(KeyAction::Redraw);
        }
        if let Key::Character(ref c) = event.logical_key {
            for ch in c.chars() {
                if let Some(new_base) = formula::char_to_base(ch) {
                    self.base = new_base;
                    // Re-parse formula with new base
                    self.formula = None;
                    self.window_dirty = true;
                    return Some(KeyAction::Redraw);
                }
            }
        }
        Some(KeyAction::None)
    }

    /// Handle keyboard input for a focused range box.
    fn handle_range_input(&mut self, event: &KeyEvent) -> Option<KeyAction> {
        let idx = self.range_box_index(self.focused_box)?;
        let mut changed = false;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.defocus_all();
                return Some(KeyAction::Redraw);
            }
            Key::Named(NamedKey::Enter) => {
                self.commit_range_edit();
                self.defocus_all();
                return Some(KeyAction::Redraw);
            }
            Key::Named(NamedKey::Backspace) => {
                changed = self.range_text_states[idx].delete_backward();
            }
            Key::Named(NamedKey::Delete) => {
                changed = self.range_text_states[idx].delete_forward();
            }
            Key::Named(NamedKey::ArrowLeft) => {
                changed = self.range_text_states[idx].move_left();
            }
            Key::Named(NamedKey::ArrowRight) => {
                changed = self.range_text_states[idx].move_right();
            }
            Key::Named(NamedKey::Home) => {
                changed = self.range_text_states[idx].home();
            }
            Key::Named(NamedKey::End) => {
                changed = self.range_text_states[idx].end();
            }
            Key::Character(c) => {
                let font_size = self.input_font_size();
                for ch in c.chars() {
                    if ch.is_control() {
                        continue;
                    }
                    let w = measure_char_width(&mut self.text_renderer, ch, font_size);
                    self.range_text_states[idx].insert(ch, w);
                    changed = true;
                }
            }
            _ => return None,
        }
        if changed {
            self.window_dirty = true;
            Some(KeyAction::Redraw)
        } else {
            Some(KeyAction::None)
        }
    }

    fn input_font_size(&self) -> f32 {
        let btn_h = self.button_height();
        let layout = PlotypusApp::compute_layout(self.width, self.height, btn_h);
        let input_rect = layout.formula_rect;
        (input_rect.h as f32 * 0.55).max(12.0)
    }
}
