use std::path::Path;
use std::time::Instant;

use cairo::{Format, ImageSurface};
use webp_animation::Decoder;

pub struct Anim {
    pub frames: Vec<ImageSurface>,
    pub durations_ms: Vec<u32>,
    pub width: i32,
    pub height: i32,
    pub total_ms: u32,
    pub start: Instant,
}

impl Anim {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let data = std::fs::read(path)?;
        let decoder = Decoder::new(&data)?;
        let mut frames = Vec::new();
        let mut durations_ms = Vec::new();
        let mut last_ts = 0i32;
        let mut width = 0i32;
        let mut height = 0i32;
        for frame in decoder.into_iter() {
            let (w, h) = frame.dimensions();
            width = w as i32;
            height = h as i32;
            let ts = frame.timestamp();
            let dur = (ts - last_ts).max(33) as u32;
            last_ts = ts;
            durations_ms.push(dur);

            let rgba = frame.data();
            let stride = (w as i32) * 4;
            let mut buf = vec![0u8; (stride * h as i32) as usize];
            for i in 0..(w as usize * h as usize) {
                let r = rgba[i * 4];
                let g = rgba[i * 4 + 1];
                let b = rgba[i * 4 + 2];
                let a = rgba[i * 4 + 3];
                let pr = ((r as u16 * a as u16) / 255) as u8;
                let pg = ((g as u16 * a as u16) / 255) as u8;
                let pb = ((b as u16 * a as u16) / 255) as u8;
                buf[i * 4] = pb;
                buf[i * 4 + 1] = pg;
                buf[i * 4 + 2] = pr;
                buf[i * 4 + 3] = a;
            }
            let surf = ImageSurface::create_for_data(buf, Format::ARgb32, w as i32, h as i32, stride)?;
            frames.push(surf);
        }
        let total_ms = durations_ms.iter().sum::<u32>().max(1);
        Ok(Self {
            frames,
            durations_ms,
            width,
            height,
            total_ms,
            start: Instant::now(),
        })
    }

    /// For a loop animation, sample the current frame index by wall clock modulo
    /// total. For non-loop, the last frame index sticks once the animation ends.
    pub fn current_frame_index(&self, looping: bool) -> usize {
        let elapsed = self.start.elapsed().as_millis() as u32;
        let t = if looping {
            elapsed % self.total_ms
        } else {
            elapsed.min(self.total_ms.saturating_sub(1))
        };
        let mut acc = 0u32;
        for (i, d) in self.durations_ms.iter().enumerate() {
            acc += d;
            if t < acc {
                return i;
            }
        }
        self.frames.len().saturating_sub(1)
    }

    pub fn current_frame(&self, looping: bool) -> &ImageSurface {
        &self.frames[self.current_frame_index(looping)]
    }

    /// Elapsed milliseconds since the animation started; used to tell when a
    /// one-shot transition has played through.
    pub fn elapsed_ms(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }
}
