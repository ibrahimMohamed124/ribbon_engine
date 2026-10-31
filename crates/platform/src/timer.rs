//! Frame timing with the game's historical delta clamp and frame cap.

use std::thread;
use std::time::{Duration, Instant};

const TARGET_FRAME: Duration = Duration::from_micros(16_000);

#[derive(Clone, Copy, Debug)]
pub struct FrameSample {
    pub dt: f32,
    pub fps: f32,
}

pub struct FrameTimer {
    last: Instant,
    frame_start: Instant,
    fps: f32,
}

impl FrameTimer {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            last: now,
            frame_start: now,
            fps: 60.0,
        }
    }

    pub fn begin_frame(&mut self) -> FrameSample {
        self.frame_start = Instant::now();
        let dt = self
            .frame_start
            .duration_since(self.last)
            .as_secs_f32()
            .min(0.05);
        self.last = self.frame_start;
        if dt > 0.0 {
            self.fps = self.fps * 0.9 + (1.0 / dt) * 0.1;
        }
        FrameSample { dt, fps: self.fps }
    }

    pub fn cap_frame(&self) {
        let elapsed = self.frame_start.elapsed();
        if elapsed < TARGET_FRAME {
            thread::sleep(TARGET_FRAME - elapsed);
        }
    }
}

impl Default for FrameTimer {
    fn default() -> Self {
        Self::new()
    }
}
