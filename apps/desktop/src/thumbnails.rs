use eframe::egui;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
};

const CACHE_LIMIT: usize = 128;
const QUEUE_LIMIT: usize = 8;
const UPLOADS_PER_FRAME: usize = 4;

struct Request {
    id: usize,
    ticket: u64,
    path: PathBuf,
}
struct Reply {
    id: usize,
    ticket: u64,
    image: Option<egui::ColorImage>,
}
struct Worker {
    sender: SyncSender<Request>,
    receiver: Receiver<Reply>,
}

#[derive(Default)]
pub struct Thumbnails {
    worker: Option<Worker>,
    pending: BTreeMap<usize, u64>,
    failed: BTreeSet<usize>,
    used: BTreeMap<usize, u64>,
    clock: u64,
}

impl Thumbnails {
    pub fn invalidate(&mut self, id: usize) {
        self.pending.remove(&id);
        self.failed.remove(&id);
        self.used.remove(&id);
    }

    pub fn request(&mut self, ctx: &egui::Context, id: usize, path: PathBuf) {
        if self.pending.contains_key(&id) || self.failed.contains(&id) {
            return;
        }
        if self.worker.is_none() {
            let (sender, requests) = mpsc::sync_channel::<Request>(QUEUE_LIMIT);
            let (results, receiver) = mpsc::sync_channel(QUEUE_LIMIT);
            let repaint = ctx.clone();
            if std::thread::Builder::new()
                .name("thumbnail-decoder".into())
                .spawn(move || {
                    while let Ok(request) = requests.recv() {
                        let image = decode(&request.path);
                        if results
                            .send(Reply {
                                id: request.id,
                                ticket: request.ticket,
                                image,
                            })
                            .is_err()
                        {
                            break;
                        }
                        repaint.request_repaint();
                    }
                })
                .is_err()
            {
                self.failed.insert(id);
                return;
            }
            self.worker = Some(Worker { sender, receiver });
        }
        self.clock += 1;
        let ticket = self.clock;
        match self
            .worker
            .as_ref()
            .unwrap()
            .sender
            .try_send(Request { id, ticket, path })
        {
            Ok(()) => {
                self.pending.insert(id, ticket);
            }
            Err(TrySendError::Full(_)) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(30));
            }
            Err(TrySendError::Disconnected(_)) => {
                self.failed.insert(id);
            }
        }
    }

    pub fn touch(&mut self, id: usize) {
        self.clock += 1;
        self.used.insert(id, self.clock);
    }

    pub fn poll(&mut self, ctx: &egui::Context, cache: &mut BTreeMap<usize, egui::TextureHandle>) {
        for _ in 0..UPLOADS_PER_FRAME {
            let Some(reply) = self
                .worker
                .as_ref()
                .and_then(|worker| worker.receiver.try_recv().ok())
            else {
                break;
            };
            if self.pending.get(&reply.id) != Some(&reply.ticket) {
                continue;
            }
            self.pending.remove(&reply.id);
            if let Some(image) = reply.image {
                while cache.len() >= CACHE_LIMIT {
                    let oldest = cache
                        .keys()
                        .min_by_key(|id| self.used.get(id).copied().unwrap_or(0))
                        .copied()
                        .unwrap();
                    cache.remove(&oldest);
                    self.used.remove(&oldest);
                }
                cache.insert(
                    reply.id,
                    ctx.load_texture(
                        format!("thumbnail-{}-{}", reply.id, reply.ticket),
                        image,
                        egui::TextureOptions::LINEAR,
                    ),
                );
                self.touch(reply.id);
            } else {
                self.failed.insert(reply.id);
            }
        }
        if !self.pending.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
}

fn decode(path: &std::path::Path) -> Option<egui::ColorImage> {
    let mut reader = image::ImageReader::open(path).ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    let decoded = if decoded.width() > 384 || decoded.height() > 384 {
        decoded.thumbnail(384, 384)
    } else {
        decoded
    }
    .to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [decoded.width() as usize, decoded.height() as usize],
        decoded.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_cache_keeps_recent_images_and_discards_stale_replies() {
        let ctx = egui::Context::default();
        let (sender, receiver) = mpsc::sync_channel(QUEUE_LIMIT);
        let (requests, _) = mpsc::sync_channel(QUEUE_LIMIT);
        let mut thumbnails = Thumbnails {
            worker: Some(Worker {
                sender: requests,
                receiver,
            }),
            ..Default::default()
        };
        let mut cache = BTreeMap::new();
        for id in 0..CACHE_LIMIT {
            cache.insert(
                id,
                ctx.load_texture(
                    format!("fixture-{id}"),
                    egui::ColorImage::example(),
                    Default::default(),
                ),
            );
            thumbnails.touch(id);
        }
        thumbnails.touch(0);
        thumbnails.pending.insert(200, 1);
        sender
            .send(Reply {
                id: 200,
                ticket: 1,
                image: Some(egui::ColorImage::example()),
            })
            .unwrap();
        thumbnails.poll(&ctx, &mut cache);
        assert_eq!(cache.len(), CACHE_LIMIT);
        assert!(cache.contains_key(&0));
        assert!(!cache.contains_key(&1));
        assert!(cache.contains_key(&200));
        thumbnails.pending.insert(201, 2);
        thumbnails.invalidate(201);
        thumbnails.pending.insert(201, 3);
        sender
            .send(Reply {
                id: 201,
                ticket: 2,
                image: Some(egui::ColorImage::example()),
            })
            .unwrap();
        thumbnails.poll(&ctx, &mut cache);
        assert!(!cache.contains_key(&201));
        assert_eq!(thumbnails.pending[&201], 3);
    }
    #[test]
    fn failed_decode_is_cached_until_invalidation() {
        let ctx = egui::Context::default();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preview.png");
        let mut thumbnails = Thumbnails::default();
        let mut cache = BTreeMap::new();
        thumbnails.request(&ctx, 1, path.clone());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !thumbnails.failed.contains(&1) {
            thumbnails.poll(&ctx, &mut cache);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        image::RgbImage::from_pixel(16, 12, image::Rgb([20, 40, 60]))
            .save(&path)
            .unwrap();
        thumbnails.request(&ctx, 1, path.clone());
        assert!(thumbnails.pending.is_empty());
        thumbnails.invalidate(1);
        thumbnails.request(&ctx, 1, path);
        while !cache.contains_key(&1) {
            thumbnails.poll(&ctx, &mut cache);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(cache[&1].size(), [16, 12]);
    }
}
