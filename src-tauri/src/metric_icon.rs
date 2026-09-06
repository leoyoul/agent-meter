use ab_glyph::{point, Font, FontVec, ScaleFont};
use std::sync::OnceLock;
use tauri::image::Image;

const WIDTH: usize = 76;
const HEIGHT: usize = 44;

fn load_font(path: &str, collection_index: u32) -> Option<FontVec> {
    let bytes = std::fs::read(path).ok()?;
    FontVec::try_from_vec_and_index(bytes, collection_index).ok()
}

fn label_font() -> Option<&'static FontVec> {
    static FONT: OnceLock<Option<FontVec>> = OnceLock::new();
    FONT.get_or_init(|| load_font("/System/Library/Fonts/Hiragino Sans GB.ttc", 0))
        .as_ref()
}

fn value_font() -> Option<&'static FontVec> {
    static FONT: OnceLock<Option<FontVec>> = OnceLock::new();
    FONT.get_or_init(|| load_font("/System/Library/Fonts/SFNSMono.ttf", 0))
        .as_ref()
}

fn draw_centered(alpha: &mut [u8], font: &FontVec, text: &str, size: f32, top: f32) {
    let scaled = font.as_scaled(size);
    let width = text
        .chars()
        .map(|ch| scaled.h_advance(scaled.glyph_id(ch)))
        .sum::<f32>();
    let mut x = (WIDTH as f32 - width).max(0.0) / 2.0;
    let baseline = top + scaled.ascent();
    for ch in text.chars() {
        let id = scaled.glyph_id(ch);
        let glyph = id.with_scale_and_position(size, point(x, baseline));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|column, row, coverage| {
                let px = bounds.min.x.floor() as i32 + column as i32;
                let py = bounds.min.y.floor() as i32 + row as i32;
                if px >= 0 && py >= 0 && px < WIDTH as i32 && py < HEIGHT as i32 {
                    let index = py as usize * WIDTH + px as usize;
                    alpha[index] = alpha[index].max((coverage * 255.0).round() as u8);
                }
            });
        }
        x += scaled.h_advance(id);
    }
}

pub fn render(label: &str, value: &str) -> Image<'static> {
    let mut alpha = vec![0_u8; WIDTH * HEIGHT];
    if let Some(font) = label_font() {
        draw_centered(&mut alpha, font, label, 13.0, 0.0);
    }
    if let Some(font) = value_font() {
        draw_centered(&mut alpha, font, value, 17.0, 20.0);
    }
    let mut rgba = Vec::with_capacity(WIDTH * HEIGHT * 4);
    for opacity in alpha {
        rgba.extend_from_slice(&[255, 255, 255, opacity]);
    }
    Image::new_owned(rgba, WIDTH as u32, HEIGHT as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_icons_are_fixed_size_and_nonblank() {
        for (label, value) in [
            ("速", "16.4"),
            ("首", "680ms"),
            ("量", "1.2M"),
            ("费", "$1.24"),
        ] {
            let image = render(label, value);
            assert_eq!(image.width(), WIDTH as u32);
            assert_eq!(image.height(), HEIGHT as u32);
            assert!(image
                .rgba()
                .iter()
                .skip(3)
                .step_by(4)
                .any(|alpha| *alpha > 0));
        }
    }
}
