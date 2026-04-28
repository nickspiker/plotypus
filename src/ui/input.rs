//! Mouse + keyboard handlers for chrome interactions.
//! Lifted concept from Photon's mouse.rs/keyboard.rs but trimmed to what Plotypus needs:
//! - Window-control button click + hover
//! - Edge-resize detection + cursor shape feedback
//! - Body drag (delegates to winit::Window::drag_window)
//! - Ctrl+D / Ctrl+H / Ctrl+T debug toggles
//! - Ctrl/Cmd +/- zoom

use crate::ui::app::{HoveredButton, PlotypusApp, ResizeEdge};
use crate::ui::compositing::{
    HIT_BODY, HIT_CLOSE_BUTTON, HIT_MAXIMIZE_BUTTON, HIT_MINIMIZE_BUTTON,
};
use crate::DEBUG_ENABLED;
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

    /// Decide what a left-mouse-down at the current cursor position should do.
    /// Caller owns the window and performs the action; this keeps PlotypusApp
    /// independent of the winit::Window handle.
    pub fn handle_mouse_click(
        &mut self,
        state: ElementState,
        button: MouseButton,
    ) -> ClickAction {
        if button != MouseButton::Left {
            return ClickAction::None;
        }
        match state {
            ElementState::Pressed => {
                self.mouse_button_pressed = true;
                let hit = self.hit_test(self.mouse_x as i32, self.mouse_y as i32);
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
                if hit == HIT_BODY {
                    return ClickAction::DragWindow;
                }
                ClickAction::None
            }
            ElementState::Released => {
                self.mouse_button_pressed = false;
                self.is_dragging_resize = false;
                self.resize_edge = ResizeEdge::None;
                ClickAction::None
            }
        }
    }

    /// Update hover state and cursor icon in response to mouse motion.
    /// Returns true if a redraw is needed (hover changed).
    pub fn handle_mouse_move(&mut self, window: &Window, x: f32, y: f32) -> bool {
        self.mouse_x = x;
        self.mouse_y = y;

        if !self.mouse_button_pressed {
            self.is_dragging_resize = false;
        }

        let hit = self.hit_test(x as i32, y as i32);
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
        } else {
            match edge {
                ResizeEdge::Top | ResizeEdge::Bottom => CursorIcon::NsResize,
                ResizeEdge::Left | ResizeEdge::Right => CursorIcon::EwResize,
                ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorIcon::NwseResize,
                ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorIcon::NeswResize,
                ResizeEdge::None => CursorIcon::Default,
            }
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

        #[cfg(target_os = "macos")]
        let zoom_mod = self.modifiers.super_key();
        #[cfg(not(target_os = "macos"))]
        let zoom_mod = self.modifiers.control_key();

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
}
