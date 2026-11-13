//! Small vector icons drawn at the current DPI; no icon fonts or image files.
use eframe::egui::{self, Color32, Pos2, Rect, Response, Stroke, Vec2, Widget};
use crate::hover_hints::HoverHint;
#[derive(Clone, Copy)]
pub enum Icon {
    Play,
    Pause,
    Previous,
    Next,
    Stop,
    Star,
    Folder,
    Search,
    Server,
    Plus,
    Refresh,
    RepeatAll,
    RepeatOne,
    Trash,
    Import,
    Export,
    Settings,
    Gear,
    Back,
    Meter,
    Equalizer,
    Volume,
    Mute,
    Position,
    Library,
    Save,
    Up,
    More,
    Fullscreen,
    History,
    Music,
}

pub struct IconButton<'a> {
    icon: Icon,
    label: &'a str,
    size: Vec2,
    fill: Option<Color32>,
    color: Option<Color32>,
    selected: Option<bool>,
    left_aligned: bool,
}
impl<'a> IconButton<'a> {
    pub fn new(icon: Icon, label: &'a str) -> Self {
        Self {
            icon,
            label,
            size: Vec2::ZERO,
            fill: None,
            color: None,
            selected: None,
            left_aligned: false,
        }
    }
    pub fn min_size(mut self, size: Vec2) -> Self {
        self.size = size;
        self
    }
    pub fn fill(mut self, color: Color32) -> Self {
        self.fill = Some(color);
        self
    }
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }
    pub fn left_aligned(mut self) -> Self {
        self.left_aligned = true;
        self
    }
}
impl Widget for IconButton<'_> {
    fn ui(self, ui: &mut egui::Ui) -> Response {
        let font = egui::TextStyle::Button.resolve(ui.style());
        let galley = ui
            .painter()
            .layout_no_wrap(self.label.to_owned(), font, Color32::WHITE);
        let gap = if self.label.is_empty() { 0.0 } else { 6.0 };
        let content = Vec2::new(16.0 + gap + galley.size().x, galley.size().y.max(16.0));
        let size = (content + ui.spacing().button_padding * 2.0).max(self.size);
        let mut button = egui::Button::new("").min_size(size);
        if let Some(selected) = self.selected {
            button = button.selected(selected);
        }
        if let Some(fill) = self.fill {
            button = button.fill(fill);
        }
        let response = ui.add(button);
        let mut color = self.color.unwrap_or(if self.selected == Some(true) {
            crate::theme::ACCENT
        } else {
            ui.style().interact(&response).fg_stroke.color
        });
        if !ui.is_enabled() {
            color = color.gamma_multiply(0.4);
        }
        let left = if self.left_aligned {
            response.rect.left() + ui.spacing().button_padding.x
        } else {
            response.rect.center().x - content.x * 0.5
        };
        draw(
            ui.painter(),
            self.icon,
            Rect::from_center_size(
                Pos2::new(left + 8.0, response.rect.center().y),
                Vec2::splat(16.0),
            ),
            color,
        );
        if !self.label.is_empty() {
            ui.painter().galley_with_override_text_color(
                Pos2::new(
                    left + 16.0 + gap,
                    response.rect.center().y - galley.size().y * 0.5,
                ),
                galley,
                color,
            );
        }
        response.widget_info(|| {
            let label = if self.label.is_empty() { self.icon.name() } else { self.label };
            match self.selected {
                Some(selected) => egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, label),
                None => egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label),
            }
        });
        response
    }
}
impl Icon {
    fn name(self) -> &'static str {
        match self {
            Self::Play => "Play",
            Self::Pause => "Pause",
            Self::Previous => "Previous",
            Self::Next => "Next",
            Self::Stop => "Stop",
            Self::Star => "Favorite",
            Self::Folder => "Folder",
            Self::Search => "Search",
            Self::Server => "Server",
            Self::Plus => "Add",
            Self::Refresh => "Refresh",
            Self::RepeatAll => "Repeat all",
            Self::RepeatOne => "Repeat one",
            Self::Trash => "Remove",
            Self::Import => "Import",
            Self::Export => "Export",
            Self::Settings => "Settings",
            Self::Gear => "Settings",
            Self::Back => "Back",
            Self::Meter => "Audio Meters",
            Self::Equalizer => "Equalizer",
            Self::Volume => "Volume",
            Self::Mute => "Mute",
            Self::Position => "Playback position",
            Self::Library => "Library",
            Self::Save => "Save",
            Self::Up => "Parent folder",
            Self::More => "More actions",
            Self::Fullscreen => "Toggle fullscreen",
            Self::History => "Recent Tracks",
            Self::Music => "Music",
        }
    }
}
pub fn button(ui: &mut egui::Ui, icon: Icon, label: &str) -> Response {
    ui.add(IconButton::new(icon, label))
}
pub fn menu_item(ui: &mut egui::Ui, icon: Icon, label: &str) -> Response {
    ui.add(IconButton::new(icon, label).left_aligned())
}
pub fn menu_choice<T: PartialEq>(
    ui: &mut egui::Ui,
    current: &mut T,
    choice: T,
    icon: Icon,
    label: &str,
) {
    if ui.add(IconButton::new(icon, label).left_aligned().selected(*current == choice)).clicked() {
        *current = choice;
    }
}
pub fn inline(ui: &mut egui::Ui, icon: Icon, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
    draw(ui.painter(), icon, rect, color);
}
/// Gear outline in the 24x24 grid `draw` uses: eight teeth, a circular
/// root between teeth, and a central hole drawn separately. Tooth tops
/// sit at radius 10.5, the root at 8.0, with a tooth centred at the top.
fn gear_outline_points() -> Vec<(f32, f32)> {
    let mut points = Vec::new();
    for tooth in 0..8 {
        let center = tooth as f32 * 45.0;
        for (radius, angle) in [
            (8.0, center - 22.5),
            (8.0, center - 18.0),
            (8.0, center - 14.0),
            (8.0, center - 11.0),
            (10.5, center - 11.0),
            (10.5, center),
            (10.5, center + 11.0),
            (8.0, center + 14.0),
            (8.0, center + 18.0),
        ] {
            let angle = (angle - 90.0).to_radians();
            points.push((12.0 + radius * angle.cos(), 12.0 + radius * angle.sin()));
        }
    }
    points
}
/// Back arrow in the 24x24 grid `draw` uses: a steep chevron pointing
/// left, plus a horizontal shaft, matching the provided arrow image.
fn back_arrow_lines() -> [Vec<(f32, f32)>; 2] {
    [
        vec![(10., 3.), (4., 12.), (10., 21.)],
        vec![(5., 12.), (21., 12.)],
    ]
}
pub fn draw(p: &egui::Painter, icon: Icon, rect: Rect, color: Color32) {
    let pos = |x: f32, y: f32| {
        Pos2::new(
            rect.left() + x * rect.width() / 24.0,
            rect.top() + y * rect.height() / 24.0,
        )
    };
    let stroke = Stroke::new(1.7, color);
    let line = |points: &[(f32, f32)]| {
        p.add(egui::Shape::line(
            points.iter().map(|&(x, y)| pos(x, y)).collect(),
            stroke,
        ));
    };
    let triangle = |points: &[(f32, f32)]| {
        p.add(egui::Shape::convex_polygon(
            points.iter().map(|&(x, y)| pos(x, y)).collect(),
            color,
            Stroke::NONE,
        ));
    };
    let box_outline = |x: f32, y: f32, w: f32, h: f32| {
        p.rect_stroke(
            Rect::from_min_max(pos(x, y), pos(x + w, y + h)),
            2.0,
            stroke,
            egui::StrokeKind::Inside,
        );
    };
    match icon {
        Icon::Music => {
            line(&[(8., 17.), (8., 5.), (20., 2.), (20., 14.)]);
            line(&[(8., 9.), (20., 6.)]);
            p.circle_filled(pos(5., 18.), rect.width() * 3.0 / 24.0, color);
            p.circle_filled(pos(17., 15.), rect.width() * 3.0 / 24.0, color);
        }
        Icon::Position => {
            line(&[(3., 12.), (9., 12.)]);
            line(&[(15., 12.), (21., 12.)]);
            p.circle_stroke(pos(12., 12.), rect.width() * 3. / 24., stroke);
        }
        Icon::Play => triangle(&[(7., 4.), (20., 12.), (7., 20.)]),
        Icon::Pause => {
            for x in [6., 14.] {
                p.rect_filled(Rect::from_min_max(pos(x, 4.), pos(x + 4., 20.)), 0.0, color);
            }
        }
        Icon::Previous => {
            line(&[(5., 5.), (5., 19.)]);
            triangle(&[(19., 5.), (7., 12.), (19., 19.)]);
        }
        Icon::Next => {
            triangle(&[(5., 5.), (17., 12.), (5., 19.)]);
            line(&[(19., 5.), (19., 19.)]);
        }
        Icon::Stop => {
            p.rect_filled(Rect::from_min_max(pos(5., 5.), pos(19., 19.)), 2.0, color);
        }
        Icon::Star => {
            let points = (0..10)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::PI / 5.0 - std::f32::consts::FRAC_PI_2;
                    let r = if i % 2 == 0 { 10.0 } else { 4.5 };
                    pos(12.0 + r * a.cos(), 12.0 + r * a.sin())
                })
                .collect();
            p.add(egui::Shape::closed_line(points, stroke));
        }
        Icon::Folder => line(&[
            (3., 7.),
            (3., 19.),
            (21., 19.),
            (21., 7.),
            (12., 7.),
            (10., 4.),
            (3., 4.),
            (3., 7.),
        ]),
        Icon::Search => {
            p.circle_stroke(pos(10., 10.), rect.width() * 0.28, stroke);
            line(&[(15., 15.), (21., 21.)]);
        }
        Icon::Server => {
            box_outline(3., 3., 18., 7.);
            box_outline(3., 14., 18., 7.);
            p.circle_filled(pos(7., 6.5), 1.2, color);
            p.circle_filled(pos(7., 17.5), 1.2, color);
            line(&[(13., 6.5), (18., 6.5)]);
            line(&[(13., 17.5), (18., 17.5)]);
        }
        Icon::Plus => {
            line(&[(12., 4.), (12., 20.)]);
            line(&[(4., 12.), (20., 12.)]);
        }
        Icon::RepeatAll | Icon::RepeatOne => {
            // Opposing arrows with rounded return bends, matching standard repeat controls.
            let mut top = vec![pos(3., 11.)];
            let mut bottom = vec![pos(21., 13.)];
            for i in 0..=12 {
                let angle = std::f32::consts::PI + i as f32 / 12.0 * std::f32::consts::FRAC_PI_2;
                top.push(pos(9. + 6. * angle.cos(), 11. + 6. * angle.sin()));
                let angle = i as f32 / 12.0 * std::f32::consts::FRAC_PI_2;
                bottom.push(pos(15. + 6. * angle.cos(), 13. + 6. * angle.sin()));
            }
            top.push(pos(21., 5.));
            bottom.push(pos(3., 19.));
            p.add(egui::Shape::line(top, stroke));
            p.add(egui::Shape::line(bottom, stroke));
            line(&[(17., 1.), (21., 5.), (17., 9.)]);
            line(&[(7., 15.), (3., 19.), (7., 23.)]);
            if matches!(icon, Icon::RepeatOne) {
                line(&[(10., 10.), (12., 8.5), (12., 15.5)]);
            }
        }
        Icon::Refresh => {
            let points = (0..25)
                .map(|i| {
                    let a = 0.4 + i as f32 / 24.0 * 5.3;
                    pos(12. + 8. * a.cos(), 12. + 8. * a.sin())
                })
                .collect();
            p.add(egui::Shape::line(points, stroke));
            line(&[(15., 3.), (20., 5.), (20., 10.)]);
        }
        Icon::Trash => {
            line(&[(4., 6.), (20., 6.)]);
            line(&[(9., 6.), (9., 3.), (15., 3.), (15., 6.)]);
            line(&[(6., 6.), (7., 21.), (17., 21.), (18., 6.)]);
            line(&[(10., 10.), (10., 17.)]);
            line(&[(14., 10.), (14., 17.)]);
        }
        Icon::Import | Icon::Export => {
            line(&[(4., 14.), (4., 20.), (20., 20.), (20., 14.)]);
            if matches!(icon, Icon::Import) {
                line(&[(12., 3.), (12., 15.)]);
                line(&[(7., 10.), (12., 15.), (17., 10.)]);
            } else {
                line(&[(12., 15.), (12., 3.)]);
                line(&[(7., 8.), (12., 3.), (17., 8.)]);
            }
        }
        Icon::History => {
            p.circle_stroke(pos(12., 12.), rect.width() * 0.36, stroke);
            line(&[(12., 6.), (12., 12.), (16., 14.)]);
        }
        Icon::Settings => {
            for (y, x) in [(5., 8.), (12., 16.), (19., 10.)] {
                line(&[(3., y), (21., y)]);
                p.circle_filled(pos(x, y), 3.0, crate::theme::SURFACE);
                p.circle_stroke(pos(x, y), 3.0, stroke);
            }
        }
        Icon::Gear => {
            let points = gear_outline_points()
                .into_iter()
                .map(|(x, y)| pos(x, y))
                .collect();
            p.add(egui::Shape::closed_line(points, stroke));
            p.circle_stroke(pos(12., 12.), rect.width() * 0.16, stroke);
        }
        Icon::Back => {
            // Bold, like the provided arrow, not the other icons' 1.7 stroke.
            let thick = Stroke::new(rect.width() * 0.15, color);
            for points in back_arrow_lines() {
                let points = points.into_iter().map(|(x, y)| pos(x, y)).collect();
                p.add(egui::Shape::line(points, thick));
            }
        }
        Icon::Meter => {
            for (x, top) in [(4., 14.), (9., 8.), (14., 4.), (19., 11.)] {
                line(&[(x, 20.), (x, top)]);
            }
        }
        Icon::Equalizer => {
            for (x, y) in [(5., 9.), (12., 15.), (19., 7.)] {
                line(&[(x, 3.), (x, y - 2.)]);
                line(&[(x, y + 2.), (x, 21.)]);
                line(&[(x - 2., y), (x + 2., y)]);
            }
        }
        Icon::Volume => {
            line(&[
                (3., 9.),
                (7., 9.),
                (12., 5.),
                (12., 19.),
                (7., 15.),
                (3., 15.),
                (3., 9.),
            ]);
            line(&[(16., 8.), (18., 10.), (18., 14.), (16., 16.)]);
            line(&[(19., 5.), (22., 9.), (22., 15.), (19., 19.)]);
        }
        Icon::Mute => {
            line(&[
                (3., 9.),
                (7., 9.),
                (12., 5.),
                (12., 19.),
                (7., 15.),
                (3., 15.),
                (3., 9.),
            ]);
            line(&[(15., 5.), (22., 19.)]);
        }
        Icon::Library => {
            box_outline(3., 4., 4., 16.);
            box_outline(10., 4., 4., 16.);
            line(&[(17., 4.), (21., 19.)]);
        }
        Icon::Save => {
            line(&[
                (3., 3.),
                (17., 3.),
                (21., 7.),
                (21., 21.),
                (3., 21.),
                (3., 3.),
            ]);
            box_outline(7., 3., 9., 6.);
            box_outline(7., 14., 10., 7.);
        }
        Icon::Up => {
            line(&[(12., 21.), (12., 3.)]);
            line(&[(5., 10.), (12., 3.), (19., 10.)]);
        }
        Icon::Fullscreen => {
            line(&[(3., 9.), (3., 3.), (9., 3.)]);
            line(&[(15., 3.), (21., 3.), (21., 9.)]);
            line(&[(21., 15.), (21., 21.), (15., 21.)]);
            line(&[(9., 21.), (3., 21.), (3., 15.)]);
        }
        Icon::More => {
            for x in [5., 12., 19.] {
                p.circle_filled(pos(x, 12.), 1.5, color);
            }
        }
    }
}

/// Animated pill switch with checkbox semantics for keyboard and accessibility.
pub fn slide_toggle(ui: &mut egui::Ui, enabled: &mut bool, label: &str) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(36., 16.), egui::Sense::click());
    if response.clicked() {
        *enabled = !*enabled;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *enabled, label)
    });
    let t = ui.ctx().animate_bool(response.id, *enabled);
    let off = Color32::from_rgb(64, 64, 64);
    let on = Color32::from_rgb(0, 122, 255);
    let track = egui::lerp(egui::Rgba::from(off)..=egui::Rgba::from(on), t);
    ui.painter().rect_filled(rect, 8., Color32::from(track));
    let center = egui::pos2(
        egui::lerp((rect.left() + 11.)..=(rect.right() - 11.), t),
        rect.center().y,
    );
    let thumb = egui::Rect::from_center_size(center, Vec2::new(20., 14.));
    ui.painter().rect_filled(thumb, 7., Color32::from_rgb(230, 230, 230));
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.),
            10.,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Outside,
        );
    }
    response.on_hover_hint(if *enabled {
        label.replacen("Show ", "Hide ", 1)
    } else {
        label.to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gear_outline_has_eight_teeth_between_root_and_tip() {
        let points = gear_outline_points();
        assert_eq!(points.len(), 72);
        let mut tips = 0;
        for (x, y) in &points {
            let distance = ((x - 12.0).powi(2) + (y - 12.0).powi(2)).sqrt();
            assert!(distance >= 8.0 - 0.001 && distance <= 10.5 + 0.001);
            if distance > 10.0 {
                tips += 1;
            }
        }
        assert_eq!(tips, 24);
    }
    #[test]
    fn back_arrow_chevron_points_left_with_horizontal_shaft() {
        let [chevron, shaft] = back_arrow_lines();
        assert_eq!(chevron, vec![(10., 3.), (4., 12.), (10., 21.)]);
        assert!(chevron[1].0 < chevron[0].0 && chevron[1].0 < chevron[2].0);
        assert_eq!((chevron[0].1 + chevron[2].1) / 2.0, 12.0);
        assert_eq!(shaft, vec![(5., 12.), (21., 12.)]);
    }
}
