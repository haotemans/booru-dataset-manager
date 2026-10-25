use super::{pixel, screen};
use bdm_domain::editing::CropRect;
use eframe::egui::{self, pos2, vec2, Rect, Vec2};

pub const RATIOS: [(&str, Option<f32>); 19] = [
    ("自由裁剪", None),
    ("1:1", Some(1.0)),
    ("4:3", Some(4.0 / 3.0)),
    ("3:4", Some(3.0 / 4.0)),
    ("3:2", Some(1.5)),
    ("2:3", Some(2.0 / 3.0)),
    ("16:9", Some(16.0 / 9.0)),
    ("9:16", Some(9.0 / 16.0)),
    ("21:9", Some(21.0 / 9.0)),
    ("9:21", Some(9.0 / 21.0)),
    ("4:5", Some(4.0 / 5.0)),
    ("5:4", Some(5.0 / 4.0)),
    ("2:1", Some(2.0)),
    ("1:2", Some(0.5)),
    ("16:10", Some(16.0 / 10.0)),
    ("10:16", Some(10.0 / 16.0)),
    ("7:5", Some(7.0 / 5.0)),
    ("5:7", Some(5.0 / 7.0)),
    ("原图比例", None),
];

/// Wide-to-tall presets, followed by free and source aspect modes.
pub fn ordered_ratios() -> Vec<usize> {
    let mut indices: Vec<_> = (1..RATIOS.len() - 1).collect();
    indices.sort_by(|&left, &right| RATIOS[right].1.unwrap().total_cmp(&RATIOS[left].1.unwrap()));
    indices.extend([0, RATIOS.len() - 1]);
    indices
}

#[derive(Default)]
pub struct CropState {
    pub selection: Option<Rect>,
    pub ratio: usize,
    drag: Option<(egui::Pos2, Rect, bool)>,
}
impl CropState {
    pub fn ratio_value(&self, size: Vec2) -> Option<f32> {
        if self.ratio == RATIOS.len() - 1 {
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
        self.selection = Some(
            Rect::from_center_size(pos2(size.x / 2.0, size.y / 2.0), dimensions)
                .intersect(Rect::from_min_size(egui::Pos2::ZERO, size)),
        );
        self.drag = None;
        crate::diagnostics::event(format!(
            "crop_centered ratio={} dimensions={size:?} selection={:?}",
            self.ratio, self.selection
        ));
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
                    crate::diagnostics::event(format!(
                        "crop_drag_start ratio={} dimensions={size:?} drag={:?} selection={:?}",
                        self.ratio, self.drag, self.selection
                    ));
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
                crate::diagnostics::event(format!(
                    "crop_drag_stop ratio={} dimensions={size:?} selection={:?}",
                    self.ratio, self.selection
                ));
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
    // A centered, full-height/width crop can exceed the image by a float ULP.
    // Clamping a delta against that rectangle creates inverted min/max limits
    // and f32::clamp panics. Bound the size first, then clamp the new origin to
    // nonnegative travel; this also handles a frame holding an older rectangle.
    let dimensions = r.size().min(size).max(Vec2::ZERO);
    let travel = (size - dimensions).max(Vec2::ZERO);
    let origin = (r.min.to_vec2() + delta).clamp(Vec2::ZERO, travel);
    Rect::from_min_size(origin.to_pos2(), dimensions)
        .intersect(Rect::from_min_size(egui::Pos2::ZERO, size))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn centered_ratios_can_be_moved_at_image_boundaries() {
        for width in (1..5000).step_by(37) {
            for height in (1..5000).step_by(53) {
                let size = vec2(width as f32, height as f32);
                for ratio in 0..RATIOS.len() {
                    let mut crop = CropState {
                        ratio,
                        ..Default::default()
                    };
                    crop.center(size);
                    let rect = crop.selection.unwrap();
                    if ratio == RATIOS.len() - 1 {
                        assert!((rect.size() - size).length() < 0.002);
                    }
                    if let Some(expected) = crop.ratio_value(size) {
                        assert!((rect.width() / rect.height() - expected).abs() < 0.002);
                    }
                    assert!(
                        Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                        "centered crop outside {width}x{height}, ratio {ratio}"
                    );
                    let translated = moved(rect, vec2(1.0, 1.0), size);
                    assert!(translated.is_finite(), "{width}x{height}, ratio {ratio}");
                    assert!(Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(translated));
                }
            }
        }
    }

    #[test]
    fn sixteen_nine_crop_can_be_dragged_after_ratio_selection() {
        let ctx = egui::Context::default();
        let size = vec2(8507.0, 3902.0);
        let mut crop = CropState {
            ratio: 6,
            ..Default::default()
        };
        crop.center(size);
        let selected_size = crop.selection.unwrap().size();
        let image = Rect::from_min_size(pos2(20.0, 20.0), size * (800.0 / size.x));
        let origin = image.center();
        let destination = origin + vec2(24.0, 8.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![],
            vec![],
            vec![egui::Event::PointerMoved(origin), button(origin, true)],
            vec![egui::Event::PointerMoved(destination)],
            vec![button(destination, false)],
        ] {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1050.0, 720.0))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = ui.interact(
                            image,
                            ui.id().with("crop-test"),
                            egui::Sense::click_and_drag(),
                        );
                        crop.canvas(ui, &response, image, size, true);
                    });
                },
            );
            ctx.tessellate(output.shapes, output.pixels_per_point);
        }
        let rect = crop.selection.unwrap();
        assert!(Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect));
        assert!((rect.size() - selected_size).length() < 0.002);
        assert!(rect.center().x > size.x * 0.5);
    }

    #[test]
    fn moving_a_crop_with_roundoff_outside_the_image_does_not_panic() {
        let size = vec2(2800.0, 3800.0);
        let rect = Rect::from_min_max(pos2(133.0, -0.00012207031), pos2(2667.0, 3800.0));
        for delta in [Vec2::ZERO, vec2(1.0, 1.0), vec2(-5000.0, 5000.0)] {
            let next = moved(rect, delta, size);
            assert!(Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(next));
            assert!((next.width() - rect.width()).abs() < 0.001);
        }
    }

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
