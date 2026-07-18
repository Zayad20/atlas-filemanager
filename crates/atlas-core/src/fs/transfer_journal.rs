use super::transfer::TransferPair;
use atlas_model::ops::{TransferKind, TransferOrigin};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

const VERSION: u32 = 1;
const FILE_NAME: &str = "transfer-journal.json";
static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static RESUMING: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingTransfer {
    pub id: u64,
    pub kind: TransferKind,
    pub origin: TransferOrigin,
    pub pairs: Vec<TransferPair>,
    #[serde(default)]
    fingerprints: Vec<SourceFingerprint>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SourceFingerprint {
    path: std::path::PathBuf,
    size: u64,
    modified_ns: Option<u64>,
}

fn fingerprint(path: &std::path::Path) -> Option<SourceFingerprint> {
    let metadata = path.metadata().ok()?;
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(u128::from(u64::MAX)) as u64);
    Some(SourceFingerprint {
        path: path.to_path_buf(),
        size: metadata.len(),
        modified_ns,
    })
}

impl PendingTransfer {
    pub fn new(
        id: u64,
        kind: TransferKind,
        origin: TransferOrigin,
        pairs: Vec<TransferPair>,
    ) -> Self {
        let fingerprints = pairs
            .iter()
            .filter_map(|pair| fingerprint(&pair.src))
            .collect();
        Self {
            id,
            kind,
            origin,
            pairs,
            fingerprints,
        }
    }

    pub fn sources_unchanged(&self) -> bool {
        self.fingerprints.len() == self.pairs.len()
            && self
                .fingerprints
                .iter()
                .all(|saved| fingerprint(&saved.path).as_ref() == Some(saved))
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Journal {
    version: u32,
    transfers: Vec<PendingTransfer>,
}

fn path() -> std::path::PathBuf {
    crate::util::config::data_dir().join(FILE_NAME)
}

fn read() -> Journal {
    std::fs::read(path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .filter(|journal: &Journal| journal.version == VERSION)
        .unwrap_or(Journal {
            version: VERSION,
            transfers: Vec::new(),
        })
}

fn write(journal: &Journal) -> std::io::Result<()> {
    let path = path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(journal)?)?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    std::fs::rename(temporary, path)
}

pub fn upsert(transfer: PendingTransfer) {
    let _guard = LOCK.get_or_init(|| Mutex::new(())).lock().ok();
    let mut journal = read();
    journal.transfers.retain(|item| item.id != transfer.id);
    journal.transfers.push(transfer);
    if let Err(error) = write(&journal) {
        tracing::warn!(?error, "failed to persist transfer journal");
    }
}

pub fn remove(id: u64) {
    let _guard = LOCK.get_or_init(|| Mutex::new(())).lock().ok();
    let mut journal = read();
    journal.transfers.retain(|item| item.id != id);
    if let Err(error) = write(&journal) {
        tracing::warn!(?error, "failed to update transfer journal");
    }
}

pub fn load() -> Vec<PendingTransfer> {
    let _guard = LOCK.get_or_init(|| Mutex::new(())).lock().ok();
    read().transfers
}

pub fn mark_resuming(id: u64) {
    if let Ok(mut ids) = RESUMING.get_or_init(|| Mutex::new(HashSet::new())).lock() {
        ids.insert(id);
    }
}

pub fn take_resuming(id: u64) -> bool {
    RESUMING
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .is_ok_and(|mut ids| ids.remove(&id))
}
