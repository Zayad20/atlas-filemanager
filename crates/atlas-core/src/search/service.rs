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
