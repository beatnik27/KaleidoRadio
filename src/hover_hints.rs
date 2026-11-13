use eframe::egui::{self, Color32, Context, Id, Response, WidgetText};

pub fn set_enabled(ctx: &Context, enabled: bool) {
    ctx.data_mut(|data| data.insert_temp(Id::new("show_hover_hints"), enabled));
}

pub trait HoverHint {
    fn on_hover_hint(self, text: impl Into<WidgetText>) -> Self;
    fn on_shortcut_hint(self, label: &str, keys: &[&str]) -> Self;
}

fn enabled(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(Id::new("show_hover_hints")).unwrap_or(true))
}

impl HoverHint for Response {
    fn on_hover_hint(self, text: impl Into<WidgetText>) -> Self {
        if enabled(&self.ctx) {
            self.on_hover_text(text)
        } else {
            self
        }
    }

    fn on_shortcut_hint(self, label: &str, keys: &[&str]) -> Self {
        if !enabled(&self.ctx) {
            return self;
        }
        self.on_hover_ui(|ui| {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.add_space(2.0);
                ui.spacing_mut().item_spacing.x = 4.0;
                for key in keys {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::Vec2::splat(24.0), egui::Sense::hover());
                    ui.painter().rect_filled(
                        rect.translate(egui::vec2(0.0, 1.0)),
                        5,
                        Color32::from_gray(132),
                    );
                    ui.painter().rect_filled(rect, 5, Color32::from_gray(224));
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        *key,
                        egui::FontId::proportional(15.0),
                        Color32::from_gray(27),
                    );
                }
            });
        })
    }
}
