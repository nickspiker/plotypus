//! Linux renderer — softbuffer with CPU buffer for Wayland triple-buffering
//!
//! Wayland compositors rotate 2–3 buffers. You never get back the one you just sent, so partial updates leave stale regions. We keep a CPU-side copy of the full frame and use softbuffer's `buffer.age()` to know how far back each compositor buffer is, then copy the dirty union into it before presenting.

use softbuffer::{Context, Surface};
use std::num::NonZeroU32;
use winit::window::Window;

/// Buffer guard — Deref gives you the CPU buffer. On present, dirty+stale strips are copied to the compositor buffer.
pub struct SoftbufferBuffer<'a> {
    inner: &'a mut Renderer,
}

impl<'a> std::ops::Deref for SoftbufferBuffer<'a> {
    type Target = [u32];
    fn deref(&self) -> &[u32] {
        &self.inner.cpu_buffer
    }
}

impl<'a> std::ops::DerefMut for SoftbufferBuffer<'a> {
    fn deref_mut(&mut self) -> &mut [u32] {
        &mut self.inner.cpu_buffer
    }
}

impl<'a> SoftbufferBuffer<'a> {
    pub fn as_mut(&mut self) -> &mut [u32] {
        &mut self.inner.cpu_buffer
    }

    pub fn mark_rows(&self, _y_start: u32, _y_end: u32) {}
    pub fn mark_all(&self) {}

    pub fn present(self) -> Result<(), ()> {
        self.inner.present_frame()
    }
}

pub struct Renderer {
    #[allow(dead_code)]
    context: Context<&'static Window>,
    surface: Surface<&'static Window, &'static Window>,
    cpu_buffer: Vec<u32>,
    width: u32,
    height: u32,
    /// Smallest y touched since last present (inclusive)
    dirty_y_min: u32,
    /// Largest y touched since last present (exclusive)
    dirty_y_max: u32,
    /// Ring of (y_min, y_max) for the last N presents, so we can replay the dirty union for any buffer age the compositor hands back.
    history: [(u32, u32); 4],
    history_idx: usize,
}

impl Renderer {
    pub fn new(window: &Window, width: u32, height: u32) -> Self {
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
            cpu_buffer: vec![0u32; (width * height) as usize],
            width,
            height,
            dirty_y_min: 0,
            dirty_y_max: height,
            history: [(0, 0); 4],
            history_idx: 0,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.width = width;
            self.height = height;
            self.cpu_buffer.resize((width * height) as usize, 0);
            self.dirty_y_min = 0;
            self.dirty_y_max = height;
            self.history = [(0, height); 4];
            let _ = self.surface.resize(
                NonZeroU32::new(width).unwrap(),
                NonZeroU32::new(height).unwrap(),
            );
        }
    }

    pub fn mark_rows(&mut self, y_start: u32, y_end: u32) {
        self.dirty_y_min = self.dirty_y_min.min(y_start);
        self.dirty_y_max = self.dirty_y_max.max(y_end);
    }

    pub fn mark_all(&mut self) {
        self.dirty_y_min = 0;
        self.dirty_y_max = self.height;
    }

    pub fn lock_buffer(&mut self) -> SoftbufferBuffer<'_> {
        SoftbufferBuffer { inner: self }
    }

    fn present_frame(&mut self) -> Result<(), ()> {
        let mut buffer = self.surface.buffer_mut().map_err(|_| ())?;
        let age = buffer.age();

        // Determine copy range: union of current dirty + however many frames this buffer is behind.
        let (copy_min, copy_max) = if age == 0 {
            // Buffer never presented — full copy
            (0, self.height)
        } else {
            // Start with current frame's dirty range
            let mut y_min = self.dirty_y_min;
            let mut y_max = self.dirty_y_max;

            // Union with previous (age-1) frames from history
            let frames_behind = (age as usize - 1).min(self.history.len());
            for i in 0..frames_behind {
                let idx = (self.history_idx + self.history.len() - 1 - i) % self.history.len();
                let (h_min, h_max) = self.history[idx];
                y_min = y_min.min(h_min);
                y_max = y_max.max(h_max);
            }

            (y_min, y_max)
        };

        // Copy strips from cpu_buffer → compositor buffer
        if copy_min < copy_max {
            let w = self.width as usize;
            let start = copy_min as usize * w;
            let end = copy_max as usize * w;
            buffer[start..end].copy_from_slice(&self.cpu_buffer[start..end]);
        }

        // Record this frame's dirty range in history ring
        self.history[self.history_idx] = (self.dirty_y_min, self.dirty_y_max);
        self.history_idx = (self.history_idx + 1) % self.history.len();

        // Reset dirty tracking
        self.dirty_y_min = u32::MAX;
        self.dirty_y_max = 0;

        buffer.present().map_err(|_| ())
    }
}
