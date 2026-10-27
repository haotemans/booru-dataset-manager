use crate::{
    app::{BdmApp, Dialog},
    theme::*,
    widgets::*,
};
use bdm_domain::dedup::DuplicateCandidate;
use eframe::egui::{self, vec2, Rect, RichText, Sense};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
};

#[derive(Default)]
struct Original {
    texture: Option<egui::TextureHandle>,
    pending: Option<Receiver<Result<egui::ColorImage, String>>>,
    error: Option<String>,
}
impl Original {
    fn load(&mut self, ctx: &egui::Context, path: PathBuf) {
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let decoded = (|| {
                let mut reader =
                    image::ImageReader::open(path).map_err(|_| "无法读取原图".to_owned())?;
                let mut limits = image::Limits::default();
                limits.max_alloc = Some(256 * 1024 * 1024);
                reader.limits(limits);
                let decoded = reader
                    .decode()
                    .map_err(|_| "无法解码原图".to_owned())?
                    .to_rgba8();
                Ok(egui::ColorImage::from_rgba_unmultiplied(
                    [decoded.width() as usize, decoded.height() as usize],
                    decoded.as_raw(),
                ))
            })();
            let _ = sender.send(decoded);
            repaint.request_repaint();
        });
        self.texture = None;
        self.error = None;
        self.pending = Some(receiver);
    }
    fn poll(&mut self, ctx: &egui::Context, id: i64) {
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(Ok(image)) => {
                    self.texture = Some(ctx.load_texture(
                        format!("comparison-original-{id}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                    self.pending = None;
                }
                Ok(Err(error)) => {
                    self.error = Some(error);
                    self.pending = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.error = Some("原图加载中断".into());
                    self.pending = None;
                }
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(60));
                }
            }
        }
    }
}

#[derive(Clone)]
struct ReviewIntent {
    pair: DuplicateCandidate,
    hashes: [String; 2],
    survivor: Option<i64>,
    restore: bool,
}

#[derive(Default)]
pub struct SimilarityReview {
    pair: Option<(i64, i64)>,
    hashes: Option<[String; 2]>,
    scope: Option<std::collections::BTreeSet<i64>>,
    cursor: usize,
    originals: [Original; 2],
    zoom: f32,
    pan: egui::Vec2,
    error: Option<String>,
    pending: Option<Receiver<Result<(), String>>>,
    intent: Option<ReviewIntent>,
    ignored: bool,
    confirm_survivor: Option<i64>,
    notice: Option<String>,
    #[cfg(test)]
    stages: Vec<Rect>,
    #[cfg(test)]
    decisions: Vec<Rect>,
    #[cfg(test)]
    confirmation_buttons: Vec<Rect>,
    #[cfg(test)]
    tabs: Vec<Rect>,
}

impl SimilarityReview {
    pub(crate) fn capture_zoom(&mut self) {
        self.zoom = 2.0;
        self.pan = vec2(0.05, 0.0);
    }
    pub fn is_saving(&self) -> bool {
        self.pending.is_some()
    }
    pub fn release(&mut self) {
        self.originals = Default::default();
        self.pair = None;
        self.hashes = None;
        self.confirm_survivor = None;
    }
}

impl BdmApp {
    pub(crate) fn pending_similarity_pairs(&self) -> Vec<DuplicateCandidate> {
        self.similarity_pairs(false)
    }

    pub(crate) fn ignored_similarity_pairs(&self) -> Vec<DuplicateCandidate> {
        self.similarity_pairs(true)
    }

    fn similarity_pairs(&self, ignored: bool) -> Vec<DuplicateCandidate> {
        let valid: std::collections::BTreeSet<_> = self
            .library
            .media
            .iter()
            .map(|media| media.id as i64)
            .collect();
        let mut pairs: Vec<_> = self
            .workflows
            .candidates
            .iter()
            .filter(|pair| {
                pair.state == if ignored { "rejected" } else { "pending" }
                    && valid.contains(&pair.first)
                    && valid.contains(&pair.second)
            })
            .cloned()
            .collect();
        pairs.sort_by(|a, b| {
            b.cosine
                .unwrap_or(-1.0)
                .total_cmp(&a.cosine.unwrap_or(-1.0))
                .then_with(|| a.first.cmp(&b.first))
                .then_with(|| a.second.cmp(&b.second))
        });
        pairs
    }

    pub(crate) fn open_pair_review(&mut self, pair: &DuplicateCandidate) {
        self.similarity_review.ignored = pair.state == "rejected";
        self.similarity_review.scope = Some(
            self.library
                .media
                .iter()
                .map(|media| media.id as i64)
                .collect(),
        );
        self.begin_pair_review(pair);
    }

    pub(crate) fn open_scoped_pair_review(&mut self, pair: &DuplicateCandidate) {
        self.similarity_review.ignored = pair.state == "rejected";
        self.similarity_review.scope = Some(self.action_ids().into_iter().collect());
        self.begin_pair_review(pair);
    }

    fn begin_pair_review(&mut self, pair: &DuplicateCandidate) {
        self.similarity_review.release();
        self.similarity_review.cursor = self
            .review_similarity_pairs()
            .iter()
            .position(|p| p.first == pair.first && p.second == pair.second)
            .unwrap_or(0);
        self.similarity_review.error = None;
        self.similarity_review.notice = None;
        self.dialog = Some(Dialog::Similarity);
    }

    fn review_similarity_pairs(&self) -> Vec<DuplicateCandidate> {
        self.similarity_pairs(self.similarity_review.ignored)
            .into_iter()
            .filter(|pair| {
                self.similarity_review
                    .scope
                    .as_ref()
                    .is_none_or(|scope| scope.contains(&pair.first) && scope.contains(&pair.second))
            })
            .collect()
    }

    pub(crate) fn poll_similarity_review(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.similarity_review.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Disconnected) => Err("审核保存中断，请重试".into()),
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(60));
                return;
            }
        };
        self.similarity_review.pending = None;
        let intent = self
            .similarity_review
            .intent
            .take()
            .expect("pending comparison intent");
        match result {
            Ok(()) => {
                if let Some(keep) = intent.survivor {
                    let remove = if keep == intent.pair.first {
                        intent.pair.second
                    } else {
                        intent.pair.first
                    };
                    self.remove_sample_media(&std::collections::BTreeSet::from([remove as usize]));
                }
                self.apply_similarity_result(&intent);
                if let Err(error) = self.reload_media() {
                    self.similarity_review.error = Some(error);
                }
                self.similarity_review.release();
            }
            Err(error) => self.similarity_review.error = Some(error),
        }
    }

    fn decide_similarity(
        &mut self,
        ctx: &egui::Context,
        pair: &DuplicateCandidate,
        survivor: Option<i64>,
    ) -> Result<(), String> {
        self.save_similarity(ctx, pair, survivor, false)
    }

    fn save_similarity(
        &mut self,
        ctx: &egui::Context,
        pair: &DuplicateCandidate,
        survivor: Option<i64>,
        restore: bool,
    ) -> Result<(), String> {
        if self.similarity_review.pending.is_some()
            || self.workflow_active()
            || self.ingest_active()
            || self.image_editor.active()
        {
            return Err("请等待当前处理结束".into());
        }
        if !self
            .similarity_pairs(restore)
            .iter()
            .any(|saved| saved == pair)
        {
            return Err("候选已变化，请重新查看".into());
        }
        if restore && survivor.is_some() {
            return Err("恢复操作不删除图片".into());
        }
        let hashes = self
            .similarity_review
            .hashes
            .clone()
            .filter(|_| self.similarity_review.pair == Some((pair.first, pair.second)));
        let intent = ReviewIntent {
            pair: pair.clone(),
            hashes: if self.demo {
                hashes.unwrap_or_default()
            } else {
                hashes.ok_or("图片已变化，请重新查看")?
            },
            survivor,
            restore,
        };
        if self.demo {
            if let Some(keep) = survivor {
                let remove = if keep == pair.first {
                    pair.second
                } else {
                    pair.first
                };
                self.remove_sample_media(&std::collections::BTreeSet::from([remove as usize]));
            }
            self.apply_similarity_result(&intent);
            self.similarity_review.release();
            return Ok(());
        }
        if self.is_capture() {
            return Err("截图模式不保存".into());
        }
        let path = self.catalog.path.clone().ok_or("数据库未打开")?;
        let task = intent.clone();
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name("similarity-decision".into())
            .spawn(move || {
                let result = bdm_infrastructure::catalog::SqliteCatalog::open(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|mut repo| {
                        if task.restore {
                            repo.restore_candidate(&task.pair, &task.hashes)
                        } else {
                            repo.resolve_candidate(&task.pair, &task.hashes, task.survivor)
                        }
                        .map_err(|e| e.to_string())
                    });
                let _ = sender.send(result);
                repaint.request_repaint();
            })
            .map_err(|_| "无法保存审核结果")?;
        self.similarity_review.pending = Some(receiver);
        self.similarity_review.intent = Some(intent);
        self.similarity_review.error = None;
        Ok(())
    }

    fn apply_similarity_result(&mut self, intent: &ReviewIntent) {
        if let Some(keep) = intent.survivor {
            let remove = if keep == intent.pair.first {
                intent.pair.second
            } else {
                intent.pair.first
            };
            self.workflows
                .candidates
                .retain(|pair| pair.first != remove && pair.second != remove);
            self.similarity_review.notice = Some(format!(
                "已保留{}，删除{}并合并来源和收藏关联",
                if keep == intent.pair.first {
                    "左图"
                } else {
                    "右图"
                },
                if remove == intent.pair.first {
                    "左图"
                } else {
                    "右图"
                }
            ));
        } else {
            if let Some(saved) =
                self.workflows.candidates.iter_mut().find(|pair| {
                    pair.first == intent.pair.first && pair.second == intent.pair.second
                })
            {
                saved.state = if intent.restore {
                    "pending"
                } else {
                    "rejected"
                }
                .into();
            }
            self.similarity_review.notice = Some(
                if intent.restore {
                    "已恢复到待审核，两张图片均保留"
                } else {
                    "已忽略误报，两张均保留；可在「已忽略」中恢复"
                }
                .into(),
            );
        }
    }

    fn switch_similarity_view(&mut self, ignored: bool) {
        self.similarity_review.release();
        self.similarity_review.ignored = ignored;
        self.similarity_review.cursor = 0;
        self.similarity_review.error = None;
        self.similarity_review.notice = None;
    }

    pub(crate) fn show_similarity_review(&mut self, ctx: &egui::Context) -> bool {
        self.poll_similarity_review(ctx);
        if self.similarity_review.confirm_survivor.is_some()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.similarity_review.confirm_survivor = None;
        }
        let pairs = self.review_similarity_pairs();
        let previous = self.similarity_review.pair;
        if let Some(position) = pairs
            .iter()
            .position(|pair| Some((pair.first, pair.second)) == previous)
        {
            self.similarity_review.cursor = position;
        }
        self.similarity_review.cursor = self
            .similarity_review
            .cursor
            .min(pairs.len().saturating_sub(1));
        let pair = pairs.get(self.similarity_review.cursor).cloned();
        if let Some(pair) = &pair {
            if previous != Some((pair.first, pair.second)) {
                self.similarity_review.release();
                self.similarity_review.pair = Some((pair.first, pair.second));
                self.similarity_review.hashes = self
                    .ingest
                    .stored
                    .get(&(pair.first as usize))
                    .zip(self.ingest.stored.get(&(pair.second as usize)))
                    .map(|(left, right)| [left.sha256.clone(), right.sha256.clone()]);
                self.similarity_review.zoom = 1.0;
                self.similarity_review.pan = egui::Vec2::ZERO;
                if self.prefs.auto_load_review_original && !self.demo && !self.is_capture() {
                    for (index, id) in [pair.first, pair.second].into_iter().enumerate() {
                        if let Some(media) = self.ingest.stored.get(&(id as usize)) {
                            self.similarity_review.originals[index].load(ctx, media.path.clone());
                        }
                    }
                }
            }
        } else {
            self.similarity_review.release();
        }
        // The initially chosen pair also starts with the default fit.
        if self.similarity_review.zoom <= 0.0 {
            self.similarity_review.zoom = 1.0;
        }
        let mut close = false;
        let mut switch_to = None;
        let screen = ctx.screen_rect();
        let size = screen.size() - vec2(24.0, 32.0);
        egui::Window::new("相似图审核")
            .id("similarity_review_fullscreen".into())
            .order(egui::Order::Foreground)
            .title_bar(false)
            .resizable(false)
            .movable(false)
            .collapsible(false)
            .fixed_pos(screen.min)
            .fixed_size(size)
            .frame(egui::Frame::none().fill(NAV).inner_margin(12.0))
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                let header = Rect::from_min_size(rect.min, vec2(rect.width(), 64.0));
                let footer =
                    Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 112.0), rect.max);
                let body = Rect::from_min_max(
                    egui::pos2(rect.left(), header.bottom() + 8.0),
                    egui::pos2(rect.right(), footer.top() - 8.0),
                );
                #[cfg(test)]
                {
                    self.similarity_review.stages.clear();
                    self.similarity_review.decisions.clear();
                    self.similarity_review.confirmation_buttons.clear();
                    self.similarity_review.tabs.clear();
                }
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(header), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("相似图审核").font(medium(17.0)));
                        if !pairs.is_empty() {
                            badge(
                                ui,
                                &format!("{} / {}", self.similarity_review.cursor + 1, pairs.len()),
                                ACCENT,
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            close |= icon_button(ui, "close", "结束审核 · Esc", false).clicked();
                        });
                    });
                    ui.horizontal(|ui| {
                        let pending_count = self.similarity_pairs_in_scope(false).len();
                        let ignored_count = self.similarity_pairs_in_scope(true).len();
                        for (ignored, label) in [(false, format!("待审核 {pending_count}")), (true, format!("已忽略 {ignored_count}"))] {
                            let response = ui.add_enabled(!self.similarity_review.is_saving(), egui::Button::new(label).selected(self.similarity_review.ignored == ignored));
                            #[cfg(test)]
                            self.similarity_review.tabs.push(response.rect);
                            if response.clicked() && self.similarity_review.ignored != ignored {
                                switch_to = Some(ignored);
                            }
                        }
                        if let Some(pair) = &pair {
                            let score = pair.cosine.filter(|v| v.is_finite()).map_or("—".into(), |v| format!("{:.1}%", v * 100.0));
                            let distance = pair.phash_distance.map_or("—".into(), |d| d.to_string());
                            ui.label(RichText::new(format!("相似度 {score} · pHash {distance}")).size(12.0).color(MUTED))
                                .on_hover_text("相似度越高、pHash 距离越小，画面越接近；仍需检查背景、表情、服装等差异。");
                        }
                    });
                });
                if let Some(pair) = &pair {
                    let gap = 12.0;
                    let width = (body.width() - gap) * 0.5;
                    for (index, id) in [pair.first, pair.second].into_iter().enumerate() {
                        let panel = Rect::from_min_size(
                            body.min + vec2(index as f32 * (width + gap), 0.0),
                            vec2(width, body.height()),
                        );
                        self.comparison_image(ui, panel, index, id);
                    }
                } else {
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body), |ui| {
                        if self.similarity_review.ignored {
                            empty(ui, "暂无已忽略项", "忽略的误报会保存在这里，随时可恢复审核", "check");
                        } else {
                            empty(ui, "审核完成", "当前范围没有待审核候选，可切换「已忽略」重新查看", "check");
                        }
                    });
                }
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(footer), |ui| {
                    let busy = self.similarity_review.pending.is_some();
                    if let Some(error) = &self.similarity_review.error {
                        ui.label(RichText::new("保存失败，未继续下一对").color(DANGER).size(12.0)).on_hover_text(error);
                    } else if let Some(keep) = self.similarity_review.confirm_survivor {
                        let left = pair.as_ref().is_some_and(|pair| keep == pair.first);
                        ui.label(RichText::new(format!("保留{}、永久删除{}；来源和收藏关联将合并。", if left { "左图" } else { "右图" }, if left { "右图" } else { "左图" })).size(12.0).color(DANGER));
                    } else if let Some(notice) = &self.similarity_review.notice {
                        ui.label(RichText::new(notice).size(12.0).color(ACCENT));
                    } else {
                        ui.label(RichText::new(if self.similarity_review.ignored { "两张图片均已保留；恢复后可重新选择保留哪张。" } else { "滚轮同步缩放 · 拖动平移 · 双击适应窗口 · 1 / 2 / 3 选择" }).size(12.0).color(MUTED));
                    }
                    ui.horizontal(|ui| {
                        let editable = !busy
                            && !self.workflow_active()
                            && !self.ingest_active()
                            && !self.image_editor.active()
                            && (!self.is_capture() || self.demo);
                        if let Some(pair) = &pair {
                            if self.similarity_review.ignored {
                                if ui.add_enabled(editable, egui::Button::new("恢复到待审核").min_size(vec2(220.0, 42.0))).clicked() {
                                    if let Err(error) = self.save_similarity(ctx, pair, None, true) {
                                        self.similarity_review.error = Some(error);
                                    }
                                }
                            } else if let Some(keep) = self.similarity_review.confirm_survivor {
                                let confirm = ui.add_enabled(editable, egui::Button::new(RichText::new(if keep == pair.first { "确认删除右图" } else { "确认删除左图" }).color(egui::Color32::WHITE)).fill(DANGER).min_size(vec2(220.0, 42.0)));
                                let cancel = ui.add_enabled(!busy, egui::Button::new("取消 · Esc").min_size(vec2(140.0, 42.0)));
                                #[cfg(test)]
                                self.similarity_review.confirmation_buttons.extend([confirm.rect, cancel.rect]);
                                if confirm.clicked() {
                                    if let Err(error) = self.decide_similarity(ctx, pair, Some(keep)) {
                                        self.similarity_review.error = Some(error);
                                    }
                                } else if cancel.clicked() {
                                    self.similarity_review.confirm_survivor = None;
                                }
                            } else {
                            for (key, label, survivor, color) in [
                                (egui::Key::Num1, "保留左图 · 1", Some(pair.first), ACCENT),
                                (egui::Key::Num2, "忽略误报 · 2", None, INK),
                                (egui::Key::Num3, "保留右图 · 3", Some(pair.second), ACCENT),
                            ] {
                                let response = ui.add_enabled(
                                    editable,
                                    egui::Button::new(
                                        RichText::new(label).color(egui::Color32::WHITE),
                                    )
                                    .fill(color)
                                    .min_size(vec2((footer.width() - 24.0) / 3.0, 42.0)),
                                );
                                #[cfg(test)]
                                {
                                    self.similarity_review.decisions.push(response.rect);
                                }
                                if response
                                    .on_disabled_hover_text("当前处理结束后即可审核")
                                    .on_hover_text(if survivor.is_some() {
                                        "选择保留这张图片，确认后删除另一张，并合并来源和收藏关联"
                                    } else {
                                        "两张都保留，后续筛查跳过这对图片；可在「已忽略」恢复"
                                    })
                                    .clicked()
                                    || (editable && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key)))
                                {
                                    if let Some(keep) = survivor {
                                        if self.demo || self.is_capture() {
                                            if let Err(error) = self.decide_similarity(ctx, pair, Some(keep)) {
                                                self.similarity_review.error = Some(error);
                                            }
                                        } else {
                                            self.similarity_review.confirm_survivor = Some(keep);
                                            self.similarity_review.error = None;
                                        }
                                    } else if let Err(error) = self.decide_similarity(ctx, pair, None) {
                                        self.similarity_review.error = Some(error);
                                    }
                                }
                            }
                            }
                        } else {
                            close |= primary(ui, "完成").clicked();
                        }
                    });
                    if !pairs.is_empty() {
                        ui.horizontal(|ui| {
                            if busy {
                                ui.spinner();
                            }
                            if ui
                                .add_enabled_ui(!busy && self.similarity_review.cursor > 0, |ui| {
                                    icon_button(ui, "back", "上一对 · ←", false)
                                })
                                .inner
                                .clicked()
                                || (!busy
                                    && ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft))
                                    && self.similarity_review.cursor > 0)
                            {
                                self.similarity_review.cursor -= 1;
                                self.similarity_review.release();
                            }
                            if ui
                                .add_enabled_ui(
                                    !busy && self.similarity_review.cursor + 1 < pairs.len(),
                                    |ui| icon_button(ui, "forward", "下一对 · →", false),
                                )
                                .inner
                                .clicked()
                                || (!busy
                                    && ctx.input(|i| i.key_pressed(egui::Key::ArrowRight))
                                    && self.similarity_review.cursor + 1 < pairs.len())
                            {
                                self.similarity_review.cursor += 1;
                                self.similarity_review.release();
                            }
                            if icon_button(ui, "reset", "同步适应窗口", false).clicked() {
                                self.similarity_review.zoom = 1.0;
                                self.similarity_review.pan = egui::Vec2::ZERO;
                            }
                            ui.label(RichText::new(format!("{:.0}%", self.similarity_review.zoom * 100.0)).size(12.0).color(MUTED));
                            ui.label(RichText::new("上一对 / 下一对 · ← →").size(11.0).color(MUTED));
                        });
                    }
                });
            });
        if let Some(ignored) = switch_to {
            self.switch_similarity_view(ignored);
        }
        close
    }

    fn similarity_pairs_in_scope(&self, ignored: bool) -> Vec<DuplicateCandidate> {
        self.similarity_pairs(ignored)
            .into_iter()
            .filter(|pair| {
                self.similarity_review
                    .scope
                    .as_ref()
                    .is_none_or(|scope| scope.contains(&pair.first) && scope.contains(&pair.second))
            })
            .collect()
    }

    fn comparison_image(&mut self, ui: &mut egui::Ui, panel: Rect, index: usize, id: i64) {
        let stage = Rect::from_min_max(panel.min + vec2(0.0, 46.0), panel.max - vec2(0.0, 28.0));
        #[cfg(test)]
        {
            self.similarity_review.stages.push(stage);
        }
        let can_load = !self.demo && !self.is_capture();
        let source_path = self
            .ingest
            .stored
            .get(&(id as usize))
            .map(|media| media.path.clone());
        self.similarity_review.originals[index].poll(ui.ctx(), id);
        let requested = {
            let original = &self.similarity_review.originals[index];
            original.pending.is_some() || original.texture.is_some() || original.error.is_some()
        };
        ui.allocate_new_ui(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(panel.min, vec2(panel.width(), 36.0))),
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(if index == 0 { "左图" } else { "右图" }).strong());
                    if self
                        .library
                        .media
                        .iter()
                        .any(|media| media.id == id as usize && media.keep)
                    {
                        badge(ui, "已保留", ACCENT);
                    }
                    if ui
                        .add_enabled_ui(can_load, |ui| {
                            icon_button(
                                ui,
                                "image",
                                if requested {
                                    "使用缩略图"
                                } else {
                                    "加载原图"
                                },
                                requested,
                            )
                        })
                        .inner
                        .clicked()
                    {
                        if requested {
                            self.similarity_review.originals[index] = Original::default();
                        } else if let Some(path) = source_path {
                            self.similarity_review.originals[index].load(ui.ctx(), path);
                        }
                    }
                    if self.similarity_review.originals[index].pending.is_some() {
                        ui.spinner();
                    }
                    if let Some(error) = &self.similarity_review.originals[index].error {
                        ui.colored_label(DANGER, "加载失败").on_hover_text(error);
                    }
                    if ui
                        .add_enabled_ui(can_load, |ui| {
                            icon_button(ui, "folder", "在系统查看器打开", false)
                        })
                        .inner
                        .clicked()
                    {
                        if let Err(error) = self.open_original(id as usize) {
                            self.similarity_review.error = Some(error);
                        }
                    }
                });
            },
        );
        let dimensions = self
            .library
            .media
            .iter()
            .find(|media| media.id == id as usize)
            .map_or(vec2(1.0, 1.0), |media| {
                vec2(media.width as f32, media.height as f32)
            });
        let response = ui.interact(
            stage,
            ui.id().with(("comparison-canvas", id)),
            Sense::click_and_drag(),
        );
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                self.similarity_review.zoom =
                    (self.similarity_review.zoom * (scroll * 0.005).exp()).clamp(1.0, 12.0);
            }
        }
        if response.dragged() {
            self.similarity_review.pan += ui.input(|input| input.pointer.delta()) / stage.size();
        }
        if response.double_clicked() {
            self.similarity_review.zoom = 1.0;
            self.similarity_review.pan = egui::Vec2::ZERO;
        }
        let fit = (stage.width() / dimensions.x).min(stage.height() / dimensions.y);
        let size = dimensions * fit * self.similarity_review.zoom;
        let excess = ((size - stage.size()) * 0.5).max(egui::Vec2::ZERO) / stage.size();
        let pan = self.similarity_review.pan.clamp(-excess, excess);
        let image_rect = Rect::from_center_size(stage.center() + pan * stage.size(), size);
        ui.painter().rect_filled(stage, 8.0, BG);
        let mut canvas = ui.new_child(egui::UiBuilder::new().max_rect(stage));
        canvas.set_clip_rect(stage);
        if let Some(texture) = &self.similarity_review.originals[index].texture {
            canvas.painter().image(
                texture.id(),
                image_rect,
                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            self.paint_media(&canvas, image_rect, id as usize);
        }
        ui.allocate_new_ui(
            egui::UiBuilder::new().max_rect(Rect::from_min_size(
                egui::pos2(panel.left(), panel.bottom() - 25.0),
                vec2(panel.width(), 25.0),
            )),
            |ui| {
                ui.horizontal(|ui| {
                    let format = self
                        .ingest
                        .stored
                        .get(&(id as usize))
                        .and_then(|media| media.path.extension())
                        .and_then(|ext| ext.to_str())
                        .unwrap_or("图片")
                        .to_ascii_uppercase();
                    ui.label(
                        RichText::new(format!(
                            "{} × {} · {format}",
                            dimensions.x as usize, dimensions.y as usize,
                        ))
                        .size(11.0)
                        .color(MUTED),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(self.media_name(id as usize)).size(11.0))
                            .truncate(),
                    )
                    .on_hover_text(self.media_name(id as usize));
                });
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Preferences;
    fn pair(first: i64, second: i64) -> DuplicateCandidate {
        DuplicateCandidate {
            first,
            second,
            phash_distance: Some(12),
            cosine: Some(0.96),
            model_version: Some("fixture".into()),
            state: "pending".into(),
        }
    }
    fn frame(
        ctx: &egui::Context,
        app: &mut BdmApp,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| app.shell(ctx),
        )
    }
    fn click(ctx: &egui::Context, app: &mut BdmApp, size: egui::Vec2, pos: egui::Pos2) {
        for pressed in [true, false] {
            frame(
                ctx,
                app,
                size,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
    }
    #[test]
    fn large_visual_comparison_keeps_buttons_outside_artwork_and_advances_decisions() {
        for size in [vec2(981.0, 613.0), vec2(1050.0, 720.0), vec2(1440.0, 960.0)] {
            for choice in 0..3 {
                let ctx = egui::Context::default();
                crate::theme::setup(&ctx);
                let mut app = BdmApp::from_preferences(Preferences::default(), true, None);
                app.workflows.candidates = vec![pair(1, 2), pair(3, 4)];
                app.open_pair_review(&pair(1, 2));
                for _ in 0..3 {
                    frame(&ctx, &mut app, size, vec![]);
                }
                let stages = app.similarity_review.stages.clone();
                let buttons = app.similarity_review.decisions.clone();
                assert_eq!(stages.len(), 2);
                assert_eq!(buttons.len(), 3);
                for stage in &stages {
                    assert!(stage.width() > size.x * 0.4);
                    assert!(stage.height() > size.y * 0.5);
                }
                for button in &buttons {
                    assert!(Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(*button));
                    assert!(stages.iter().all(|stage| !stage.intersects(*button)));
                }
                click(&ctx, &mut app, size, buttons[choice].center());
                frame(&ctx, &mut app, size, vec![]);
                assert_eq!(app.similarity_review.pair, Some((3, 4)));
                assert_eq!(
                    app.library.media.iter().any(|media| media.id == 1),
                    choice != 2
                );
                assert_eq!(
                    app.library.media.iter().any(|media| media.id == 2),
                    choice != 0
                );
                if choice == 1 {
                    assert_eq!(app.workflows.candidates[0].state, "rejected");
                }
                let center = app.similarity_review.stages[0].center();
                frame(
                    &ctx,
                    &mut app,
                    size,
                    vec![
                        egui::Event::PointerMoved(center),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: vec2(0.0, 80.0),
                            modifiers: Default::default(),
                        },
                    ],
                );
                assert!(app.similarity_review.zoom > 1.0);
                let button = app.similarity_review.decisions[1].center();
                click(&ctx, &mut app, size, button);
                let output = frame(&ctx, &mut app, size, vec![]);
                assert!(app.pending_similarity_pairs().is_empty());
                assert!(app
                    .similarity_review
                    .originals
                    .iter()
                    .all(|image| image.pending.is_none() && image.texture.is_none()));
                assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text()=="审核完成")));
                frame(
                    &ctx,
                    &mut app,
                    size,
                    vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                );
                assert!(app.dialog.is_none());
            }
        }
    }
    #[test]
    fn scope_and_close_preserve_candidates_and_discard_late_original_replies() {
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let size = vec2(1050.0, 720.0);
        let mut app = BdmApp::from_preferences(Preferences::default(), true, None);
        app.workflows.candidates = vec![pair(1, 2), pair(3, 4)];
        app.action_snapshot = vec![1, 2];
        app.open_scoped_pair_review(&pair(1, 2));
        assert_eq!(app.review_similarity_pairs().len(), 1);
        frame(&ctx, &mut app, size, vec![]);
        let (sender, receiver) = mpsc::channel();
        app.similarity_review.originals[0].pending = Some(receiver);
        frame(
            &ctx,
            &mut app,
            size,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(sender.send(Ok(egui::ColorImage::example())).is_err());
        assert_eq!(app.pending_similarity_pairs().len(), 2);
        app.open_pair_review(&pair(3, 4));
        frame(&ctx, &mut app, size, vec![]);
        assert_eq!(app.similarity_review.pair, Some((3, 4)));
    }
    #[test]
    fn save_error_keeps_pair_and_does_not_advance() {
        let ctx = egui::Context::default();
        let mut app = BdmApp::from_preferences(Preferences::default(), true, None);
        let pair = pair(1, 2);
        app.workflows.candidates = vec![pair.clone()];
        app.open_pair_review(&pair);
        app.similarity_review.pair = Some((1, 2));
        let (sender, receiver) = mpsc::channel();
        app.similarity_review.pending = Some(receiver);
        app.similarity_review.intent = Some(ReviewIntent {
            pair,
            hashes: [String::new(), String::new()],
            survivor: Some(2),
            restore: false,
        });
        sender.send(Err("fixture storage failure".into())).unwrap();
        app.poll_similarity_review(&ctx);
        assert_eq!(app.similarity_review.pair, Some((1, 2)));
        assert_eq!(app.pending_similarity_pairs().len(), 1);
        assert!(app.library.media.iter().any(|media| media.id == 1));
        assert!(app.similarity_review.error.is_some());
    }

    #[test]
    fn original_preference_loads_both_off_thread_and_navigation_discards_the_old_pair() {
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let size = vec2(1050.0, 720.0);
        let directory = tempfile::tempdir().unwrap();
        let mut app = BdmApp::from_preferences(Preferences::default(), false, None);
        for id in 1..=4 {
            let path = directory.path().join(format!("original-{id}.png"));
            image::RgbImage::from_pixel(80, 60, image::Rgb([id as u8 * 20, 30, 40]))
                .save(&path)
                .unwrap();
            app.library.media.push(crate::model::Media {
                id,
                artist: None,
                character: None,
                tags: vec![],
                keep: false,
                optimized: false,
                caption: false,
                width: 80,
                height: 60,
                rating: 1,
            });
            app.ingest.stored.insert(
                id,
                bdm_domain::ingest::StoredMedia {
                    id: id as i64,
                    path,
                    thumbnail: directory.path().join("missing-preview.png"),
                    sha256: format!("hash-{id}"),
                    source_sha256: String::new(),
                    collections: Default::default(),
                    keep: false,
                    minimum_short_side: 1,
                    flatten_transparency: true,
                    edit: None,
                    post: bdm_domain::ingest::SourcePost {
                        id: id as i64,
                        md5: String::new(),
                        file_url: None,
                        file_ext: "png".into(),
                        file_size: 1,
                        image_width: 80,
                        image_height: 60,
                        tag_string: String::new(),
                        tag_string_artist: String::new(),
                        tag_string_character: String::new(),
                        tag_string_general: String::new(),
                        tag_string_copyright: String::new(),
                        tag_string_meta: String::new(),
                        rating: "g".into(),
                        is_deleted: false,
                    },
                },
            );
        }
        app.workflows.candidates = vec![pair(1, 2), pair(3, 4)];
        app.open_pair_review(&pair(1, 2));
        for _ in 0..3 {
            frame(&ctx, &mut app, size, vec![]);
        }
        assert!(app
            .similarity_review
            .originals
            .iter()
            .all(|original| original.pending.is_none() && original.texture.is_none()));
        app.prefs.auto_load_review_original = true;
        app.open_pair_review(&pair(1, 2));
        frame(&ctx, &mut app, size, vec![]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app
            .similarity_review
            .originals
            .iter()
            .any(|original| original.texture.is_none())
        {
            frame(&ctx, &mut app, size, vec![]);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app
            .similarity_review
            .originals
            .iter()
            .all(|original| original.texture.as_ref().unwrap().size() == [80, 60]));
        // A manual thumbnail override persists for this pair until navigation changes it.
        app.similarity_review.originals[0] = Default::default();
        frame(&ctx, &mut app, size, vec![]);
        assert!(app.similarity_review.originals[0].pending.is_none());
        let (sender, receiver) = mpsc::channel();
        app.similarity_review.originals[0].pending = Some(receiver);
        frame(
            &ctx,
            &mut app,
            size,
            vec![egui::Event::Key {
                key: egui::Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(sender.send(Ok(egui::ColorImage::example())).is_err());
        frame(&ctx, &mut app, size, vec![]);
        assert_eq!(app.similarity_review.pair, Some((3, 4)));
        assert_eq!(
            app.similarity_review.hashes,
            Some(["hash-3".into(), "hash-4".into()])
        );
        frame(
            &ctx,
            &mut app,
            size,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(app
            .similarity_review
            .originals
            .iter()
            .all(|original| original.pending.is_none() && original.texture.is_none()));
    }
}
