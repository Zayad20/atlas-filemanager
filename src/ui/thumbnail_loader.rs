use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use egui::load::{
    LoadError, SizeHint, SizedTexture, TextureLoadResult, TextureLoader, TexturePoll,
};

const URI_PREFIX: &str = "thumbnail://";
const WORKER_COUNT: usize = 2;
const QUEUE_DEBOUNCE: Duration = Duration::from_millis(24);
const MAX_READY_ITEMS: usize = 512;
const MAX_READY_BYTES: usize = 96 * 1024 * 1024;

/// Installs the bounded thumbnail pipeline before the generic egui texture loader.
pub fn install(ctx: &egui::Context) {
    if !ctx.is_loader_installed(ThumbnailLoader::ID) {
        ctx.add_texture_loader(Arc::new(ThumbnailLoader::new()));
    }
}

/// Stable URI for an image-grid thumbnail. Quantizing the edge keeps Ctrl+scroll
/// resizing from creating a separate GPU texture for every four-pixel step.
pub fn uri(
    path: &Path,
    requested_edge: f32,
    modified: Option<SystemTime>,
    file_size: Option<u64>,
) -> String {
    format!(
        "{URI_PREFIX}{}/{}/{}",
        bucket_edge(requested_edge),
        revision(modified, file_size),
        path.to_string_lossy()
    )
}

/// Queue a nearby thumbnail without painting it. Visible cells request their URIs
/// afterwards, so they always outrank this speculative work.
pub fn prefetch(
    ctx: &egui::Context,
    path: &Path,
    requested_edge: f32,
    modified: Option<SystemTime>,
    file_size: Option<u64>,
) {
    let _ = ctx.try_load_texture(
        &uri(path, requested_edge, modified, file_size),
        egui::TextureOptions::default(),
        SizeHint::default(),
    );
}

fn revision(modified: Option<SystemTime>, file_size: Option<u64>) -> String {
    let modified = modified
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    format!("{modified}-{}", file_size.unwrap_or(0))
}

fn bucket_edge(requested_edge: f32) -> u32 {
    const BUCKETS: [u32; 5] = [96, 128, 160, 192, 256];
    let edge = requested_edge.ceil().max(1.0) as u32;
    BUCKETS
        .into_iter()
        .find(|bucket| edge <= *bucket)
        .unwrap_or(256)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CacheKey {
    uri: String,
    texture_options: egui::TextureOptions,
}

#[derive(Clone)]
struct ThumbnailSpec {
    path: PathBuf,
    edge: u32,
}

enum CacheEntry {
    Queued {
        token: u64,
        last_used: u64,
        last_enqueued: u64,
    },
    Loading {
        token: u64,
        last_used: u64,
    },
    Decoded {
        last_used: u64,
        image: Arc<egui::ColorImage>,
    },
    Ready {
        last_used: u64,
        texture: egui::TextureHandle,
        size: egui::Vec2,
        bytes: usize,
    },
    Failed {
        last_used: u64,
        message: String,
    },
}

struct Job {
    priority: u64,
    token: u64,
    key: CacheKey,
    spec: ThumbnailSpec,
    ctx: egui::Context,
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl Eq for Job {}

impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Job {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
    }
}

#[derive(Default)]
struct State {
    cache: HashMap<CacheKey, CacheEntry>,
    jobs: BinaryHeap<Job>,
    next_token: u64,
    next_priority: u64,
}

impl State {
    fn enqueue(&mut self, key: CacheKey, spec: ThumbnailSpec, ctx: &egui::Context, pass: u64) {
        self.next_token = self.next_token.wrapping_add(1);
        self.next_priority = self.next_priority.wrapping_add(1);
        let token = self.next_token;
        self.cache.insert(
            key.clone(),
            CacheEntry::Queued {
                token,
                last_used: pass,
                last_enqueued: pass,
            },
        );
        self.jobs.push(Job {
            priority: self.next_priority,
            token,
            key,
            spec,
            ctx: ctx.clone(),
        });
    }

    fn next_job(&mut self) -> Option<Job> {
        while let Some(job) = self.jobs.pop() {
            let Some(CacheEntry::Queued {
                token, last_used, ..
            }) = self.cache.get(&job.key)
            else {
                continue;
            };
            if *token != job.token {
                continue;
            }
            let last_used = *last_used;
            self.cache.insert(
                job.key.clone(),
                CacheEntry::Loading {
                    token: job.token,
                    last_used,
                },
            );
            return Some(job);
        }
        None
    }

    fn prune(&mut self, pass: u64) {
        self.cache.retain(|_, entry| {
            !matches!(
                entry,
                CacheEntry::Queued { last_used, .. }
                    | CacheEntry::Loading { last_used, .. }
                    | CacheEntry::Decoded { last_used, .. }
                    | CacheEntry::Failed { last_used, .. }
                    if *last_used < pass
            )
        });
        self.jobs.retain(|job| {
            matches!(
                self.cache.get(&job.key),
                Some(CacheEntry::Queued { token, .. }) if *token == job.token
            )
        });

        let mut ready_count = 0;
        let mut ready_bytes = 0;
        let mut evictable = Vec::new();
        for (key, entry) in &self.cache {
            if let CacheEntry::Ready {
                last_used, bytes, ..
            } = entry
            {
                ready_count += 1;
                ready_bytes += bytes;
                if *last_used < pass {
                    evictable.push((*last_used, key.clone(), *bytes));
                }
            }
        }
        evictable.sort_unstable_by_key(|(last_used, _, _)| *last_used);
        for (_, key, bytes) in evictable {
            if ready_count <= MAX_READY_ITEMS && ready_bytes <= MAX_READY_BYTES {
                break;
            }
            if self.cache.remove(&key).is_some() {
                ready_count -= 1;
                ready_bytes = ready_bytes.saturating_sub(bytes);
            }
        }
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    work_ready: Condvar,
}

pub struct ThumbnailLoader {
    shared: Arc<Shared>,
    completed_pass: AtomicU64,
}

impl ThumbnailLoader {
    const ID: &'static str = egui::generate_loader_id!(ThumbnailLoader);

    fn new() -> Self {
        let shared = Arc::new(Shared::default());
        for index in 0..WORKER_COUNT {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name(format!("Atlas-thumbnail-{index}"))
                .spawn(move || worker_loop(&shared))
                .expect("failed to spawn thumbnail worker");
        }
        Self {
            shared,
            completed_pass: AtomicU64::new(0),
        }
    }

    fn usage_pass(&self) -> u64 {
        self.completed_pass.load(AtomicOrdering::Relaxed) + 1
    }
}

impl TextureLoader for ThumbnailLoader {
    fn id(&self) -> &str {
        Self::ID
    }

    fn load(
        &self,
        ctx: &egui::Context,
        uri: &str,
        texture_options: egui::TextureOptions,
        _size_hint: SizeHint,
    ) -> TextureLoadResult {
        let Some(spec) = parse_uri(uri) else {
            return Err(LoadError::NotSupported);
        };
        let key = CacheKey {
            uri: uri.to_owned(),
            texture_options,
        };
        let pass = self.usage_pass();
        let mut state = lock(&self.shared.state);

        match state.cache.remove(&key) {
            Some(CacheEntry::Ready {
                texture,
                size,
                bytes,
                ..
            }) => {
                let sized = SizedTexture::new(texture.id(), size);
                state.cache.insert(
                    key,
                    CacheEntry::Ready {
                        last_used: pass,
                        texture,
                        size,
                        bytes,
                    },
                );
                Ok(TexturePoll::Ready { texture: sized })
            }
            Some(CacheEntry::Decoded { image, .. }) => {
                drop(state);
                let size = egui::vec2(image.size[0] as f32, image.size[1] as f32);
                let bytes = image.pixels.len() * std::mem::size_of::<egui::Color32>();
                let texture = ctx.load_texture(uri, image, texture_options);
                let sized = SizedTexture::new(texture.id(), size);
                lock(&self.shared.state).cache.insert(
                    key,
                    CacheEntry::Ready {
                        last_used: pass,
                        texture,
                        size,
                        bytes,
                    },
                );
                Ok(TexturePoll::Ready { texture: sized })
            }
            Some(CacheEntry::Queued {
                token,
                last_used,
                last_enqueued,
            }) if last_enqueued == pass => {
                state.cache.insert(
                    key,
                    CacheEntry::Queued {
                        token,
                        last_used: pass.max(last_used),
                        last_enqueued,
                    },
                );
                Ok(TexturePoll::Pending { size: None })
            }
            Some(CacheEntry::Queued { .. }) => {
                state.enqueue(key, spec, ctx, pass);
                self.shared.work_ready.notify_all();
                Ok(TexturePoll::Pending { size: None })
            }
            Some(CacheEntry::Loading { token, .. }) => {
                state.cache.insert(
                    key,
                    CacheEntry::Loading {
                        token,
                        last_used: pass,
                    },
                );
                Ok(TexturePoll::Pending { size: None })
            }
            Some(CacheEntry::Failed { message, .. }) => {
                state.cache.insert(
                    key,
                    CacheEntry::Failed {
                        last_used: pass,
                        message: message.clone(),
                    },
                );
                Err(LoadError::Loading(message))
            }
            None => {
                state.enqueue(key, spec, ctx, pass);
                self.shared.work_ready.notify_all();
                Ok(TexturePoll::Pending { size: None })
            }
        }
    }

    fn forget(&self, uri: &str) {
        lock(&self.shared.state)
            .cache
            .retain(|key, _| key.uri != uri);
    }

    fn forget_all(&self) {
        let mut state = lock(&self.shared.state);
        state.cache.clear();
        state.jobs.clear();
    }

    fn end_pass(&self, pass_index: u64) {
        self.completed_pass
            .store(pass_index, AtomicOrdering::Relaxed);
        lock(&self.shared.state).prune(pass_index);
    }

    fn byte_size(&self) -> usize {
        lock(&self.shared.state)
            .cache
            .values()
            .map(|entry| match entry {
                CacheEntry::Decoded { image, .. } => {
                    image.pixels.len() * std::mem::size_of::<egui::Color32>()
                }
                CacheEntry::Ready { bytes, .. } => *bytes,
                _ => 0,
            })
            .sum()
    }
}

fn parse_uri(uri: &str) -> Option<ThumbnailSpec> {
    let (edge, tail) = uri.strip_prefix(URI_PREFIX)?.split_once('/')?;
    let (_, path) = tail.split_once('/')?;
    Some(ThumbnailSpec {
        path: PathBuf::from(path),
        edge: edge.parse().ok()?,
    })
}

fn worker_loop(shared: &Shared) {
    loop {
        let job = {
            let mut state = lock(&shared.state);
            loop {
                if state.jobs.is_empty() {
                    state = wait(&shared.work_ready, state);
                    continue;
                }
                let (next_state, timeout) = shared
                    .work_ready
                    .wait_timeout(state, QUEUE_DEBOUNCE)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state = next_state;
                if timeout.timed_out() {
                    if let Some(job) = state.next_job() {
                        break job;
                    }
                }
            }
        };

        let decoded = decode_thumbnail(&job.spec);
        let mut state = lock(&shared.state);
        let Some(CacheEntry::Loading { token, last_used }) = state.cache.get(&job.key) else {
            continue;
        };
        if *token != job.token {
            continue;
        }
        let last_used = *last_used;
        state.cache.insert(
            job.key,
            match decoded {
                Ok(image) => CacheEntry::Decoded {
                    last_used,
                    image: Arc::new(image),
                },
                Err(message) => CacheEntry::Failed { last_used, message },
            },
        );
        drop(state);
        job.ctx.request_repaint();
    }
}

fn decode_thumbnail(spec: &ThumbnailSpec) -> Result<egui::ColorImage, String> {
    let mut reader = image::ImageReader::open(&spec.path).map_err(|err| err.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader
        .decode()
        .map_err(|err| err.to_string())?
        .thumbnail(spec.edge, spec.edge)
        .to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn wait<'a, T>(condvar: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    condvar
        .wait(guard)
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_quantizes_sizes_and_round_trips_absolute_paths() {
        let uri = uri(Path::new("/tmp/photos/image.jpg"), 129.0, None, Some(42));
        assert_eq!(uri, "thumbnail://160/0-42//tmp/photos/image.jpg");
        let spec = parse_uri(&uri).expect("thumbnail uri");
        assert_eq!(spec.edge, 160);
        assert_eq!(spec.path, Path::new("/tmp/photos/image.jpg"));
    }

    #[test]
    fn uri_changes_when_file_metadata_changes() {
        let path = Path::new("/tmp/photos/image.jpg");
        let before = uri(
            path,
            96.0,
            Some(UNIX_EPOCH + Duration::from_secs(1)),
            Some(42),
        );
        let after = uri(
            path,
            96.0,
            Some(UNIX_EPOCH + Duration::from_secs(2)),
            Some(42),
        );
        assert_ne!(before, after);
    }

    #[test]
    fn newest_queued_thumbnail_runs_first() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        for name in ["old.jpg", "new.jpg"] {
            let uri = format!("thumbnail://96/0-0/{name}");
            state.enqueue(
                CacheKey {
                    uri,
                    texture_options: Default::default(),
                },
                ThumbnailSpec {
                    path: PathBuf::from(name),
                    edge: 96,
                },
                &ctx,
                1,
            );
        }
        assert_eq!(
            state.next_job().expect("queued job").spec.path,
            Path::new("new.jpg")
        );
    }

    #[test]
    fn stale_pending_work_is_pruned_after_viewport_moves() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        state.enqueue(
            CacheKey {
                uri: "thumbnail://96/0-0/old.jpg".to_owned(),
                texture_options: Default::default(),
            },
            ThumbnailSpec {
                path: PathBuf::from("old.jpg"),
                edge: 96,
            },
            &ctx,
            1,
        );
        state.prune(2);
        assert!(state.next_job().is_none());
        assert!(state.cache.is_empty());
    }

    #[test]
    fn decode_downsizes_before_upload() {
        let path =
            std::env::temp_dir().join(format!("Atlas-thumb-{}.png", uuid::Uuid::new_v4()));
        image::RgbaImage::new(400, 200)
            .save(&path)
            .expect("write test image");
        let decoded = decode_thumbnail(&ThumbnailSpec {
            path: path.clone(),
            edge: 96,
        })
        .expect("decode thumbnail");
        let _ = std::fs::remove_file(path);
        assert_eq!(decoded.size, [96, 48]);
    }
}
