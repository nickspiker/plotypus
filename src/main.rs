//! Plotypus entry point. Fluor owns the window, event loop, and present; we hand it a
//! `PlotypusApp` (which implements `fluor::host::app::FluorApp`) and `run_app` drives it.

use plotypus::ui::PlotypusApp;

fn main() {
    fluor::host::app::run_app(PlotypusApp::new()).expect("event loop");
}
