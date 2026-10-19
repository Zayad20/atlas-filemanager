//! Reproducible heap-request comparison; no filesystem or UI is involved.
//! Run: cargo run -p atlas-model --example file_entry_memory -- <output.json>
#![allow(unsafe_code, dead_code)] // allocator instrumentation only, outside production

use atlas_model::file_entry::{FileEntry, FileKind, RecycleBinInfo};
use serde::Serialize;
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;
use std::time::SystemTime;

struct CountingAllocator;
static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static LIVE_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOC_CALLS: AtomicUsize = AtomicUsize::new(0);
static REALLOC_CALLS: AtomicUsize = AtomicUsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);

fn account_alloc(bytes: usize) {
    let live = LIVE_BYTES.fetch_add(bytes, Relaxed) + bytes;
    LIVE_COUNT.fetch_add(1, Relaxed);
    ALLOCATED_BYTES.fetch_add(bytes, Relaxed);
    ALLOC_CALLS.fetch_add(1, Relaxed);
    PEAK_BYTES.fetch_max(live, Relaxed);
}

// SAFETY: every operation forwards unchanged pointers/layouts to System. The
// instrumentation consists exclusively of allocation-free atomic operations.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            account_alloc(layout.size());
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            account_alloc(layout.size());
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE_BYTES.fetch_sub(layout.size(), Relaxed);
        LIVE_COUNT.fetch_sub(1, Relaxed);
        unsafe { System.dealloc(ptr, layout) };
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(ptr, layout, new_size) };
        if !result.is_null() {
            let live = if new_size >= layout.size() {
                LIVE_BYTES.fetch_add(new_size - layout.size(), Relaxed) + new_size - layout.size()
            } else {
                LIVE_BYTES.fetch_sub(layout.size() - new_size, Relaxed) - (layout.size() - new_size)
            };
            ALLOCATED_BYTES.fetch_add(new_size, Relaxed);
            REALLOC_CALLS.fetch_add(1, Relaxed);
            PEAK_BYTES.fetch_max(live, Relaxed);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
const COUNT: usize = 100_000;

/// Exact historical field types, including PathBuf and the unchanged payloads.
struct LegacyFileEntry {
    name: String,
    name_lower: String,
    path: PathBuf,
    kind: FileKind,
    size: Option<u64>,
    modified: Option<SystemTime>,
    extension: Option<String>,
    display_size: Arc<str>,
    display_date: Arc<str>,
    display_type: Arc<str>,
    _is_hidden: bool,
    _is_symlink: bool,
    archive_root: Option<PathBuf>,
    recycle_info: Option<RecycleBinInfo>,
}

#[derive(Clone, Copy)]
enum Names {
    Short,
    Mixed,
    Long,
}

impl Names {
    fn name(self, index: usize) -> String {
        match self {
            Self::Short => format!("file{index:06}.txt"),
            Self::Long => {
                format!("quarterly-financial-projection-and-analysis-report-number-{index:06}.txt")
            }
            Self::Mixed => match index % 5 {
                0..=2 => Self::Short.name(index),
                3 => Self::Long.name(index),
                _ => format!("café-{index:06}.txt"),
            },
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Short => "short_ascii",
            Self::Mixed => "mixed_80_percent_inline",
            Self::Long => "long_ascii",
        }
    }
}

#[derive(Clone)]
struct Displays {
    size: Arc<str>,
    date: Arc<str>,
    kind: Arc<str>,
}
impl Displays {
    fn new() -> Self {
        Self {
            size: "4.0 KiB".into(),
            date: "2026-10-15 12:00".into(),
            kind: "Text document".into(),
        }
    }
}

#[derive(Clone, Copy)]
struct Snapshot {
    live_bytes: usize,
    live_count: usize,
    allocated_bytes: usize,
    alloc_calls: usize,
    realloc_calls: usize,
}
impl Snapshot {
    fn read() -> Self {
        Self {
            live_bytes: LIVE_BYTES.load(Relaxed),
            live_count: LIVE_COUNT.load(Relaxed),
            allocated_bytes: ALLOCATED_BYTES.load(Relaxed),
            alloc_calls: ALLOC_CALLS.load(Relaxed),
            realloc_calls: REALLOC_CALLS.load(Relaxed),
        }
    }
}

#[derive(Serialize)]
struct Measurement {
    retained_heap_bytes: usize,
    retained_allocations: usize,
    peak_heap_bytes: usize,
    cumulative_requested_bytes: usize,
    allocation_calls: usize,
    reallocation_calls: usize,
    entry_struct_bytes: usize,
    inline_names: usize,
    semantic_checksum: u64,
}

fn measure<T>(
    make: impl FnOnce() -> Vec<T>,
    checksum: impl FnOnce(&[T]) -> (u64, usize),
) -> Measurement {
    let before = Snapshot::read();
    PEAK_BYTES.store(before.live_bytes, Relaxed);
    let entries = make();
    black_box(&entries);
    let after = Snapshot::read();
    let peak = PEAK_BYTES.load(Relaxed);
    let (semantic_checksum, inline_names) = checksum(&entries);
    let result = Measurement {
        retained_heap_bytes: after.live_bytes - before.live_bytes,
        retained_allocations: after.live_count - before.live_count,
        peak_heap_bytes: peak - before.live_bytes,
        cumulative_requested_bytes: after.allocated_bytes - before.allocated_bytes,
        allocation_calls: after.alloc_calls - before.alloc_calls,
        reallocation_calls: after.realloc_calls - before.realloc_calls,
        entry_struct_bytes: std::mem::size_of::<T>(),
        inline_names,
        semantic_checksum,
    };
    drop(entries);
    assert_eq!(
        LIVE_BYTES.load(Relaxed),
        before.live_bytes,
        "fixture leaked memory"
    );
    result
}

fn hash_fields(hash: &mut u64, fields: [&str; 7]) {
    for field in fields {
        for byte in field.bytes().chain([0]) {
            *hash = (*hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211);
        }
    }
}

#[derive(Serialize)]
struct Scenario {
    filenames: &'static str,
    display_storage: &'static str,
    entries: usize,
    legacy: Measurement,
    current: Measurement,
    retained_heap_reduction_percent: f64,
    retained_allocation_reduction_percent: f64,
}

fn scenario(names: Names, shared_displays: bool) -> Scenario {
    let displays = Displays::new();
    let legacy = measure(
        || {
            let mut entries = Vec::with_capacity(COUNT);
            for index in 0..COUNT {
                let name = names.name(index);
                let path = PathBuf::from(format!("C:\\AtlasMemoryBench\\{name}"));
                let display = if shared_displays {
                    displays.clone()
                } else {
                    Displays::new()
                };
                entries.push(LegacyFileEntry {
                    name_lower: name.to_lowercase(),
                    name,
                    path,
                    kind: FileKind::File,
                    size: Some(4096),
                    modified: None,
                    extension: Some("txt".to_owned()),
                    display_size: display.size,
                    display_date: display.date,
                    display_type: display.kind,
                    _is_hidden: false,
                    _is_symlink: false,
                    archive_root: None,
                    recycle_info: None,
                });
            }
            entries
        },
        |entries| {
            let mut checksum = 14_695_981_039_346_656_037;
            for entry in entries {
                hash_fields(
                    &mut checksum,
                    [
                        &entry.name,
                        &entry.name_lower,
                        entry.path.to_str().unwrap(),
                        entry.extension.as_deref().unwrap(),
                        &format!("{:?}", entry.size),
                        &format!("{:?}", entry.modified),
                        &format!("{:?}", entry.kind),
                    ],
                );
            }
            (checksum, 0)
        },
    );
    let current = measure(
        || {
            let directory =
                atlas_model::file_entry::EntryDirectory::new(PathBuf::from("C:\\AtlasMemoryBench"));
            let mut entries = Vec::with_capacity(COUNT);
            for index in 0..COUNT {
                let name = names.name(index);
                let path = PathBuf::from(format!("C:\\AtlasMemoryBench\\{name}"));
                entries.push(
                    FileEntry::in_directory(
                        atlas_model::file_entry::FileEntryData {
                            name: name.into(),
                            path,
                            kind: FileKind::File,
                            size: Some(4096),
                            modified: None,
                            _is_hidden: false,
                            _is_symlink: false,
                            archive_root: None,
                            recycle_info: None,
                        },
                        directory.clone(),
                    )
                    .with_native_metadata(0, index as u128 + 1),
                );
            }
            entries
        },
        |entries| {
            let mut checksum = 14_695_981_039_346_656_037;
            let mut inline = 0;
            for entry in entries {
                hash_fields(
                    &mut checksum,
                    [
                        &entry.name,
                        &entry.name_lower(),
                        entry.path().to_str().unwrap(),
                        entry.extension().as_deref().unwrap(),
                        &format!("{:?}", entry.size()),
                        &format!("{:?}", entry.modified),
                        &format!("{:?}", entry.kind()),
                    ],
                );
                inline += usize::from(!entry.name.is_heap_allocated());
            }
            (checksum, inline)
        },
    );
    assert_eq!(legacy.semantic_checksum, current.semantic_checksum);
    let reduction = |old: usize, new: usize| 100.0 * (1.0 - new as f64 / old as f64);
    Scenario {
        filenames: names.label(),
        display_storage: if shared_displays {
            "shared_3_arcs"
        } else {
            "per_entry_3_arcs"
        },
        entries: COUNT,
        retained_heap_reduction_percent: reduction(
            legacy.retained_heap_bytes,
            current.retained_heap_bytes,
        ),
        retained_allocation_reduction_percent: reduction(
            legacy.retained_allocations,
            current.retained_allocations,
        ),
        legacy,
        current,
    }
}

#[derive(Serialize)]
struct Report {
    metric: &'static str,
    target: &'static str,
    pointer_width: u32,
    scopes_excluded: &'static str,
    overall_ram_60_percent_proven: bool,
    retained_entry_heap_60_percent_proven: bool,
    scenarios: Vec<Scenario>,
}

fn main() {
    let scenarios: Vec<Scenario> = [false, true]
        .into_iter()
        .flat_map(|shared| {
            [Names::Short, Names::Mixed, Names::Long].map(|names| scenario(names, shared))
        })
        .collect();
    let retained_entry_heap_60_percent_proven = scenarios
        .iter()
        .all(|scenario| scenario.retained_heap_reduction_percent >= 60.0);
    assert!(std::mem::size_of::<FileEntry>() <= 64);
    assert!(
        retained_entry_heap_60_percent_proven,
        "entry heap reduction must remain at least 60% in every scenario"
    );
    let report = Report {
        metric: "GlobalAlloc requested live heap bytes/count after construction; Vec backing storage included; allocator overhead excluded",
        target: std::env::consts::ARCH, pointer_width: usize::BITS,
        scopes_excluded: "Process working set/RSS, allocator bookkeeping/fragmentation, UI/GPUI/thumbnail caches, filesystem listing and grouping caches",
        overall_ram_60_percent_proven: false, retained_entry_heap_60_percent_proven, scenarios,
    };
    let json = serde_json::to_string_pretty(&report).unwrap();
    if let Some(output) = std::env::args().nth(1) {
        std::fs::write(output, &json).unwrap();
    }
    println!("{json}");
}
