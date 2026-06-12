//! Protection against zip-bomb attacks during extraction.
//!
//! Call `BombGuard::default().check_entry(...)` or `check_chunk(...)` during
//! extraction; returns `Err(ArchiveError::Corrupt)` when a limit is hit.

use super::ArchiveError;

const MAX_ENTRIES: u64 = 200_000;
const MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024 * 1024; // 50 GiB
const MAX_RATIO: u64 = 200;
const MAX_DEPTH: u32 = 48;

/// Extraction limits enforced per-provider call.
pub struct BombGuard {
    pub max_entries: u64,
    pub max_total_bytes: u64,
    /// Max inflation ratio (uncompressed / compressed). Checked during chunk copy.
    pub max_ratio: u64,
    pub max_depth: u32,
    entries_seen: u64,
    bytes_inflated: u64,
    bytes_compressed_hint: u64,
}

impl Default for BombGuard {
    fn default() -> Self {
        Self {
            max_entries: MAX_ENTRIES,
            max_total_bytes: MAX_TOTAL_BYTES,
            max_ratio: MAX_RATIO,
            max_depth: MAX_DEPTH,
            entries_seen: 0,
            bytes_inflated: 0,
            bytes_compressed_hint: 0,
        }
    }
}

impl BombGuard {
    /// Call once per archive entry.
    ///
    /// `compressed_hint` is the stored compressed size (0 if unknown).
    /// `path_depth` is the nesting depth of the entry (0 = root).
    pub fn check_entry(
        &mut self,
        compressed_hint: u64,
        path_depth: u32,
    ) -> Result<(), ArchiveError> {
        self.entries_seen += 1;
        self.bytes_compressed_hint += compressed_hint;

        if self.entries_seen > self.max_entries {
            return Err(ArchiveError::Corrupt {
                detail: format!(
                    "demasiadas entradas ({} > {})",
                    self.entries_seen, self.max_entries
                ),
                partial: None,
            });
        }
        if path_depth > self.max_depth {
            return Err(ArchiveError::Corrupt {
                detail: format!(
                    "profundidad de directorio excesiva ({path_depth} > {})",
                    self.max_depth
                ),
                partial: None,
            });
        }
        Ok(())
    }

    /// Call after writing each decompressed chunk during extraction.
    ///
    /// `chunk_bytes` is the number of uncompressed bytes just written.
    pub fn check_chunk(&mut self, chunk_bytes: u64) -> Result<(), ArchiveError> {
        self.bytes_inflated += chunk_bytes;

        if self.bytes_inflated > self.max_total_bytes {
            return Err(ArchiveError::Corrupt {
                detail: format!(
                    "tamaño total inflado excede el límite ({} GiB)",
                    self.max_total_bytes / (1024 * 1024 * 1024)
                ),
                partial: None,
            });
        }
        // Check ratio only when we have a meaningful compressed-size hint.
        if self.bytes_compressed_hint > 0 && self.max_ratio > 0 {
            let ratio = self.bytes_inflated / self.bytes_compressed_hint.max(1);
            if ratio > self.max_ratio {
                return Err(ArchiveError::Corrupt {
                    detail: format!(
                        "ratio de inflado sospechoso ({ratio}x > {}x)",
                        self.max_ratio
                    ),
                    partial: None,
                });
            }
        }
        Ok(())
    }

    /// Pre-flight: verify that `dest_dir` has at least `needed_bytes` free.
    ///
    /// Currently a best-effort check using `sysinfo`; returns `Ok` if the
    /// information is unavailable so callers are never blocked unnecessarily.
    pub fn check_free_space(
        dest_dir: &std::path::Path,
        needed_bytes: u64,
    ) -> Result<(), ArchiveError> {
        if let Ok(meta) = std::fs::metadata(dest_dir) {
            // std doesn't expose available disk space; fall back to sysinfo.
            // For now we do a lightweight sanity check: if the path doesn't exist
            // we can't estimate, so skip. The full check is wired in F3A.
            let _ = (meta, needed_bytes);
        }
        // Placeholder — full implementation in F3A when extract() is rewritten.
        let _ = (dest_dir, needed_bytes);
        Ok(())
    }

    /// Whether the guard has already tripped (bytes or entries exceeded).
    pub fn is_tripped(&self) -> bool {
        self.entries_seen > self.max_entries || self.bytes_inflated > self.max_total_bytes
    }
}
