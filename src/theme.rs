use eframe::egui::{self, Color32, FontId, TextStyle};
pub const MEDIA_CORNER_RADIUS: f32 = 14.0;
pub const ACCENT: Color32 = Color32::from_rgb(165, 139, 250);
pub const MUTED: Color32 = Color32::from_rgb(154, 160, 180);
pub const SURFACE: Color32 = Color32::from_rgb(27, 30, 43);
pub fn apply(ctx: &egui::Context) {
    // egui bundles Hack, but only its monospace family uses it by default.
    // Use it as a fallback for symbols such as the shortcut arrows.
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .push("Hack".to_owned());
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(12.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(11.0));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(21.0));
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(8.0, 5.0);
    style.spacing.interact_size = egui::vec2(28.0, 28.0);
    style.spacing.slider_width = 160.0;
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = Color32::from_rgb(18, 20, 30);
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = Color32::from_rgb(13, 15, 24);
    style.visuals.faint_bg_color = Color32::from_rgb(24, 27, 39);
    style.visuals.override_text_color = Some(Color32::from_rgb(233, 235, 245));
    style.visuals.selection.bg_fill = Color32::from_rgb(68, 53, 107);
    style.visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    style.visuals.hyperlink_color = ACCENT;
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        widget.corner_radius = egui::CornerRadius::same(8);
    }
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(38, 41, 57);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(54, 49, 77);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(70, 58, 103);
    ctx.set_style_of(egui::Theme::Dark, style);
    ctx.set_theme(egui::Theme::Dark);
}

pub const DEFAULT_ARTWORK_COLOR: Color32 = Color32::from_rgb(41, 48, 61);

pub fn artwork_color(image: &image::RgbaImage) -> Color32 {
    let mut sum = [0.0; 3];
    let mut weight = 0.0;
    for pixel in image.pixels() {
        let alpha = pixel[3] as f64 / 255.0;
        weight += alpha;
        for channel in 0..3 {
            sum[channel] += pixel[channel] as f64 * alpha;
        }
    }
    if weight == 0.0 {
        return DEFAULT_ARTWORK_COLOR;
    }
    let boost = [1.1, 1.05, 1.05];
    let rgb = std::array::from_fn::<_, 3, _>(|i| {
        (sum[i] / weight * boost[i]).round().clamp(0.0, 255.0) as u8
    });
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

pub fn darker(color: Color32, amount: f32) -> Color32 {
    let channel = |c: u8| (c as f32 * (1.0 - amount)).round() as u8;
    Color32::from_rgb(channel(color.r()), channel(color.g()), channel(color.b()))
}

pub fn background(ui: &egui::Ui, color: Color32, sidebar: bool, margin: f32) {
    let rect = ui.max_rect().expand(margin);
    let shades = if sidebar {
        [0.46, 0.56, 0.72]
    } else {
        [0.58, 0.32, 0.18]
    };
    let mut mesh = egui::Mesh::default();
    // A diagonal gradient with the same three shade stops as the macOS app.
    const STEPS: u32 = 16;
    for y in 0..=STEPS {
        for x in 0..=STEPS {
            let t = (x + y) as f32 / (2 * STEPS) as f32;
            let (a, b, fraction) = if t < 0.5 {
                (shades[0], shades[1], t * 2.0)
            } else {
                (shades[1], shades[2], (t - 0.5) * 2.0)
            };
            mesh.colored_vertex(
                egui::pos2(
                    rect.left() + rect.width() * x as f32 / STEPS as f32,
                    rect.top() + rect.height() * y as f32 / STEPS as f32,
                ),
                darker(color, a + (b - a) * fraction),
            );
        }
    }
    for y in 0..STEPS {
        for x in 0..STEPS {
            let i = y * (STEPS + 1) + x;
            mesh.add_triangle(i, i + 1, i + STEPS + 1);
            mesh.add_triangle(i + 1, i + STEPS + 2, i + STEPS + 1);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proportional_font_renders_shortcut_and_link_symbols() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let font = TextStyle::Body.resolve(ui.style());
            ui.fonts_mut(|fonts| {
                let glyph_position = |fonts: &mut egui::epaint::text::FontsView<'_>, symbol: char| {
                    let galley = fonts.layout_no_wrap(symbol.to_string(), font.clone(), Color32::WHITE);
                    galley.rows[0].glyphs[0].uv_rect.min
                };
                let missing = glyph_position(fonts, '\u{10ffff}');
                for symbol in ['⌘', '←', '→', '↗'] {
                    assert_ne!(glyph_position(fonts, symbol), missing, "Missing glyph: {symbol}");
                }
            });
        });
        output.textures_delta.clear();
    }

    #[test]
    fn samples_and_boosts_average_artwork_color() {
        let image = image::RgbaImage::from_fn(2, 1, |x, _| {
            if x == 0 {
                image::Rgba([200, 0, 0, 255])
            } else {
                image::Rgba([0, 100, 200, 255])
            }
        });
        assert_eq!(artwork_color(&image), Color32::from_rgb(110, 53, 105));
        assert_eq!(darker(Color32::WHITE, 1.0), Color32::BLACK);
    }

    #[test]
    fn transparent_pixels_do_not_tint_background() {
        let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 0]));
        assert_eq!(artwork_color(&image), DEFAULT_ARTWORK_COLOR);
    }
}
