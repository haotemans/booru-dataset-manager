use eframe::egui::{self, Color32, FontFamily, FontId, Stroke, TextStyle};

pub const BG: Color32 = Color32::WHITE;
pub const PANEL: Color32 = Color32::WHITE;
pub const INK: Color32 = Color32::from_rgb(24, 34, 52);
pub const MUTED: Color32 = Color32::from_rgb(107, 119, 140);
pub const LINE: Color32 = Color32::from_rgb(226, 234, 244);
pub const ACCENT: Color32 = Color32::from_rgb(37, 99, 235);
pub const SOFT: Color32 = Color32::from_rgb(239, 246, 255);
pub const NAV: Color32 = Color32::from_rgb(245, 249, 255);
pub const HOVER_LINE: Color32 = Color32::from_rgb(147, 197, 253);
pub const AMBER: Color32 = Color32::from_rgb(148, 105, 43);
pub const DANGER: Color32 = Color32::from_rgb(220, 68, 68);

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
    // Let selected controls use their white foreground on an accent selection.
    style.visuals.override_text_color = None;
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.code_bg_color = SOFT;
    style.visuals.selection.bg_fill = ACCENT;
    style.visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    style.visuals.widgets.noninteractive.bg_fill = PANEL;
    style.visuals.widgets.noninteractive.weak_bg_fill = PANEL;
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, INK);
    style.visuals.widgets.inactive.bg_fill = PANEL;
    style.visuals.widgets.inactive.weak_bg_fill = PANEL;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, LINE);
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, INK);
    style.visuals.widgets.hovered.bg_fill = NAV;
    style.visuals.widgets.hovered.weak_bg_fill = NAV;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, HOVER_LINE);
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, INK);
    style.visuals.widgets.active.bg_fill = SOFT;
    style.visuals.widgets.active.weak_bg_fill = SOFT;
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.open.bg_fill = SOFT;
    style.visuals.widgets.open.weak_bg_fill = SOFT;
    style.visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, HOVER_LINE);
    style.visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, ACCENT);
    for visuals in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        visuals.rounding = 7.0.into();
        visuals.expansion = 0.0;
    }
    style.visuals.window_rounding = 10.0.into();
    style.visuals.window_stroke = Stroke::new(1.0_f32, LINE);
    style.visuals.window_shadow = egui::epaint::Shadow {
        offset: [0.0, 4.0].into(),
        blur: 14.0,
        spread: 0.0,
        color: Color32::from_black_alpha(14),
    };
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 34.0;
    style.spacing.scroll.foreground_color = true;
    style.spacing.scroll.bar_width = 6.0;
    style.spacing.scroll.floating_width = 2.0;
    style.spacing.scroll.floating_allocated_width = 8.0;
    style.spacing.scroll.dormant_handle_opacity = 1.0;
    style.spacing.scroll.active_handle_opacity = 1.0;
    style.text_styles.insert(TextStyle::Heading, medium(22.0));
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
        .rounding(10.0)
        .inner_margin(20.0)
}
