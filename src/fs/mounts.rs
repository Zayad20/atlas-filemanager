use std::path::PathBuf;
use std::time::{Duration, Instant};

thread_local! {
    static MOUNT_CACHE: std::cell::RefCell<Option<(Instant, Vec<PathBuf>)>> = const { std::cell::RefCell::new(None) };
}

pub fn read_mounts() -> Vec<PathBuf> {
    MOUNT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((last_update, mounts)) = &*cache {
            if last_update.elapsed() < Duration::from_secs(5) {
                return mounts.clone();
            }
        }

        let new_mounts = read_mounts_internal();
        *cache = Some((Instant::now(), new_mounts.clone()));
        new_mounts
    })
}

/// Linux: parse `/proc/mounts` and keep only real on-disk filesystems (skipping the
/// many virtual/pseudo mounts the kernel exposes).
#[cfg(target_os = "linux")]
fn read_mounts_internal() -> Vec<PathBuf> {
    let content = match std::fs::read_to_string("/proc/mounts") {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let real_fstypes = [
        "ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "jfs", "reiserfs", "ntfs", "ntfs-3g",
        "vfat", "exfat", "fuseblk", "zfs",
    ];
    content
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let _dev = parts.next()?;
            let mountpoint = parts.next()?;
            let fstype = parts.next()?;
            real_fstypes
                .contains(&fstype)
                .then(|| PathBuf::from(mountpoint))
        })
        .collect()
}

/// Windows / macOS: enumerate mounted volumes via `sysinfo` (already a dependency and
/// cross-platform). On Windows these are drive roots like `C:\`; on macOS they are the
/// root `/` plus `/Volumes/*` for external disks.
#[cfg(not(target_os = "linux"))]
fn read_mounts_internal() -> Vec<PathBuf> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut mounts: Vec<PathBuf> = disks
        .list()
        .iter()
        .map(|disk| disk.mount_point().to_path_buf())
        .collect();
    mounts.sort();
    mounts.dedup();
    mounts
}
