//! Minimal text rendering: ab_glyph coverage blended into a tiny-skia pixmap.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use tiny_skia::Pixmap;

pub struct Text {
    font: FontVec,
}

impl Text {
    pub fn load() -> anyhow::Result<Self> {
        // Linux DejaVu/Noto/Liberation; SF on macOS; Helvetica Neue / Arial as fallbacks.
        for (path, idx) in [
            ("/usr/share/fonts/dejavu/DejaVuSans.ttf", 0),
            ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0),
            ("/usr/share/fonts/noto/NotoSans-Regular.ttf", 0),
            ("/usr/share/fonts/liberation/LiberationSans-Regular.ttf", 0),
            ("/System/Library/Fonts/SFNS.ttf", 0),
            ("/System/Library/Fonts/HelveticaNeue.ttc", 0),
            ("/System/Library/Fonts/Supplemental/Arial.ttf", 0),
        ] {
            if let Ok(data) = std::fs::read(path) {
                if let Ok(font) = FontVec::try_from_vec_and_index(data, idx) {
                    return Ok(Text { font });
                }
            }
        }
        anyhow::bail!("no system font with Cyrillic found")
    }

    pub fn width(&self, s: &str, size: f32) -> f32 {
        let f = self.font.as_scaled(PxScale::from(size));
        s.chars().map(|c| f.glyph_id(c)).filter(|id| id.0 != 0).map(|id| f.h_advance(id)).sum()
    }

    /// Draws `s` with its baseline at `y`. Returns the x after the last glyph.
    pub fn draw(&self, pm: &mut Pixmap, s: &str, x: f32, y: f32, size: f32, rgba: (u8, u8, u8, u8)) -> f32 {
        let scale = PxScale::from(size);
        let f = self.font.as_scaled(scale);
        let (w, h) = (pm.width() as i32, pm.height() as i32);
        let data = pm.data_mut();
        let mut pen = x;
        for c in s.chars() {
            let id = f.glyph_id(c);
            if id.0 == 0 {
                continue; // no glyph (emoji variation selectors etc.)
            }
            let glyph = id.with_scale_and_position(scale, ab_glyph::point(pen, y));
            pen += f.h_advance(id);
            let Some(outline) = self.font.outline_glyph(glyph) else { continue };
            let b = outline.px_bounds();
            outline.draw(|gx, gy, cov| {
                let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if px < 0 || py < 0 || px >= w || py >= h {
                    return;
                }
                let a = cov.clamp(0.0, 1.0) * rgba.3 as f32 / 255.0;
                let i = ((py * w + px) * 4) as usize;
                // Premultiplied source-over.
                for (k, c) in [rgba.0, rgba.1, rgba.2].into_iter().enumerate() {
                    data[i + k] = (c as f32 * a + data[i + k] as f32 * (1.0 - a)).round() as u8;
                }
                data[i + 3] = (255.0 * a + data[i + 3] as f32 * (1.0 - a)).round() as u8;
            });
        }
        pen
    }
}
