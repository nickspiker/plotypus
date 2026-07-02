pub mod audio;
pub mod formula;
pub mod plotnum;
pub mod synth;
pub mod ui;

#[cfg(feature = "logging")]
pub fn log(msg: &str) {
    eprintln!("{}", msg);
}

#[cfg(not(feature = "logging"))]
pub fn log(_msg: &str) {}

#[macro_export]
macro_rules! debug_println {
    ($($arg:tt)*) => {
        #[cfg(feature = "debug-keys")]
        eprintln!($($arg)*);
    };
}

pub static DEBUG_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
