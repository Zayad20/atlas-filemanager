//! Versioned IPC contract and the Atlas Search Service client backend.

use super::{SearchBackend, SearchBackendStatus, SearchBatch, SearchProgress, SearchQuery};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const PIPE_PATH: &str = r"\\.\pipe\AtlasSearchService-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ServiceRequest {
    Hello { protocol: u32 },
    Search(SearchQuery),
    Status,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ServiceResponse {
    Hello { protocol: u32, schema: u32 },
    Search(SearchBatch),
    Status(Vec<SearchProgress>),
    Error(String),
}

pub struct WindowsServiceBackend {
    id: String,
}

impl WindowsServiceBackend {
    pub fn new() -> Self {
        Self {
            id: "atlas-search-service".into(),
        }
    }

    #[cfg(windows)]
    pub fn request(&self, request: &ServiceRequest) -> std::io::Result<ServiceResponse> {
        use std::ffi::OsStr;
        let payload = serde_json::to_vec(request).map_err(std::io::Error::other)?;
        let response =
            atlas_win_space::call_named_pipe(OsStr::new(PIPE_PATH), &payload, 512 * 1024)?;
        serde_json::from_slice(&response).map_err(std::io::Error::other)
    }

    #[cfg(not(windows))]
    pub fn request(&self, _request: &ServiceRequest) -> std::io::Result<ServiceResponse> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Windows service unavailable",
        ))
    }
}

impl Default for WindowsServiceBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchBackend for WindowsServiceBackend {
    fn id(&self) -> &str {
        &self.id
    }
    fn status(&self) -> SearchProgress {
        match self.request(&ServiceRequest::Status) {
            Ok(ServiceResponse::Status(statuses)) => {
                statuses.into_iter().next().unwrap_or(SearchProgress {
                    source: self.id.clone(),
                    status: SearchBackendStatus::Loading,
                    indexed: 0,
                    total: None,
                })
            }
            Ok(ServiceResponse::Error(error)) => SearchProgress {
                source: self.id.clone(),
                status: SearchBackendStatus::Error(error),
                indexed: 0,
                total: None,
            },
            _ => SearchProgress {
                source: self.id.clone(),
                status: SearchBackendStatus::Fallback,
                indexed: 0,
                total: None,
            },
        }
    }
    fn search(&self, query: &SearchQuery) -> SearchBatch {
        match self.request(&ServiceRequest::Search(query.clone())) {
            Ok(ServiceResponse::Search(batch)) => batch,
            _ => SearchBatch {
                generation: query.generation,
                results: Vec::new(),
                complete: true,
            },
        }
    }
}

/// Check if the Windows search service pipe is currently reachable and responding.
pub fn is_service_available() -> bool {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        let payload = match serde_json::to_vec(&ServiceRequest::Status) {
            Ok(p) => p,
            Err(_) => return false,
        };
        match atlas_win_space::call_named_pipe(OsStr::new(PIPE_PATH), &payload, 16 * 1024) {
            Ok(bytes) => serde_json::from_slice::<ServiceResponse>(&bytes).is_ok(),
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    false
}

/// Ensure the search service is running:
/// 1. If already responding, returns `true`.
/// 2. Attempts to start the installed Windows service (`sc.exe start AtlasSearchService`).
/// 3. If still not responding, locates `atlas-search-service.exe` and spawns it with `--console` (hidden background process).
pub fn ensure_service_started() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        if is_service_available() {
            return true;
        }

        // Try starting the installed Windows service first
        let _ = std::process::Command::new("sc.exe")
            .args(["start", "AtlasSearchService"])
            .creation_flags(CREATE_NO_WINDOW)
            .status();

        for _ in 0..6 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if is_service_available() {
                return true;
            }
        }

        // If not running, find the binary and spawn it with --console in the background
        if let Some(exe_path) = find_search_service_exe() {
            let _ = std::process::Command::new(exe_path)
                .arg("--console")
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();

            for _ in 0..10 {
                std::thread::sleep(std::time::Duration::from_millis(50));
                if is_service_available() {
                    return true;
                }
            }
        }

        is_service_available()
    }
    #[cfg(not(windows))]
    false
}

/// Locate the `atlas-search-service.exe` binary.
pub fn find_search_service_exe() -> Option<std::path::PathBuf> {
    if let Ok(current) = std::env::current_exe() {
        let mut cur = current.parent();
        for _ in 0..4 {
            if let Some(dir) = cur {
                let direct = dir.join("atlas-search-service.exe");
                if direct.exists() {
                    return Some(direct);
                }
                let in_release = dir.join("release").join("atlas-search-service.exe");
                if in_release.exists() {
                    return Some(in_release);
                }
                let in_debug = dir.join("debug").join("atlas-search-service.exe");
                if in_debug.exists() {
                    return Some(in_debug);
                }
                cur = dir.parent();
            } else {
                break;
            }
        }
    }
    let candidates = [
        "atlas-search-service.exe",
        "target/release/atlas-search-service.exe",
        "target/debug/atlas-search-service.exe",
    ];
    for candidate in candidates {
        let path = std::path::Path::new(candidate);
        if path.exists() {
            return Some(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_search_service_exe() {
        let exe = find_search_service_exe();
        assert!(exe.is_some(), "should locate atlas-search-service.exe in workspace target");
    }

    #[test]
    fn test_ensure_service_and_multi_drive_search() {
        let started = ensure_service_started();
        assert!(started, "search service should start and be available");
        assert!(is_service_available(), "pipe should be responsive");

        let backend = WindowsServiceBackend::new();
        let query = SearchQuery::text("windows");
        let batch = backend.search(&query);
        assert!(!batch.results.is_empty(), "search for 'windows' should return results");

        let drive_letters: std::collections::HashSet<char> = batch
            .results
            .iter()
            .filter_map(|r| r.path.to_str()?.chars().next())
            .collect();
        println!("Drives found in search results: {:?}", drive_letters);
        assert!(drive_letters.contains(&'C') || drive_letters.contains(&'F') || drive_letters.contains(&'E'));
    }
}
