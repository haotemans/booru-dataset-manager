use eframe::egui::{self, Color32, FontFamily, FontId, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(255, 255, 255);
pub const PANEL: Color32 = Color32::WHITE;
pub const INK: Color32 = Color32::from_rgb(31, 33, 38);
pub const MUTED: Color32 = Color32::from_rgb(100, 105, 114);
pub const LINE: Color32 = Color32::from_rgb(229, 230, 234);
pub const ACCENT: Color32 = Color32::from_rgb(48, 65, 91);
pub const SOFT: Color32 = Color32::from_rgb(238, 241, 246);
pub const NAV: Color32 = Color32::from_rgb(247, 247, 249);
pub const AMBER: Color32 = Color32::from_rgb(148, 105, 43);
pub const DANGER: Color32 = Color32::from_rgb(184, 58, 58);

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}

fn load_font(fonts: &mut egui::FontDefinitions, name: &str, path: &str, index: u32) -> bool {
    if let Ok(data) = std::fs::read(path) {
        let mut font = egui::FontData::from_owned(data);
        font.index = index;
        fonts.font_data.insert(name.into(), font);
        true
    } else {
        false
    }
}

pub fn setup(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Light);
    let mut fonts = egui::FontDefinitions::default();
    let mut regular = Vec::new();
    let mut emphasis = Vec::new();
    // Latin/numerals use Segoe UI; face 1 in the Windows CJK collection is YaHei UI.
    if load_font(&mut fonts, "latin", "C:/Windows/Fonts/segoeui.ttf", 0) {
        regular.push("latin".into());
    }
    if load_font(
        &mut fonts,
        "latin-medium",
        "C:/Windows/Fonts/seguisb.ttf",
        0,
    ) {
        emphasis.push("latin-medium".into());
    }
    for (path, index) in [
        ("C:/Windows/Fonts/msyh.ttc", 1),
        ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
        ("/System/Library/Fonts/PingFang.ttc", 0),
    ] {
        if load_font(&mut fonts, "cjk", path, index) {
            regular.push("cjk".into());
            break;
        }
    }
    if load_font(&mut fonts, "cjk-medium", "C:/Windows/Fonts/msyhbd.ttc", 1) {
        emphasis.push("cjk-medium".into());
    }
    regular.extend(fonts.families[&FontFamily::Proportional].clone());
    emphasis.extend(regular.clone());
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts
        .families
        .insert(FontFamily::Name("medium".into()), emphasis);
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = PANEL;
    style.visuals.window_fill = PANEL;
    style.visuals.extreme_bg_color = PANEL;
    style.visuals.faint_bg_color = BG;
    style.visuals.override_text_color = Some(INK);
    style.visuals.selection.bg_fill = SOFT;
    style.visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, INK);
    style.visuals.widgets.inactive.bg_fill = PANEL;
    style.visuals.widgets.inactive.weak_bg_fill = PANEL;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, LINE);
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, INK);
    style.visuals.widgets.hovered.bg_fill = NAV;
    style.visuals.widgets.hovered.weak_bg_fill = NAV;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_gray(185));
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, INK);
    style.visuals.widgets.active.bg_fill = SOFT;
    style.visuals.widgets.active.weak_bg_fill = SOFT;
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, INK);
    for visuals in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        visuals.rounding = 6.0.into();
        visuals.expansion = 0.0;
    }
    style.visuals.window_rounding = 10.0.into();
    style.visuals.window_stroke = Stroke::new(1.0_f32, LINE);
    style.visuals.window_shadow = egui::epaint::Shadow::NONE;
    style.spacing.item_spacing = egui::vec2(10.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.spacing.interact_size.y = 34.0;
    style.text_styles.insert(TextStyle::Heading, medium(24.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(13.0));
    ctx.set_style(style);
}

pub fn card() -> egui::Frame {
    egui::Frame::none()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, LINE))
        .rounding(8.0)
        .inner_margin(18.0)
}
