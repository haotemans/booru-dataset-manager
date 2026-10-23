use crate::{app::BdmApp, theme::*};
use bdm_domain::editing::MaskRegion;
use bdm_infrastructure::editing::{self, RepairPreview};
use eframe::egui::{self, vec2, Rect, RichText};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
    Arc,
};
mod crop;
pub use crop::CropState;

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub enum Tool {
    #[default]
    Pan,
    Rectangle,
    Brush,
    Polygon,
}

enum ResultEvent {
    Installed(Result<(), String>),
    Preview(Result<RepairPreview, String>),
    Prepared(RepairPreview, Result<egui::ColorImage, String>),
    Saved(Box<SaveResult>),
}
struct SaveResult {
    id: usize,
    preview: RepairPreview,
    media: Result<bdm_domain::ingest::StoredMedia, String>,
}

fn preview_pixels(preview: &RepairPreview) -> Result<egui::ColorImage, String> {
    let rgba = image::open(&preview.image)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}
pub struct EditorState {
    pub visible: bool,
    pub cropping: bool,
    pub crop: CropState,
    pub tool: Tool,
    pub regions: Vec<MaskRegion>,
    pub draft: Vec<[f32; 2]>,
    pub radius: f32,
    pub model: usize,
    pub preview: Option<RepairPreview>,
    pub texture: Option<egui::TextureHandle>,
    pub show_preview: bool,
    pub error: String,
    media_id: Option<usize>,
    pending: Option<Receiver<ResultEvent>>,
    cancel: Option<Arc<AtomicBool>>,
    auto_commit: bool,
    saving: bool,
    #[cfg(test)]
    pub open_button: Option<Rect>,
    #[cfg(test)]
    pub editor_button: Option<Rect>,
}
impl Default for EditorState {
    fn default() -> Self {
        Self {
            visible: false,
            cropping: false,
            crop: CropState::default(),
            tool: Tool::Pan,
            regions: Vec::new(),
            draft: Vec::new(),
            radius: 20.0,
            model: 0,
            preview: None,
            texture: None,
            show_preview: true,
            error: String::new(),
            media_id: None,
            pending: None,
            cancel: None,
            auto_commit: false,
            saving: false,
            #[cfg(test)]
            open_button: None,
            #[cfg(test)]
            editor_button: None,
        }
    }
}
impl Drop for EditorState {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl EditorState {
    pub fn discard(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        // Keep receiving an in-flight save after navigation so committed metadata cannot go stale.
        if !self.saving {
            self.cancel = None;
            self.pending = None;
        }
        self.auto_commit = false;
        self.preview = None;
        self.texture = None;
        self.regions.clear();
        self.draft.clear();
        self.media_id = None;
        self.crop.selection = None;
    }
    pub fn reset(&mut self, id: usize) {
        if self.media_id == Some(id) {
            return;
        }
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        if !self.saving {
            self.cancel = None;
            self.pending = None;
        }
        self.auto_commit = false;
        self.preview = None;
        self.texture = None;
        self.regions.clear();
        self.draft.clear();
        self.error.clear();
        self.media_id = Some(id);
        self.tool = Tool::Pan;
        self.crop.selection = None;
    }
    pub fn active(&self) -> bool {
        self.pending.is_some()
    }
    fn model_name(&self) -> &'static str {
        if self.model == 0 {
            "anime-lama"
        } else {
            "lama"
        }
    }
}

pub(crate) fn pixel(point: egui::Pos2, image: Rect, size: egui::Vec2) -> [f32; 2] {
    let p = ((point - image.min) / image.size()).clamp(egui::Vec2::ZERO, vec2(1.0, 1.0)) * size;
    [p.x, p.y]
}
fn screen(point: [f32; 2], image: Rect, size: egui::Vec2) -> egui::Pos2 {
    image.min + vec2(point[0], point[1]) / size * image.size()
}

impl BdmApp {
    pub fn open_original(&mut self, id: usize) -> Result<(), String> {
        if self.demo || self.is_capture() {
            return Err("示例图片没有本地原图文件。".into());
        }
        let path = self
            .ingest
            .stored
            .get(&id)
            .ok_or("找不到当前图片记录。")?
            .path
            .canonicalize()
            .map_err(|_| "原图文件缺失。")?;
        if !path.is_file() {
            return Err("原图文件缺失。".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let output = std::process::Command::new("powershell.exe")
                .creation_flags(0x08000000)
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "$ErrorActionPreference='Stop'; Start-Process -FilePath $env:BDM_OPEN_IMAGE",
                ])
                .env("BDM_OPEN_IMAGE", &path)
                .output()
                .map_err(|_| "无法启动图片查看器。")?;
            if !output.status.success() {
                return Err("系统没有可用的图片打开方式，请设置默认图片查看器。".into());
            }
        }
        #[cfg(not(windows))]
        {
            std::process::Command::new(if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            })
            .arg(path)
            .spawn()
            .map_err(|_| "无法启动图片查看器。")?;
        }
        Ok(())
    }

    pub fn editor_panel(&mut self, ui: &mut egui::Ui, id: usize, height: f32) {
        let model = self.image_editor.model_name();
        let root = crate::paths::project_root();
        let busy = self.image_editor.active();
        let real = !self.demo && !self.is_capture();
        let cropping = self.image_editor.cropping;
        let dimensions = self
            .ingest
            .stored
            .get(&id)
            .map(|m| m.dimensions())
            .map(|d| vec2(d.0 as f32, d.1 as f32))
            .or_else(|| {
                self.library
                    .media
                    .iter()
                    .find(|m| m.id == id)
                    .map(|m| vec2(m.width as f32, m.height as f32))
            })
            .unwrap_or(vec2(1.0, 1.0));
        egui::ScrollArea::vertical()
            .id_salt("inpainting-tools")
            .max_height(height)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(if cropping {
                        "裁剪图片"
                    } else {
                        "局部修复"
                    })
                    .font(medium(16.0)),
                );
                ui.add_enabled_ui(!busy && self.image_editor.preview.is_none(), |ui| {
                    ui.checkbox(&mut self.prefs.confirm_image_edits, "编辑后二次确认")
                        .on_hover_text("裁剪和去水印共用；关闭后点击执行按钮即处理并替换当前图片。设置自动记住。删除确认不受影响。");
                });
                ui.label(
                    RichText::new(if !self.prefs.confirm_image_edits && self.image_editor.preview.is_none() {
                        "直接保存 · 处理成功后替换当前图片"
                    } else if cropping {
                        "拖动选区 → 预览裁剪 → 确认保存"
                    } else {
                        "圈选区域 → 生成预览 → 确认替换"
                    })
                    .size(12.0)
                    .color(MUTED),
                );
                ui.add_space(8.0);
                if cropping && self.image_editor.preview.is_none() {
                    ui.add_enabled_ui(!busy, |ui| {
                        crate::review_ui::section().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new("裁剪比例").size(12.0).color(MUTED));
                            let old = self.image_editor.crop.ratio;
                            egui::ComboBox::from_id_salt("crop-ratio")
                                .width(ui.available_width() - 24.0)
                                .selected_text(crop::RATIOS[old].0)
                                .show_ui(ui, |ui| {
                                    for (index, (name, _)) in crop::RATIOS.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut self.image_editor.crop.ratio,
                                            index,
                                            *name,
                                        );
                                    }
                                });
                            if old != self.image_editor.crop.ratio
                                && self.image_editor.crop.selection.is_some()
                            {
                                self.image_editor.crop.center(dimensions);
                            }
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                let width = (ui.available_width() - 8.0) / 2.0;
                                ui.spacing_mut().item_spacing.x = 8.0;
                                if ui
                                    .add_sized(
                                        [width, 30.0],
                                        egui::Button::new(RichText::new("居中选区").size(12.0)),
                                    )
                                    .clicked()
                                {
                                    self.image_editor.crop.center(dimensions);
                                }
                                if ui
                                    .add_sized(
                                        [width, 30.0],
                                        egui::Button::new(RichText::new("重新框选").size(12.0)),
                                    )
                                    .clicked()
                                {
                                    self.image_editor.crop.selection = None;
                                }
                            });
                        });
                        ui.add_space(10.0);
                        if let Some(c) = self.image_editor.crop.pixels(dimensions) {
                            crate::review_ui::section().show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new("输出尺寸").size(12.0).color(MUTED));
                                ui.label(
                                    RichText::new(format!("{} × {} px", c.width, c.height))
                                        .font(medium(20.0)),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "起点 {}, {} · 原图 {} × {}",
                                        c.x, c.y, dimensions.x as u32, dimensions.y as u32
                                    ))
                                    .size(12.0)
                                    .color(MUTED),
                                );
                            });
                        } else {
                            ui.label(
                                RichText::new("在左侧图片上拖动，创建裁剪框。")
                                    .size(12.0)
                                    .color(MUTED),
                            );
                        }
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("框内移动 · 四角调整\n右键平移 · 滚轮缩放")
                                .size(11.0)
                                .color(MUTED),
                        );
                    });
                } else if self.image_editor.preview.is_none() {
                    ui.add_enabled_ui(!busy, |ui| {
                        crate::review_ui::section().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new("修复模型").size(12.0).color(MUTED));
                            egui::ComboBox::from_id_salt("repair-model")
                                .width(ui.available_width() - 24.0)
                                .selected_text(if self.image_editor.model == 0 {
                                    "Anime LaMa · 二次元"
                                } else {
                                    "LaMa · 通用"
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.image_editor.model,
                                        0,
                                        "Anime LaMa · 二次元",
                                    );
                                    ui.selectable_value(
                                        &mut self.image_editor.model,
                                        1,
                                        "LaMa · 通用",
                                    );
                                });
                            ui.add_space(8.0);
                            ui.label(RichText::new("选区工具").size(12.0).color(MUTED));
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                let width = (ui.available_width() - 12.0) / 4.0;
                                for (tool, name) in [
                                    (Tool::Pan, "平移"),
                                    (Tool::Rectangle, "矩形"),
                                    (Tool::Brush, "画笔"),
                                    (Tool::Polygon, "多边形"),
                                ] {
                                    if crate::review_ui::choice(
                                        ui,
                                        name,
                                        self.image_editor.tool == tool,
                                        width,
                                    )
                                    .clicked()
                                    {
                                        self.image_editor.tool = tool;
                                        self.image_editor.draft.clear();
                                    }
                                }
                            });
                            if self.image_editor.tool == Tool::Brush {
                                ui.add(
                                    egui::Slider::new(&mut self.image_editor.radius, 2.0..=150.0)
                                        .text("画笔半径 px"),
                                );
                            }
                            if self.image_editor.tool == Tool::Polygon {
                                ui.label(
                                    RichText::new("点击添加顶点，完成后闭合选区。")
                                        .size(12.0)
                                        .color(MUTED),
                                );
                                if ui
                                    .add_enabled(
                                        self.image_editor.draft.len() >= 3,
                                        egui::Button::new("完成多边形"),
                                    )
                                    .clicked()
                                {
                                    self.image_editor.regions.push(MaskRegion::Polygon {
                                        points: std::mem::take(&mut self.image_editor.draft),
                                    });
                                }
                            }
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                if ui.button("撤销选区").clicked() {
                                    self.image_editor.regions.pop();
                                    self.image_editor.draft.clear();
                                }
                                if ui.button("清空选区").clicked() {
                                    self.image_editor.regions.clear();
                                    self.image_editor.draft.clear();
                                }
                            });
                        });
                    });
                }
                if !cropping {
                    ui.label(
                        RichText::new(format!(
                            "{} 个选区 · 右键拖动平移",
                            self.image_editor.regions.len()
                        ))
                        .size(12.0)
                        .color(MUTED),
                    );
                }
                ui.add_space(10.0);
                if busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(if self.image_editor.saving { "正在保存…" } else { "正在处理…" });
                    });
                    if ui.button("取消任务").clicked() {
                        if let Some(cancel) = &self.image_editor.cancel {
                            cancel.store(true, Ordering::Relaxed);
                        }
                    }
                } else if self.image_editor.preview.is_some() {
                    if let Some(crop) = self.image_editor.preview.as_ref().and_then(|p| p.crop) {
                        ui.label(
                            RichText::new(format!(
                                "裁剪预览 · {} × {} px",
                                crop.width, crop.height
                            ))
                            .font(medium(14.0)),
                        );
                    }
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.image_editor.show_preview, false, "当前图片");
                        ui.selectable_value(
                            &mut self.image_editor.show_preview,
                            true,
                            if cropping {
                                "裁剪预览"
                            } else {
                                "修复预览"
                            },
                        );
                    });
                    ui.label(
                        RichText::new(if cropping {
                            "确认后替换当前图片，不保留裁剪前副本；取消保留当前文件。"
                        } else {
                            "检查预览后确认替换；取消保留当前文件。"
                        })
                        .size(12.0)
                        .color(AMBER),
                    );
                    ui.add_space(8.0);
                    if crate::review_ui::action(
                        ui,
                        "确认替换当前图片",
                        real && !self.ingest_active() && !self.workflow_active(),
                        true,
                    )
                    .clicked()
                    {
                        if let Err(error) = self.editor_commit(ui.ctx(),id) { self.image_editor.error = error; }
                    }
                    if crate::review_ui::action(ui, "取消预览", true, false).clicked() {
                        self.image_editor.preview = None;
                        self.image_editor.texture = None;
                    }
                } else {
                    if !cropping && !editing::model_ready(&root, model) {
                        ui.label(
                            RichText::new("所选模型尚未安装。下载权重后在本机运行，图片不会上传。")
                                .size(12.0)
                                .color(MUTED),
                        );
                        if ui
                            .add_enabled(
                                real && editing::runtime(&root).is_file(),
                                egui::Button::new("安装所选模型"),
                            )
                            .clicked()
                        {
                            self.editor_start(ui.ctx(), id, true);
                        }
                        if !editing::runtime(&root).is_file() {
                            ui.label(
                                RichText::new(
                                    "请先运行 scripts/setup_analysis.ps1 安装本地运行环境。",
                                )
                                .size(12.0)
                                .color(AMBER),
                            );
                        }
                    }
                    if crate::review_ui::action(
                        ui,
                        if !self.prefs.confirm_image_edits {
                            if cropping { "裁剪并保存" } else { "去水印并保存" }
                        } else if cropping {
                            "预览裁剪"
                        } else {
                            "生成修复预览"
                        },
                        real && if cropping {
                            self.image_editor.crop.pixels(dimensions).is_some()
                        } else {
                            editing::model_ready(&root, model)
                                && !self.image_editor.regions.is_empty()
                        } && !self.ingest_active()
                            && !self.workflow_active(),
                        true,
                    )
                    .clicked()
                    {
                        self.editor_start(ui.ctx(), id, false);
                    }
                }
                if !self.image_editor.error.is_empty() {
                    ui.label(
                        RichText::new(&self.image_editor.error)
                            .size(12.0)
                            .color(DANGER),
                    );
                }
            });
    }

    fn editor_start(&mut self, ctx: &egui::Context, id: usize, install: bool) {
        if self.demo
            || self.is_capture()
            || self.image_editor.active()
            || self.image_editor.preview.is_some()
            || self.ingest_active()
            || self.workflow_active()
        {
            return;
        }
        self.image_editor.reset(id);
        let root = crate::paths::project_root();
        let model = self.image_editor.model_name();
        let media = self.ingest.stored.get(&id).cloned();
        let regions = self.image_editor.regions.clone();
        let cropping = self.image_editor.cropping;
        let crop = media.as_ref().and_then(|m| {
            let d = m.dimensions();
            self.image_editor.crop.pixels(vec2(d.0 as f32, d.1 as f32))
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let thread_cancel = cancel.clone();
        let repaint = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.image_editor.pending = Some(receiver);
        self.image_editor.cancel = Some(cancel);
        self.image_editor.auto_commit = !install && !self.prefs.confirm_image_edits;
        let auto_commit = self.image_editor.auto_commit;
        self.image_editor.error.clear();
        std::thread::spawn(move || {
            let result = if install {
                ResultEvent::Installed(
                    editing::install(&root, model, &thread_cancel).map_err(|e| e.to_string()),
                )
            } else {
                ResultEvent::Preview(media.ok_or("原图不可用。".into()).and_then(|media| {
                    if cropping {
                        crop.ok_or("请先框选裁剪区域。".into()).and_then(|crop| {
                            editing::crop_preview(&media, crop, &thread_cancel)
                                .map_err(|e| e.to_string())
                        })
                    } else {
                        editing::preview(&root, &media, model, regions, &thread_cancel)
                            .map_err(|e| e.to_string())
                    }
                }))
            };
            let result = match result {
                ResultEvent::Preview(Ok(preview)) if !auto_commit => {
                    let pixels = preview_pixels(&preview);
                    ResultEvent::Prepared(preview, pixels)
                }
                event => event,
            };
            let _ = sender.send(result);
            repaint.request_repaint();
        });
    }
    pub(crate) fn editor_poll(&mut self, ctx: &egui::Context) {
        let event = match self.image_editor.pending.as_ref().map(|r| r.try_recv()) {
            Some(Ok(event)) => Some(event),
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.image_editor.pending = None;
                self.image_editor.cancel = None;
                self.image_editor.auto_commit = false;
                self.image_editor.saving = false;
                self.image_editor.error = "修复任务已中断。".into();
                None
            }
            _ => None,
        };
        if let Some(event) = event {
            let auto_commit = std::mem::take(&mut self.image_editor.auto_commit);
            let cancelled = self
                .image_editor
                .cancel
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Relaxed));
            self.image_editor.pending = None;
            self.image_editor.cancel = None;
            if cancelled && !matches!(event, ResultEvent::Saved(_)) {
                self.image_editor.error = "任务已取消。".into();
                return;
            }
            match event {
                ResultEvent::Installed(result) => {
                    if let Err(error) = result {
                        self.image_editor.error = error
                    }
                }
                ResultEvent::Preview(result) => match result {
                    Ok(preview) => {
                        self.image_editor.preview = Some(preview);
                        if auto_commit {
                            if let Some(id) = self.image_editor.media_id {
                                if let Err(error) = self.editor_commit(ctx, id) {
                                    self.image_editor.error = error;
                                }
                            }
                        } else {
                            self.editor_prepare_preview(ctx);
                        }
                    }
                    Err(error) => self.image_editor.error = error,
                },
                ResultEvent::Prepared(preview, pixels) => {
                    self.image_editor.preview = Some(preview);
                    self.image_editor.show_preview = true;
                    match pixels {
                        Ok(pixels) => {
                            self.image_editor.texture = Some(ctx.load_texture(
                                "inpainting-preview",
                                pixels,
                                egui::TextureOptions::LINEAR,
                            ));
                            if preview_is_crop(&self.image_editor) {
                                self.media_review.view = crate::media_review::CanvasView::default();
                            }
                        }
                        Err(error) => self.image_editor.error = error,
                    }
                }
                ResultEvent::Saved(result) => {
                    self.image_editor.saving = false;
                    let SaveResult { id, preview, media } = *result;
                    match media {
                        Ok(media) => {
                            self.editor_apply_saved(ctx, media);
                        }
                        Err(error) if !cancelled && Some(id) == self.image_editor.media_id => {
                            self.image_editor.error = error;
                            self.image_editor.preview = Some(preview);
                            if self.image_editor.texture.is_none() {
                                self.editor_prepare_preview(ctx);
                            }
                        }
                        Err(error) => {
                            if Some(id) == self.image_editor.media_id {
                                self.image_editor.error = error;
                                self.image_editor.texture = None;
                                self.image_editor.show_preview = false;
                            }
                        }
                    }
                }
            }
        }
        if self.image_editor.active() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn editor_prepare_preview(&mut self, ctx: &egui::Context) {
        let Some(preview) = self.image_editor.preview.take() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        self.image_editor.pending = Some(receiver);
        self.image_editor.cancel = Some(Arc::new(AtomicBool::new(false)));
        std::thread::spawn(move || {
            let pixels = preview_pixels(&preview);
            let _ = sender.send(ResultEvent::Prepared(preview, pixels));
            repaint.request_repaint();
        });
    }

    fn editor_commit(&mut self, ctx: &egui::Context, id: usize) -> Result<(), String> {
        if self.demo
            || self.is_capture()
            || self.ingest_active()
            || self.workflow_active()
            || self.image_editor.active()
            || self.image_editor.media_id != Some(id)
        {
            return Err("当前无法保存编辑，请等待其他任务结束后重试。".into());
        }
        let path = self.catalog.path.clone().ok_or("本地数据库未打开。")?;
        let preview = self
            .image_editor
            .preview
            .take()
            .ok_or("没有可保存的编辑结果。")?;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        self.image_editor.pending = Some(receiver);
        self.image_editor.cancel = Some(cancel);
        self.image_editor.saving = true;
        std::thread::spawn(move || {
            let media = bdm_infrastructure::catalog::SqliteCatalog::open(&path)
                .map_err(|e| e.to_string())
                .and_then(|mut repo| {
                    repo.commit_repair_cancellable(id as i64, &preview, &worker_cancel)
                        .map_err(|e| e.to_string())
                });
            let _ = sender.send(ResultEvent::Saved(Box::new(SaveResult {
                id,
                preview,
                media,
            })));
            repaint.request_repaint();
        });
        Ok(())
    }

    fn editor_apply_saved(
        &mut self,
        ctx: &egui::Context,
        mut media: bdm_domain::ingest::StoredMedia,
    ) {
        let id = media.id as usize;
        let cropping = media.edit.as_ref().is_some_and(|e| e.operation == "crop");
        let dimensions = media.dimensions();
        if let Some(item) = self.library.media.iter_mut().find(|m| m.id == id) {
            item.width = dimensions.0 as usize;
            item.height = dimensions.1 as usize;
            media.keep = item.keep;
        }
        self.ingest.stored.insert(id, media);
        self.invalidate_review_texture(id);
        if self.image_editor.media_id == Some(id) {
            self.image_editor.preview = None;
            self.image_editor.texture = None;
            self.image_editor.regions.clear();
            self.image_editor.draft.clear();
            self.image_editor.crop.selection = None;
            self.media_review.view = crate::media_review::CanvasView::default();
        }
        self.image_editor.error.clear();
        self.workflows
            .candidates
            .retain(|pair| pair.first != id as i64 && pair.second != id as i64);
        self.notify(
            ctx,
            if cropping {
                "裁剪结果已保存。"
            } else {
                "修复结果已保存。"
            },
        );
    }

    pub fn editor_canvas(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        image: Rect,
        size: egui::Vec2,
    ) {
        if !self.image_editor.visible || self.image_editor.saving {
            return;
        }
        if self.image_editor.cropping {
            if self.image_editor.preview.is_none() {
                let enabled = !self.image_editor.active();
                self.image_editor
                    .crop
                    .canvas(ui, response, image, size, enabled);
            }
            return;
        }
        let editing = !self.image_editor.active()
            && self.image_editor.preview.is_none()
            && self.image_editor.tool != Tool::Pan;
        if editing {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            if response.drag_started_by(egui::PointerButton::Primary)
                && self.image_editor.tool != Tool::Polygon
            {
                if let Some(p) = ui
                    .input(|i| i.pointer.press_origin())
                    .filter(|p| image.contains(*p))
                {
                    self.image_editor.draft = vec![pixel(p, image, size)];
                }
            }
            if response.dragged_by(egui::PointerButton::Primary)
                && self.image_editor.tool != Tool::Polygon
                && !self.image_editor.draft.is_empty()
            {
                if let Some(p) = response.interact_pointer_pos() {
                    let p = pixel(p, image, size);
                    if self.image_editor.tool == Tool::Rectangle {
                        self.image_editor.draft.truncate(1);
                    }
                    self.image_editor.draft.push(p);
                }
            }
            if response.drag_stopped_by(egui::PointerButton::Primary)
                && !self.image_editor.draft.is_empty()
            {
                let points = std::mem::take(&mut self.image_editor.draft);
                match self.image_editor.tool {
                    Tool::Rectangle if points.len() > 1 => {
                        self.image_editor.regions.push(MaskRegion::Rectangle {
                            start: points[0],
                            end: *points.last().unwrap(),
                        })
                    }
                    Tool::Brush => self.image_editor.regions.push(MaskRegion::Brush {
                        points,
                        radius: self.image_editor.radius,
                    }),
                    _ => self.image_editor.draft = points,
                }
            }
            if self.image_editor.tool == Tool::Polygon
                && response.clicked_by(egui::PointerButton::Primary)
            {
                if let Some(p) = response
                    .interact_pointer_pos()
                    .filter(|p| image.contains(*p))
                {
                    self.image_editor.draft.push(pixel(p, image, size));
                }
            }
            if self.image_editor.tool == Tool::Brush
                && response.clicked_by(egui::PointerButton::Primary)
            {
                if let Some(p) = response
                    .interact_pointer_pos()
                    .filter(|p| image.contains(*p))
                {
                    self.image_editor.regions.push(MaskRegion::Brush {
                        points: vec![pixel(p, image, size)],
                        radius: self.image_editor.radius,
                    });
                }
            }
        }
        if self.image_editor.preview.is_some() {
            return;
        }
        let painter = ui.painter().with_clip_rect(image.intersect(ui.clip_rect()));
        let color = egui::Color32::from_rgba_unmultiplied(240, 80, 80, 65);
        let mut regions = self.image_editor.regions.clone();
        if !self.image_editor.draft.is_empty() {
            match self.image_editor.tool {
                Tool::Rectangle => regions.push(MaskRegion::Rectangle {
                    start: self.image_editor.draft[0],
                    end: *self.image_editor.draft.last().unwrap(),
                }),
                Tool::Brush => regions.push(MaskRegion::Brush {
                    points: self.image_editor.draft.clone(),
                    radius: self.image_editor.radius,
                }),
                Tool::Polygon => {
                    let points: Vec<_> = self
                        .image_editor
                        .draft
                        .iter()
                        .map(|p| screen(*p, image, size))
                        .collect();
                    painter.add(egui::Shape::line(
                        points,
                        egui::Stroke::new(2.0_f32, DANGER),
                    ));
                }
                _ => {}
            }
        }
        for region in regions {
            match region {
                MaskRegion::Rectangle { start, end } => {
                    painter.rect_filled(
                        Rect::from_two_pos(screen(start, image, size), screen(end, image, size)),
                        0.0,
                        color,
                    );
                }
                MaskRegion::Brush { points, radius } => {
                    let points: Vec<_> =
                        points.into_iter().map(|p| screen(p, image, size)).collect();
                    let radius = radius * image.width() / size.x;
                    for p in &points {
                        painter.circle_filled(*p, radius, color);
                    }
                    if points.len() > 1 {
                        painter.add(egui::Shape::line(
                            points,
                            egui::Stroke::new(radius * 2.0, color),
                        ));
                    }
                }
                MaskRegion::Polygon { points } => {
                    let points: Vec<_> =
                        points.into_iter().map(|p| screen(p, image, size)).collect();
                    painter.add(egui::Shape::closed_line(
                        points,
                        egui::Stroke::new(2.0_f32, DANGER),
                    ));
                }
            }
        }
    }
}

fn preview_is_crop(editor: &EditorState) -> bool {
    editor.preview.as_ref().is_some_and(|p| p.crop.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdm_ports::{CollectionRepository, IngestRepository, MediaProvider};

    fn imported_app() -> (tempfile::TempDir, BdmApp, bdm_domain::ingest::StoredMedia) {
        imported_image((800, 800), "png")
    }
    fn imported_image(
        dimensions: (u32, u32),
        extension: &str,
    ) -> (tempfile::TempDir, BdmApp, bdm_domain::ingest::StoredMedia) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog.sqlite3");
        let mut repo = bdm_infrastructure::catalog::SqliteCatalog::open(&path).unwrap();
        let collection = repo
            .add(
                bdm_domain::catalog::CollectionKind::Character,
                &bdm_domain::catalog::SourceTag {
                    id: 1,
                    name: "character".into(),
                    category: 4,
                    post_count: 1,
                },
            )
            .unwrap()
            .record
            .id;
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(
            dimensions.0,
            dimensions.1,
            |x, y| image::Rgb([(x / 4) as u8, (y / 4) as u8, 80]),
        ))
        .write_to(
            &mut bytes,
            if extension == "jpg" {
                image::ImageFormat::Jpeg
            } else {
                image::ImageFormat::Png
            },
        )
        .unwrap();
        let bytes = bytes.into_inner();
        use md5::Digest;
        let post = bdm_domain::ingest::SourcePost {
            id: 10,
            md5: format!("{:x}", md5::Md5::digest(&bytes)),
            file_url: Some("https://synthetic/image.png".into()),
            file_ext: extension.into(),
            file_size: bytes.len() as u64,
            image_width: dimensions.0,
            image_height: dimensions.1,
            tag_string: "character scenery".into(),
            tag_string_artist: String::new(),
            tag_string_character: "character".into(),
            tag_string_general: "scenery".into(),
            tag_string_copyright: String::new(),
            tag_string_meta: String::new(),
            rating: "g".into(),
            is_deleted: false,
        };
        struct Provider(Vec<u8>);
        impl MediaProvider for Provider {
            fn posts(
                &self,
                _: &str,
                _: Option<i64>,
            ) -> Result<Vec<bdm_domain::ingest::SourcePost>, bdm_ports::IngestError> {
                Ok(Vec::new())
            }
            fn download(
                &self,
                _: &bdm_domain::ingest::SourcePost,
                path: &std::path::Path,
                _: &AtomicBool,
            ) -> Result<(), bdm_ports::IngestError> {
                std::fs::write(path, &self.0).map_err(|_| bdm_ports::IngestError::Storage)
            }
        }
        repo.import(
            &post,
            &directory.path().join("library"),
            collection,
            800,
            &AtomicBool::new(false),
            &Provider(bytes),
        )
        .unwrap();
        let media = repo.list_media().unwrap().remove(0);
        let mut app = BdmApp::from_preferences(crate::model::Preferences::default(), false, None);
        app.catalog.path = Some(path);
        app.reload_media().unwrap();
        app.open_media(media.id as usize);
        app.image_editor.reset(media.id as usize);
        app.image_editor.visible = true;
        (directory, app, media)
    }
    fn finish(app: &mut BdmApp, ctx: &egui::Context) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.image_editor.active() && std::time::Instant::now() < deadline {
            app.editor_poll(ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!app.image_editor.active());
    }
    fn repair_result(media: &bdm_domain::ingest::StoredMedia) -> RepairPreview {
        let (width, height) = media.dimensions();
        let mut result = editing::crop_preview(
            media,
            bdm_domain::editing::CropRect {
                x: 0,
                y: 0,
                width,
                height,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        result.crop = None;
        result.model = "synthetic-inpainting-test".into();
        result.mask = result.directory.path().join("mask.png");
        image::GrayImage::from_pixel(width, height, image::Luma([255]))
            .save(&result.mask)
            .unwrap();
        let mut pixels = image::open(&result.image).unwrap().to_rgba8();
        pixels.put_pixel(50, 50, image::Rgba([255, 0, 0, 255]));
        pixels.save(&result.image).unwrap();
        result
    }
    fn deliver(app: &mut BdmApp, result: RepairPreview, auto_commit: bool, cancelled: bool) {
        let (sender, receiver) = mpsc::channel();
        app.image_editor.pending = Some(receiver);
        app.image_editor.auto_commit = auto_commit;
        app.image_editor.cancel = Some(Arc::new(AtomicBool::new(cancelled)));
        sender.send(ResultEvent::Preview(Ok(result))).unwrap();
    }

    #[test]
    fn crop_and_inpainting_honor_confirmation_mode_and_snapshot_it_when_started() {
        let (_directory, mut app, media) = imported_app();
        let ctx = egui::Context::default();
        let id = media.id as usize;
        let source = std::fs::read(&media.path).unwrap();
        for confirmation in [true, false] {
            app.image_editor.cropping = true;
            app.prefs.confirm_image_edits = confirmation;
            app.image_editor.crop.selection = Some(Rect::from_min_size(
                egui::pos2(10.0, 20.0),
                vec2(600.0, 400.0),
            ));
            app.editor_start(&ctx, id, false);
            assert_eq!(app.image_editor.auto_commit, !confirmation);
            app.prefs.confirm_image_edits = !confirmation; // Running tasks use the chosen mode at click time.
            finish(&mut app, &ctx);
            assert!(
                app.image_editor.error.is_empty(),
                "{}",
                app.image_editor.error
            );
            if confirmation {
                assert_eq!(std::fs::read(&media.path).unwrap(), source);
                assert!(app.image_editor.preview.is_some());
                app.image_editor.preview = None;
                app.image_editor.texture = None;
            } else {
                assert!(app.image_editor.preview.is_none());
                assert_eq!(app.ingest.stored[&id].dimensions(), (600, 400));
                assert_eq!(
                    app.library
                        .media
                        .iter()
                        .find(|m| m.id == id)
                        .map(|m| (m.width, m.height)),
                    Some((600, 400))
                );
            }
        }
        let cropped = app.ingest.stored[&id].clone();
        let before = std::fs::read(&media.path).unwrap();
        app.image_editor.cropping = false;
        app.image_editor.tool = Tool::Brush;
        deliver(&mut app, repair_result(&cropped), false, false);
        finish(&mut app, &ctx);
        assert_eq!(std::fs::read(&media.path).unwrap(), before);
        assert!(app.image_editor.preview.is_some());
        app.image_editor.preview = None;
        app.image_editor.texture = None;
        deliver(&mut app, repair_result(&cropped), true, false);
        finish(&mut app, &ctx);
        assert!(app.image_editor.preview.is_none());
        assert_ne!(std::fs::read(&media.path).unwrap(), before);
        assert_eq!(app.image_editor.tool, Tool::Brush);
        assert_eq!(app.ingest.stored[&id].post, media.post);
        assert_eq!(
            app.ingest.stored[&id].edit.as_ref().unwrap().source_sha256,
            media.sha256
        );
    }

    #[test]
    fn direct_save_cancel_stale_result_and_image_switch_preserve_current_file() {
        let (_directory, mut app, media) = imported_app();
        let ctx = egui::Context::default();
        let original = std::fs::read(&media.path).unwrap();
        let preview = repair_result(&media);
        let temp = preview.directory.path().to_owned();
        deliver(&mut app, preview, true, true);
        app.editor_poll(&ctx);
        assert!(!temp.exists());
        assert_eq!(std::fs::read(&media.path).unwrap(), original);
        let mut preview = repair_result(&media);
        preview.parent_sha256 = "stale".into();
        deliver(&mut app, preview, true, false);
        finish(&mut app, &ctx);
        assert!(!app.image_editor.error.is_empty());
        assert!(app.image_editor.preview.is_some()); // Failed commits retain the result for inspection/retry.
        assert_eq!(std::fs::read(&media.path).unwrap(), original);
        let preview = repair_result(&media);
        let temp = preview.directory.path().to_owned();
        deliver(&mut app, preview, true, false);
        app.image_editor.reset(999);
        app.editor_poll(&ctx);
        assert!(!temp.exists());
        assert!(app.image_editor.preview.is_none());
        assert_eq!(std::fs::read(&media.path).unwrap(), original);
    }
    #[test]
    fn background_save_keeps_review_responsive_during_database_wait_and_cancels_before_commit() {
        let (_directory, mut app, media) = imported_app();
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let id = media.id as usize;
        let original = std::fs::read(&media.path).unwrap();
        for direct in [true, false] {
            app.image_editor.reset(id);
            let preview = repair_result(&media);
            let database = rusqlite::Connection::open(app.catalog.path.as_ref().unwrap()).unwrap();
            database.execute_batch("BEGIN IMMEDIATE").unwrap();
            let started = std::time::Instant::now();
            if direct {
                deliver(&mut app, preview, true, false);
                app.editor_poll(&ctx);
                assert!(
                    app.image_editor.texture.is_none(),
                    "direct save skips preview decoding/upload"
                );
            } else {
                app.image_editor.preview = Some(preview);
                app.editor_commit(&ctx, id).unwrap();
            }
            assert!(app.image_editor.active());
            assert!(app.image_editor.saving);
            for _ in 0..12 {
                app.editor_poll(&ctx);
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(1050.0, 720.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.shell(ctx),
                );
            }
            assert!(
                started.elapsed() < std::time::Duration::from_secs(1),
                "UI must keep rendering while the database worker waits"
            );
            assert!(app.image_editor.active());
            app.image_editor.reset(999);
            assert!(
                app.image_editor.active(),
                "track saving until worker acknowledges cancellation"
            );
            database.execute_batch("ROLLBACK").unwrap();
            finish(&mut app, &ctx);
            assert!(app.image_editor.preview.is_none());
            assert_eq!(std::fs::read(&media.path).unwrap(), original);
            assert_eq!(app.ingest.stored[&id].dimensions(), (800, 800));
            app.ingest.detail_texture = None;
        }
    }

    #[test]
    fn completed_save_delivered_after_navigation_updates_its_image_without_resetting_new_editor() {
        let (_directory, mut app, media) = imported_app();
        let ctx = egui::Context::default();
        let preview = editing::crop_preview(
            &media,
            bdm_domain::editing::CropRect {
                x: 10,
                y: 20,
                width: 400,
                height: 300,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        let mut repo =
            bdm_infrastructure::catalog::SqliteCatalog::open(app.catalog.path.as_ref().unwrap())
                .unwrap();
        let edited = repo.commit_repair(media.id, &preview).unwrap();
        let (sender, receiver) = mpsc::channel();
        app.image_editor.pending = Some(receiver);
        app.image_editor.cancel = Some(Arc::new(AtomicBool::new(false)));
        app.image_editor.saving = true;
        app.image_editor.reset(999);
        app.image_editor.tool = Tool::Polygon;
        app.image_editor.draft = vec![[20.0, 30.0]];
        sender
            .send(ResultEvent::Saved(Box::new(SaveResult {
                id: media.id as usize,
                preview,
                media: Ok(edited),
            })))
            .unwrap();
        app.editor_poll(&ctx);
        assert!(!app.image_editor.active());
        assert_eq!(
            app.ingest.stored[&(media.id as usize)].dimensions(),
            (400, 300)
        );
        assert_eq!(app.image_editor.media_id, Some(999));
        assert_eq!(app.image_editor.tool, Tool::Polygon);
        assert_eq!(app.image_editor.draft, vec![[20.0, 30.0]]);
        assert!(app.image_editor.preview.is_none());
    }

    #[test]
    #[ignore = "opt-in large-image crop timing; synthetic catalog only"]
    fn large_jpeg_crop_save_reports_worker_and_ui_poll_time() {
        let (_directory, mut app, media) = imported_image((3442, 2160), "jpg");
        let ctx = egui::Context::default();
        app.image_editor.cropping = true;
        app.prefs.confirm_image_edits = false;
        app.image_editor.crop.selection = Some(Rect::from_min_size(
            egui::pos2(100.0, 100.0),
            vec2(3000.0, 1800.0),
        ));
        let started = std::time::Instant::now();
        app.editor_start(&ctx, media.id as usize, false);
        let mut slowest_poll = std::time::Duration::ZERO;
        let mut polls = 0;
        while app.image_editor.active() {
            let frame = std::time::Instant::now();
            app.editor_poll(&ctx);
            slowest_poll = slowest_poll.max(frame.elapsed());
            polls += 1;
            assert!(started.elapsed() < std::time::Duration::from_secs(60));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        eprintln!(
            "3442x2160 JPEG -> 3000x1800: total={:?}, slowest UI poll={:?}, polls={polls}",
            started.elapsed(),
            slowest_poll
        );
        assert!(
            app.image_editor.error.is_empty(),
            "{}",
            app.image_editor.error
        );
        assert_eq!(
            app.ingest.stored[&(media.id as usize)].dimensions(),
            (3000, 1800)
        );
        assert!(app.image_editor.texture.is_none());
        assert_eq!(
            image::guess_format(&std::fs::read(&media.path).unwrap()).unwrap(),
            image::ImageFormat::Jpeg
        );
        assert!(slowest_poll < std::time::Duration::from_millis(100));
    }

    #[test]
    fn selection_transform_survives_zoom_pan_and_reset_discards_preview_state() {
        let image = Rect::from_min_size(egui::pos2(-200.0, 50.0), vec2(1200.0, 800.0));
        let dimensions = vec2(2400.0, 1600.0);
        let p = [800.0, 600.0];
        assert_eq!(pixel(screen(p, image, dimensions), image, dimensions), p);
        let mut editor = EditorState::default();
        editor.reset(1);
        editor.regions.push(MaskRegion::Rectangle {
            start: [0.0, 0.0],
            end: p,
        });
        editor.reset(1);
        assert_eq!(editor.regions.len(), 1);
        editor.reset(2);
        assert!(editor.regions.is_empty());
    }
}
