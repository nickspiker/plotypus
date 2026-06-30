//! Plotypus UI. The window/event/render/chrome foundation is Fluor's; this module holds
//! Plotypus's domain UI: the `FluorApp` (`app`), the plot region (`plot`), and theme
//! colours (`theme`) used by the plot's axis labels.

pub mod app;
pub mod plot;
pub mod theme;

pub use app::PlotypusApp;
