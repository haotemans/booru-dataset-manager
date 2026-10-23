use super::{pixel, screen};
use bdm_domain::editing::CropRect;
use eframe::egui::{self, pos2, vec2, Rect, Vec2};

pub const RATIOS: [(&str, Option<f32>); 9] = [
    ("自由裁剪", None),
    ("1:1", Some(1.0)),
    ("4:3", Some(4.0 / 3.0)),
    ("3:4", Some(3.0 / 4.0)),
    ("3:2", Some(1.5)),
    ("2:3", Some(2.0 / 3.0)),
    ("16:9", Some(16.0 / 9.0)),
    ("9:16", Some(9.0 / 16.0)),
    ("原图比例", None),
];

#[derive(Default)]
pub struct CropState {
    pub selection: Option<Rect>,
    pub ratio: usize,
    drag: Option<(egui::Pos2, Rect, bool)>,
}
impl CropState {
    pub fn ratio_value(&self, size: Vec2) -> Option<f32> {
        if self.ratio == 8 {
            Some(size.x / size.y)
        } else {
            RATIOS[self.ratio].1
        }
    }
    pub fn pixels(&self, size: Vec2) -> Option<CropRect> {
        let r = self
            .selection?
            .intersect(Rect::from_min_size(egui::Pos2::ZERO, size));
        let x = r.min.x.round().max(0.0) as u32;
        let y = r.min.y.round().max(0.0) as u32;
        let width = (r.max.x.round().min(size.x) as u32).saturating_sub(x);
        let height = (r.max.y.round().min(size.y) as u32).saturating_sub(y);
        let crop = CropRect {
            x,
            y,
            width,
            height,
        };
        crop.valid((size.x as u32, size.y as u32)).then_some(crop)
    }
    pub fn center(&mut self, size: Vec2) {
        let ratio = self.ratio_value(size).unwrap_or(size.x / size.y);
        let width = size.x.min(size.y * ratio);
        let dimensions = vec2(width, width / ratio);
        self.selection = Some(Rect::from_center_size(
            pos2(size.x / 2.0, size.y / 2.0),
            dimensions,
        ));
        self.drag = None;
    }
    pub fn canvas(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        image: Rect,
        size: Vec2,
        enabled: bool,
    ) {
        if enabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            if response.drag_started_by(egui::PointerButton::Primary) {
                if let Some(p) = ui
                    .input(|i| i.pointer.press_origin())
                    .filter(|p| image.contains(*p))
                {
                    let origin = pixel(p, image, size);
                    let origin = pos2(origin[0], origin[1]);
                    let corner = self.selection.and_then(|r| {
                        corners(r)
                            .into_iter()
                            .enumerate()
                            .find(|(_, c)| screen([c.x, c.y], image, size).distance(p) < 12.0)
                            .map(|(i, _)| corners(r)[(i + 2) % 4])
                    });
                    self.drag = Some(if let Some(anchor) = corner {
                        (anchor, Rect::NOTHING, false)
                    } else if let Some(r) = self.selection.filter(|r| r.contains(origin)) {
                        (origin, r, true)
                    } else {
                        (origin, Rect::NOTHING, false)
                    });
                }
            }
            if response.dragged_by(egui::PointerButton::Primary) {
                if let (Some(p), Some((anchor, original, moving))) =
                    (response.interact_pointer_pos(), self.drag)
                {
                    let p = pixel(p, image, size);
                    let p = pos2(p[0], p[1]);
                    self.selection = Some(if moving {
                        moved(original, p - anchor, size)
                    } else {
                        bounded(anchor, p, size, self.ratio_value(size))
                    });
                }
            }
            if response.drag_stopped_by(egui::PointerButton::Primary) {
                self.drag = None;
            }
        }
        if let Some(r) = self.selection {
            let r = Rect::from_min_max(
                screen([r.min.x, r.min.y], image, size),
                screen([r.max.x, r.max.y], image, size),
            );
            let painter = ui.painter().with_clip_rect(image.intersect(ui.clip_rect()));
            let shade = egui::Color32::from_black_alpha(155);
            for outside in [
                Rect::from_min_max(image.min, pos2(image.max.x, r.min.y)),
                Rect::from_min_max(pos2(image.min.x, r.max.y), image.max),
                Rect::from_min_max(pos2(image.min.x, r.min.y), pos2(r.min.x, r.max.y)),
                Rect::from_min_max(pos2(r.max.x, r.min.y), pos2(image.max.x, r.max.y)),
            ] {
                painter.rect_filled(outside, 0.0, shade);
            }
            let stroke = egui::Stroke::new(1.0_f32, egui::Color32::WHITE);
            painter.rect_stroke(r, 0.0, egui::Stroke::new(2.0_f32, egui::Color32::WHITE));
            for n in [1.0 / 3.0, 2.0 / 3.0] {
                let x = r.min.x + r.width() * n;
                let y = r.min.y + r.height() * n;
                painter.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], stroke);
                painter.line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], stroke);
            }
            for c in corners(r) {
                painter.rect_filled(
                    Rect::from_center_size(c, vec2(9.0, 9.0)),
                    1.0,
                    egui::Color32::WHITE,
                );
            }
        }
    }
}
fn corners(r: Rect) -> [egui::Pos2; 4] {
    [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
    ]
}
fn bounded(anchor: egui::Pos2, pointer: egui::Pos2, size: Vec2, ratio: Option<f32>) -> Rect {
    let delta = pointer - anchor;
    let sign = vec2(
        if delta.x < 0.0 { -1.0 } else { 1.0 },
        if delta.y < 0.0 { -1.0 } else { 1.0 },
    );
    let limit = vec2(
        if sign.x < 0.0 {
            anchor.x
        } else {
            size.x - anchor.x
        },
        if sign.y < 0.0 {
            anchor.y
        } else {
            size.y - anchor.y
        },
    );
    let mut width = delta.x.abs().min(limit.x);
    let mut height = delta.y.abs().min(limit.y);
    if let Some(ratio) = ratio {
        width = width.max(height * ratio).min(limit.x).min(limit.y * ratio);
        height = width / ratio;
    }
    let end = (anchor + vec2(width, height) * sign)
        .to_vec2()
        .clamp(Vec2::ZERO, size);
    Rect::from_two_pos(anchor, end.to_pos2())
}
fn moved(r: Rect, delta: Vec2, size: Vec2) -> Rect {
    r.translate(delta.clamp(-r.min.to_vec2(), size - r.max.to_vec2()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ratios_stay_inside_image_for_all_directions_and_moves_keep_size() {
        let size = vec2(1600.0, 900.0);
        for (_, ratio) in RATIOS {
            for p in [
                pos2(0.0, 0.0),
                pos2(1600.0, 900.0),
                pos2(0.0, 900.0),
                pos2(1600.0, 0.0),
            ] {
                let r = bounded(pos2(1300.0, 200.0), p, size, ratio);
                assert!(Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(r));
                if let Some(ratio) = ratio {
                    assert!((r.width() / r.height() - ratio).abs() < 0.001);
                }
                let next = moved(r, vec2(5000.0, -5000.0), size);
                assert!((r.size() - next.size()).length() < 0.001);
                assert!(Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(next));
            }
        }
    }
}
