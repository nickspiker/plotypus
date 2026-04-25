pub mod app;
pub mod colour;
pub mod compositing;
pub mod display_profile;
pub mod drawing;
pub mod input;
pub mod text_rasterizing;
pub mod theme;

#[cfg(target_os = "linux")]
pub mod renderer_linux_softbuffer;

#[cfg(target_os = "macos")]
pub mod renderer_macos;

#[cfg(target_os = "linux")]
pub use renderer_linux_softbuffer as renderer;

#[cfg(target_os = "macos")]
pub use renderer_macos as renderer;

pub use app::PlotipusApp;

#[derive(Debug, Clone)]
pub enum PlotipusEvent {
    Redraw,
}
