//! macOS renderer — softbuffer (direct CPU buffer via Core Graphics)
//!
//! The compositor gets a direct pointer to our pixel buffer.
//! Single-pixel read/write with zero copy overhead.

use softbuffer::{Context, Surface};
use std::num::NonZeroU32;
use winit::window::Window;

/// Buffer guard — Deref gives you the compositor's actual buffer.
pub struct SoftbufferBuffer<'a> {
    inner: softbuffer::Buffer<'a, &'static Window, &'static Window>,
}

impl<'a> std::ops::Deref for SoftbufferBuffer<'a> {
    type Target = [u32];
    fn deref(&self) -> &[u32] {
        &self.inner
    }
}

impl<'a> std::ops::DerefMut for SoftbufferBuffer<'a> {
    fn deref_mut(&mut self) -> &mut [u32] {
        &mut self.inner
    }
}

impl<'a> SoftbufferBuffer<'a> {
    pub fn as_mut(&mut self) -> &mut [u32] {
        &mut self.inner
    }

    pub fn mark_rows(&self, _y_start: u32, _y_end: u32) {}
    pub fn mark_all(&self) {}

    pub fn present(self) -> Result<(), ()> {
        self.inner.present().map_err(|_| ())
    }
}

pub struct Renderer {
    #[allow(dead_code)]
    context: Context<&'static Window>,
    surface: Surface<&'static Window, &'static Window>,
    width: u32,
    height: u32,
}

impl Renderer {
    pub fn new(window: &Window, width: u32, height: u32) -> Self {
        // SAFETY: The surface lives inside Renderer, which is owned by PhotonApp.
        // PhotonApp is dropped before Window is dropped in main.rs.
        let static_window: &'static Window = unsafe { std::mem::transmute(window) };

        let context = Context::new(static_window).unwrap();
        let mut surface = Surface::new(&context, static_window).unwrap();

        surface
            .resize(
                NonZeroU32::new(width).unwrap(),
                NonZeroU32::new(height).unwrap(),
            )
            .unwrap();

        Self {
            context,
            surface,
            width,
            height,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.width = width;
            self.height = height;
            let _ = self.surface.resize(
                NonZeroU32::new(width).unwrap(),
                NonZeroU32::new(height).unwrap(),
            );
        }
    }

    pub fn mark_rows(&mut self, _y_start: u32, _y_end: u32) {}
    pub fn mark_all(&mut self) {}

    pub fn lock_buffer(&mut self) -> SoftbufferBuffer<'_> {
        SoftbufferBuffer {
            inner: self.surface.buffer_mut().unwrap(),
        }
    }
}
