use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};
use std::sync::OnceLock;
use tauri::image::Image;

const WIDTH: usize = 60;
const HEIGHT: usize = 44;

fn load_font(path: &str, collection_index: u32) -> Option<FontVec> {
    let bytes = std::fs::read(path).ok()?;
    FontVec::try_from_vec_and_index(bytes, collection_index).ok()
}

fn pingfang_font() -> Option<&'static FontVec> {
    static FONT: OnceLock<Option<FontVec>> = OnceLock::new();
    FONT.get_or_init(|| {
        let mut paths = vec![
            "/System/Library/Fonts/PingFang.ttc".into(),
            "/System/Library/Fonts/LanguageSupport/PingFang.ttc".into(),
        ];
        if let Ok(entries) =
            std::fs::read_dir("/System/Library/AssetsV2/com_apple_MobileAsset_Font8")
        {
            paths.extend(
                entries
                    .flatten()
                    .map(|entry| entry.path().join("AssetData/PingFang.ttc")),
            );
        }
        paths
            .iter()
            .find_map(|path| load_font(&path.to_string_lossy(), 11))
    })
    .as_ref()
}

fn text_width(font: &FontVec, text: &str, size: f32) -> f32 {
    let scaled = font.as_scaled(size);
    text.chars()
        .map(|ch| scaled.h_advance(scaled.glyph_id(ch)))
        .sum::<f32>()
}

fn fitted_size(font: &FontVec, text: &str, preferred: f32, minimum: f32) -> f32 {
    let width = text_width(font, text, preferred);
    if width <= (WIDTH - 2) as f32 {
        preferred
    } else {
        (preferred * (WIDTH - 2) as f32 / width).max(minimum)
    }
}

fn draw_centered(
    alpha: &mut [u8],
    font: &FontVec,
    text: &str,
    height: f32,
    width: f32,
    top: f32,
    semibold: bool,
) {
    let scale = PxScale {
        x: width,
        y: height,
    };
    let scaled = font.as_scaled(scale);
    let text_width = text_width(font, text, width);
    let mut x = (WIDTH as f32 - text_width).max(0.0) / 2.0;
    let baseline = top + scaled.ascent();
    for ch in text.chars() {
        let id = scaled.glyph_id(ch);
        let glyph = id.with_scale_and_position(scale, point(x, baseline));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|column, row, coverage| {
                let px = bounds.min.x.floor() as i32 + column as i32;
                let py = bounds.min.y.floor() as i32 + row as i32;
                if px >= 0 && py >= 0 && px < WIDTH as i32 && py < HEIGHT as i32 {
                    let index = py as usize * WIDTH + px as usize;
                    let opacity = (coverage * 255.0).round() as u8;
                    alpha[index] = alpha[index].max(opacity);
                    if semibold && px + 1 < WIDTH as i32 {
                        alpha[index + 1] = alpha[index + 1].max(opacity.saturating_sub(24));
                    }
                }
            });
        }
        x += scaled.h_advance(id);
    }
}

pub fn render(label: &str, value: &str) -> Image<'static> {
    let mut alpha = vec![0_u8; WIDTH * HEIGHT];
    if let Some(font) = pingfang_font() {
        let width = fitted_size(font, value, 46.0, 20.0);
        draw_centered(&mut alpha, font, value, 46.0, width, -7.5, true);
    }
    if let Some(font) = pingfang_font() {
        let width = fitted_size(font, label, 18.0, 13.0);
        draw_centered(&mut alpha, font, label, 18.0, width, 29.5, true);
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
            ("TPS", "16.4"),
            ("TTFT", "680ms"),
            ("TOK", "1.2M"),
            ("USD", "$1.24"),
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

    #[test]
    fn value_and_label_use_separate_rows() {
        let image = render("TOK", "123.4M");
        let alpha = image
            .rgba()
            .iter()
            .skip(3)
            .step_by(4)
            .copied()
            .collect::<Vec<_>>();
        assert!(alpha[..WIDTH * 29].iter().any(|value| *value > 0));
        assert!(alpha[WIDTH * 30..].iter().any(|value| *value > 0));
    }

    #[test]
    fn common_values_keep_large_type() {
        let font = pingfang_font().expect("PingFang SC Semibold font");
        for (_, value) in [
            ("TPS", "21.6"),
            ("TTFT", "18s"),
            ("TOK", "150M"),
            ("USD", "$116"),
        ] {
            let width = fitted_size(font, value, 46.0, 20.0);
            assert!(
                text_width(font, value, width) <= (WIDTH - 2) as f32,
                "{value}"
            );
        }
    }

    #[test]
    fn letter_labels_fit_the_bottom_row() {
        let font = pingfang_font().expect("PingFang SC Semibold font");
        for label in ["TPS", "TTFT", "TOK", "USD"] {
            let width = fitted_size(font, label, 18.0, 13.0);
            assert!(
                text_width(font, label, width) <= (WIDTH - 2) as f32,
                "{label}"
            );
        }
    }
}
