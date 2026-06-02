use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use jwalk::{Parallelism, WalkDir};

const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(80);
const MAX_SCAN_THREADS: usize = 4;

#[derive(Clone, Debug)]
pub struct SpaceItem {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Clone, Debug)]
struct ScanSnapshot {
    scan_id: u64,
    items: Vec<SpaceItem>,
    scanned_bytes: u64,
    scanned_entries: u64,
    error_count: u64,
    finished: bool,
}

#[derive(Clone)]
pub struct SpaceLensState {
    pub active_path: Option<PathBuf>,
    pub items: Vec<SpaceItem>,
    pub selected_for_deletion: HashSet<PathBuf>,
    pub loading: bool,
    pub physics_nodes: HashMap<PathBuf, PhysicsNode>,
    pub bubble_pointer_down: bool,
    pub scanned_bytes: u64,
    pub scanned_entries: u64,
    pub error_count: u64,
    active_scan_id: u64,
    scan_generation: Arc<AtomicU64>,
    rx: flume::Receiver<ScanSnapshot>,
    tx: flume::Sender<ScanSnapshot>,
}

#[derive(Clone, Debug)]
pub struct PhysicsNode {
    pub pos: egui::Pos2,
    pub vel: egui::Vec2,
    pub radius: f32,
    pub target_radius: f32,
    pub color: egui::Color32,
}

impl Default for SpaceLensState {
    fn default() -> Self {
        let (tx, rx) = flume::unbounded();
        Self {
            active_path: None,
            items: Vec::new(),
            selected_for_deletion: HashSet::new(),
            loading: false,
            physics_nodes: HashMap::new(),
            bubble_pointer_down: false,
            scanned_bytes: 0,
            scanned_entries: 0,
            error_count: 0,
            active_scan_id: 0,
            scan_generation: Arc::new(AtomicU64::new(0)),
            rx,
            tx,
        }
    }
}

impl SpaceLensState {
    pub fn update(&mut self) {
        while let Ok(snapshot) = self.rx.try_recv() {
            if snapshot.scan_id != self.active_scan_id {
                continue;
            }
            self.items = snapshot.items;
            self.scanned_bytes = snapshot.scanned_bytes;
            self.scanned_entries = snapshot.scanned_entries;
            self.error_count = snapshot.error_count;
            self.loading = !snapshot.finished;
            self.physics_nodes
                .retain(|path, _| self.items.iter().any(|item| item.path == *path));
        }
    }

    pub fn open_for_path(&mut self, path: PathBuf) {
        let scan_id = self.scan_generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.active_scan_id = scan_id;
        self.active_path = Some(path.clone());
        self.items.clear();
        self.selected_for_deletion.clear();
        self.physics_nodes.clear();
        self.bubble_pointer_down = false;
        self.loading = true;
        self.scanned_bytes = 0;
        self.scanned_entries = 0;
        self.error_count = 0;

        let tx = self.tx.clone();
        let scan_generation = self.scan_generation.clone();
        std::thread::spawn(move || scan_path(path, scan_id, scan_generation, tx));
    }

    pub fn close(&mut self) {
        self.scan_generation.fetch_add(1, Ordering::Relaxed);
        self.active_path = None;
        self.items.clear();
        self.selected_for_deletion.clear();
        self.physics_nodes.clear();
        self.bubble_pointer_down = false;
        self.loading = false;
        self.scanned_bytes = 0;
        self.scanned_entries = 0;
        self.error_count = 0;
    }
}

fn scan_path(
    root: PathBuf,
    scan_id: u64,
    scan_generation: Arc<AtomicU64>,
    tx: flume::Sender<ScanSnapshot>,
) {
    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(MAX_SCAN_THREADS);
    let filter_root = root.clone();
    let walker = WalkDir::new(&root)
        .min_depth(1)
        .skip_hidden(false)
        .follow_links(false)
        .process_read_dir(move |_, _, _, children| {
            for entry in children.iter_mut().filter_map(|entry| entry.as_mut().ok()) {
                if should_skip_descendants(&filter_root, &entry.path()) {
                    entry.read_children_path = None;
                }
            }
        })
        .parallelism(Parallelism::RayonNewPool(threads));

    let mut items = HashMap::<PathBuf, SpaceItem>::new();
    let mut scanned_bytes = 0_u64;
    let mut scanned_entries = 0_u64;
    let mut error_count = 0_u64;
    let mut last_snapshot = Instant::now();

    for result in walker {
        if scan_generation.load(Ordering::Relaxed) != scan_id {
            return;
        }

        let entry = match result {
            Ok(entry) => entry,
            Err(_) => {
                error_count += 1;
                continue;
            }
        };
        let path = entry.path();
        let Some(child_path) = top_level_child(&root, &path) else {
            continue;
        };
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => {
                error_count += 1;
                continue;
            }
        };
        let size = metadata.len();

        scanned_bytes = scanned_bytes.saturating_add(size);
        scanned_entries += 1;
        let item = items
            .entry(child_path.clone())
            .or_insert_with(|| SpaceItem {
                name: child_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path: child_path,
                is_dir: entry.depth() == 1 && entry.file_type().is_dir(),
                size: 0,
            });
        item.size = item.size.saturating_add(size);

        if last_snapshot.elapsed() >= SNAPSHOT_INTERVAL {
            if !send_snapshot(
                &tx,
                scan_id,
                &items,
                scanned_bytes,
                scanned_entries,
                error_count,
                false,
            ) {
                return;
            }
            last_snapshot = Instant::now();
        }
    }

    if scan_generation.load(Ordering::Relaxed) == scan_id {
        let _ = send_snapshot(
            &tx,
            scan_id,
            &items,
            scanned_bytes,
            scanned_entries,
            error_count,
            true,
        );
    }
}

fn send_snapshot(
    tx: &flume::Sender<ScanSnapshot>,
    scan_id: u64,
    items: &HashMap<PathBuf, SpaceItem>,
    scanned_bytes: u64,
    scanned_entries: u64,
    error_count: u64,
    finished: bool,
) -> bool {
    let mut items: Vec<_> = items.values().cloned().collect();
    items.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    tx.send(ScanSnapshot {
        scan_id,
        items,
        scanned_bytes,
        scanned_entries,
        error_count,
        finished,
    })
    .is_ok()
}

fn top_level_child(root: &Path, path: &Path) -> Option<PathBuf> {
    path.strip_prefix(root)
        .ok()?
        .components()
        .next()
        .map(|component| root.join(component.as_os_str()))
}

fn should_skip_descendants(root: &Path, path: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        root == Path::new("/")
            && path.parent() == Some(root)
            && matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some("dev" | "proc" | "run" | "sys")
            )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, path);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{should_skip_descendants, top_level_child};
    use std::path::{Path, PathBuf};

    #[test]
    fn finds_immediate_child_for_nested_paths() {
        assert_eq!(
            top_level_child(
                Path::new("/home/user"),
                Path::new("/home/user/projects/app/src")
            ),
            Some(PathBuf::from("/home/user/projects"))
        );
    }

    #[test]
    fn rejects_paths_outside_root() {
        assert_eq!(
            top_level_child(Path::new("/home/user"), Path::new("/tmp/file")),
            None
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn skips_linux_virtual_mounts_only_when_scanning_root() {
        assert!(should_skip_descendants(Path::new("/"), Path::new("/proc")));
        assert!(!should_skip_descendants(Path::new("/"), Path::new("/home")));
        assert!(!should_skip_descendants(
            Path::new("/proc"),
            Path::new("/proc/self")
        ));
    }
}
