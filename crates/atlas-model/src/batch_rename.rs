#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Per-file verdict for the current previews, used to flag rows in the UI and to
/// block an unsafe apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameIssue {
    /// Safe to apply (may be a no-op if the name is unchanged).
    Ok,
    /// The new name is empty, `.`/`..`, or contains a path separator.
    Invalid,
    /// Two outputs land on the same path, or the target already exists on disk and is
    /// not itself one of the files being renamed.
    Collision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameMode {
    Find,
    Prefix,
    Suffix,
    Number,
}

/// Whether a proposed new name is a usable filename on Linux/macOS/Windows. We
/// reject path separators (which would move the file) and the `.`/`..` specials.
pub fn name_is_valid(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\\')
}

impl std::fmt::Display for RenameMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Find => write!(f, "Find & Replace"),
            Self::Prefix => write!(f, "Add Prefix"),
            Self::Suffix => write!(f, "Add Suffix"),
            Self::Number => write!(f, "Number Files"),
        }
    }
}

pub struct BatchRenameState {
    pub files: Vec<PathBuf>,
    pub mode: RenameMode,
    pub pattern: String,
    pub replacement: String,
    pub previews: Vec<String>, // new filenames
}

impl BatchRenameState {
    pub fn new(files: Vec<PathBuf>) -> Self {
        let mut s = Self {
            files,
            mode: RenameMode::Find,
            pattern: String::new(),
            replacement: String::new(),
            previews: Vec::new(),
        };
        s.update_previews();
        s
    }

    pub fn update_previews(&mut self) {
        self.previews = self
            .files
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let ext = path
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                let name = format!("{stem}{ext}");

                match &self.mode {
                    RenameMode::Find => {
                        if self.pattern.is_empty() {
                            name
                        } else {
                            name.replace(&self.pattern, &self.replacement)
                        }
                    }
                    RenameMode::Prefix => format!("{}{}", self.pattern, name),
                    RenameMode::Suffix => format!("{}{}{}", stem, self.pattern, ext),
                    RenameMode::Number => {
                        format!("{:03}{}{}{}", i + 1, self.replacement, stem, ext)
                    }
                }
            })
            .collect();
    }

    /// Resolve the absolute target path for preview `i` (its parent dir + new name).
    fn target(&self, i: usize) -> Option<PathBuf> {
        let parent = self.files.get(i)?.parent()?;
        Some(parent.join(self.previews.get(i)?))
    }

    /// Classify every file: invalid name, colliding target, or ok. Aligned with
    /// `previews`. Collisions cover both duplicate outputs and pre-existing files on
    /// disk that aren't part of this batch.
    pub fn issues(&self) -> Vec<RenameIssue> {
        let sources: HashSet<&PathBuf> = self.files.iter().collect();
        // How many outputs map to each target path (to catch duplicate destinations).
        let mut target_counts: HashMap<PathBuf, usize> = HashMap::new();
        for i in 0..self.files.len() {
            if let Some(t) = self.target(i) {
                *target_counts.entry(t).or_insert(0) += 1;
            }
        }

        (0..self.files.len())
            .map(|i| {
                let name = self.previews.get(i).map(String::as_str).unwrap_or_default();
                if !name_is_valid(name) {
                    return RenameIssue::Invalid;
                }
                let Some(target) = self.target(i) else {
                    return RenameIssue::Invalid;
                };
                // Unchanged name is always fine (no-op).
                if Some(&target) == self.files.get(i) {
                    return RenameIssue::Ok;
                }
                if target_counts.get(&target).copied().unwrap_or(0) > 1 {
                    return RenameIssue::Collision;
                }
                // A pre-existing file at the target that isn't being renamed away.
                if target.exists() && !sources.contains(&target) {
                    return RenameIssue::Collision;
                }
                RenameIssue::Ok
            })
            .collect()
    }

    /// True when at least one file has a blocking problem (invalid name or collision),
    /// so the UI can refuse to apply rather than clobber data.
    pub fn has_blocking_issues(&self) -> bool {
        self.issues()
            .iter()
            .any(|v| matches!(v, RenameIssue::Invalid | RenameIssue::Collision))
    }
}
