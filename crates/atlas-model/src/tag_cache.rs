#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
//! In-memory, UI-facing view of the tag system.
//!
//! All persistence lives in [`crate::fs::tag_db`] (`SQLite`). This cache is what the
//! sidebar and inspector read every frame, so it must never touch SQL. It is kept in
//! sync by the background tag-db worker via `BgResult::{TagDefinitions, TagsForPaths}`
//! and updated optimistically on assign/unassign so chips appear the same frame.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Stable identity of a tag, matching the `tags.id` primary key in `SQLite`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TagId(pub i64);

/// A tag definition (name + color + scope), mirrored from the `tags` table.
#[derive(Debug, Clone)]
pub struct TagDef {
    pub id: TagId,
    pub name: String,
    pub color: [u8; 3],
    /// `None` = global tag (visible in every profile); `Some(name)` = profile-local.
    pub profile: Option<String>,
}

#[derive(Default)]
pub struct TagCache {
    /// Every tag definition across all profiles + globals.
    pub defs: Vec<TagDef>,
    /// Cached associations for the paths the UI has asked about (selection/dir).
    /// A present-but-empty entry means "known to have no tags" (avoids re-querying).
    pub for_path: HashMap<PathBuf, Vec<TagId>>,
}

impl TagCache {
    pub fn def(&self, id: TagId) -> Option<&TagDef> {
        self.defs.iter().find(|d| d.id == id)
    }

    pub fn tags_of(&self, path: &Path) -> &[TagId] {
        self.for_path.get(path).map_or(&[], Vec::as_slice)
    }

    /// True once we have a (possibly empty) cached answer for `path`.
    pub fn knows(&self, path: &Path) -> bool {
        self.for_path.contains_key(path)
    }

    pub fn apply_definitions(&mut self, defs: Vec<TagDef>) {
        self.defs = defs;
    }

    pub fn apply_for_paths(&mut self, m: HashMap<PathBuf, Vec<TagId>>) {
        for (p, tags) in m {
            self.for_path.insert(p, tags);
        }
    }

    /// Tags that should appear for the given profile: its own + all globals.
    pub fn visible_defs<'a>(&'a self, profile: &'a str) -> impl Iterator<Item = &'a TagDef> + 'a {
        self.defs.iter().filter(move |d| match &d.profile {
            None => true,
            Some(p) => p == profile,
        })
    }

    /// Optimistic local assign (DB write-through happens separately).
    pub fn assign_local(&mut self, tag: TagId, paths: &[PathBuf]) {
        for p in paths {
            let entry = self.for_path.entry(p.clone()).or_default();
            if !entry.contains(&tag) {
                entry.push(tag);
            }
        }
    }

    /// Optimistic local unassign.
    pub fn unassign_local(&mut self, tag: TagId, paths: &[PathBuf]) {
        for p in paths {
            if let Some(entry) = self.for_path.get_mut(p) {
                entry.retain(|t| *t != tag);
            }
        }
    }
}
