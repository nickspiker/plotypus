use plotypus::ui::app::ResizeEdge;
use plotypus::ui::input::{ClickAction, KeyAction};
use plotypus::ui::{PlotypusApp, PlotypusEvent};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Fullscreen, ResizeDirection, Window, WindowId};

struct App {
    window: Option<Window>,
    plotypus_app: Option<PlotypusApp>,
    target_frame_duration_ms: u64,
    #[allow(dead_code)]
    event_proxy: EventLoopProxy<PlotypusEvent>,
}

impl ApplicationHandler<PlotypusEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let monitor = event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next())
            .expect("No monitor found");
        let screen = monitor.size();

        self.target_frame_duration_ms = monitor
            .refresh_rate_millihertz()
            .map(|m| (1000 / (m / 1000).max(1)) as u64)
            .unwrap_or(16);

        let win_h = screen.width.min(screen.height) * 3 / 4;
        let win_w = win_h * 5 / 4;
        let x = (screen.width.saturating_sub(win_w)) / 2;
        let y = (screen.height.saturating_sub(win_h)) / 2;

        // TODO(macos): with_resizable(false) blocks winit's drag_resize_window.
        //              Need to lift Photon's manual resize tracking — see
        //              mouse.rs::apply_resize + main.rs::poll_macos_resize there.
        let attrs = Window::default_attributes()
            .with_title("Plotypus")
            .with_inner_size(PhysicalSize::new(win_w, win_h))
            .with_position(PhysicalPosition::new(x as i32, y as i32))
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(cfg!(not(target_os = "macos")));

        let window = event_loop.create_window(attrs).unwrap();
        let app = PlotypusApp::new(&window);
        window.request_redraw();
        self.window = Some(window);
        self.plotypus_app = Some(app);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => {
                if let (Some(app), Some(window)) = (&mut self.plotypus_app, &self.window) {
                    app.resize(size);
                    window.request_redraw();
                }
            }

            WindowEvent::RedrawRequested => {
                if let Some(app) = &mut self.plotypus_app {
                    app.render();
                }
            }

            WindowEvent::ModifiersChanged(modifiers) => {
                if let Some(app) = &mut self.plotypus_app {
                    app.update_modifiers(modifiers.state());
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                if let (Some(app), Some(window)) = (&mut self.plotypus_app, &self.window) {
                    if app.handle_mouse_move(window, position.x as f32, position.y as f32) {
                        window.request_redraw();
                    }
                }
            }

            WindowEvent::CursorLeft { .. } => {
                if let Some(app) = &mut self.plotypus_app {
                    app.hovered_button = plotypus::ui::app::HoveredButton::None;
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }

            WindowEvent::MouseInput { state, button, .. } => {
                let (Some(app), Some(window)) = (&mut self.plotypus_app, &self.window) else {
                    return;
                };
                let action = app.handle_mouse_click(state, button);
                match action {
                    ClickAction::Exit => event_loop.exit(),
                    ClickAction::Minimize => {
                        let _ = window.set_minimized(true);
                    }
                    ClickAction::ToggleFullscreen => {
                        let to_full = window.fullscreen().is_none();
                        window.set_fullscreen(if to_full {
                            Some(Fullscreen::Borderless(None))
                        } else {
                            None
                        });
                        app.set_fullscreen(to_full);
                        window.request_redraw();
                    }
                    ClickAction::DragWindow => {
                        let _ = window.drag_window();
                    }
                    ClickAction::DragResize(edge) => {
                        if let Some(dir) = resize_edge_to_direction(edge) {
                            let _ = window.drag_resize_window(dir);
                        }
                    }
                    ClickAction::None => {}
                }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                let (Some(app), Some(window)) = (&mut self.plotypus_app, &self.window) else {
                    return;
                };
                match app.handle_keyboard(event) {
                    KeyAction::Exit => event_loop.exit(),
                    KeyAction::Redraw => {
                        window.request_redraw();
                    }
                    KeyAction::None => {}
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {}
}

fn resize_edge_to_direction(edge: ResizeEdge) -> Option<ResizeDirection> {
    match edge {
        ResizeEdge::Top => Some(ResizeDirection::North),
        ResizeEdge::Bottom => Some(ResizeDirection::South),
        ResizeEdge::Left => Some(ResizeDirection::West),
        ResizeEdge::Right => Some(ResizeDirection::East),
        ResizeEdge::TopLeft => Some(ResizeDirection::NorthWest),
        ResizeEdge::TopRight => Some(ResizeDirection::NorthEast),
        ResizeEdge::BottomLeft => Some(ResizeDirection::SouthWest),
        ResizeEdge::BottomRight => Some(ResizeDirection::SouthEast),
        ResizeEdge::None => None,
    }
}

fn main() {
    let event_loop = EventLoop::<PlotypusEvent>::with_user_event().build().unwrap();
    let event_proxy = event_loop.create_proxy();

    let mut app = App {
        window: None,
        plotypus_app: None,
        target_frame_duration_ms: 16,
        event_proxy,
    };
    event_loop.run_app(&mut app).unwrap();
}
