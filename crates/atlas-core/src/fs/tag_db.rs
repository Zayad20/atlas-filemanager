// reason: runs on the dedicated SQLite blocking worker, never the gpui render
// thread; the sync-fs ban targets UI-foreground blocking.
#![allow(clippy::disallowed_methods)]
//! SQLite-backed tag store.
//!
//! Owns the only `rusqlite::Connection` on a dedicated blocking worker (rusqlite is
//! `!Sync` and SQL is blocking I/O). The UI never touches the connection: it sends
//! [`TagDbCmd`] over a channel and receives answers back through the shared
//! `BgResult` channel, drained on the UI thread. Writes are fire-and-forget; reads
//! reply asynchronously. This keeps the 60fps loop free of SQL latency.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use atlas_model::app_state::BgResult;
use atlas_model::tag_cache::{TagDef, TagId};

/// Commands accepted by the tag-db worker.
pub enum TagDbCmd {
    // --- tag definitions (each re-sends the full def list afterwards) ---
    CreateTag {
        name: String,
        color: [u8; 3],
        profile: Option<String>,
    },
    RenameTag {
        tag: TagId,
        name: String,
    },
    DeleteTag(TagId),
    // --- associations (fire-and-forget) ---
    AssignTag {
        tag: TagId,
        paths: Vec<PathBuf>,
    },
    UnassignTag {
        tag: TagId,
        paths: Vec<PathBuf>,
    },
    /// Re-key associations after an in-app move/rename (file or directory subtree).
    RenamePath {
        old: PathBuf,
        new: PathBuf,
    },
    // --- reads (reply via BgResult) ---
    QueryTagsForPaths(Vec<PathBuf>),
    QueryPathsForTag(TagId),
}

/// Spawn the tag-db worker. `migration` is imported only on a brand-new database
/// (used to carry over tags that previously lived in profile config). Returns the
/// channel the UI uses to talk to the worker.
pub fn spawn(
    rt: &tokio::runtime::Handle,
    db_path: PathBuf,
    migration: Vec<TagDef>,
    bg_tx: flume::Sender<BgResult>,
) -> flume::Sender<TagDbCmd> {
    let (tx, rx) = flume::unbounded::<TagDbCmd>();
    rt.spawn_blocking(move || {
        let conn = match open(&db_path) {
            Ok(conn) => conn,
            Err(e) => {
                tracing::error!("tag_db: failed to open {db_path:?}: {e}");
                return;
            }
        };
        if let Err(e) = migrate_initial(&conn, &migration) {
            tracing::error!("tag_db: initial migration failed: {e}");
        }
        if let Err(e) = send_defs(&conn, &bg_tx) {
            tracing::error!("tag_db: initial def load failed: {e}");
        }
        while let Ok(cmd) = rx.recv() {
            if let Err(e) = handle(&conn, cmd, &bg_tx) {
                tracing::error!("tag_db: command failed: {e}");
            }
        }
    });
    tx
}

fn open(db_path: &Path) -> rusqlite::Result<Connection> {
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(db_path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS tags (
            id      INTEGER PRIMARY KEY,
            name    TEXT    NOT NULL,
            color_r INTEGER NOT NULL,
            color_g INTEGER NOT NULL,
            color_b INTEGER NOT NULL,
            profile TEXT,
            UNIQUE(name, profile)
         );
         CREATE TABLE IF NOT EXISTS file_tags (
            path   TEXT    NOT NULL,
            tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
            PRIMARY KEY (path, tag_id)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS idx_file_tags_tag ON file_tags(tag_id);",
    )?;
    Ok(conn)
}

fn migrate_initial(conn: &Connection, migration: &[TagDef]) -> rusqlite::Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))?;
    if count > 0 || migration.is_empty() {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT OR IGNORE INTO tags (name, color_r, color_g, color_b, profile)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for def in migration {
            stmt.execute(params![
                def.name,
                def.color[0],
                def.color[1],
                def.color[2],
                def.profile,
            ])?;
        }
    }
    tx.commit()
}

fn handle(
    conn: &Connection,
    cmd: TagDbCmd,
    bg_tx: &flume::Sender<BgResult>,
) -> rusqlite::Result<()> {
    match cmd {
        TagDbCmd::CreateTag {
            name,
            color,
            profile,
        } => {
            conn.execute(
                "INSERT OR IGNORE INTO tags (name, color_r, color_g, color_b, profile)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![name, color[0], color[1], color[2], profile],
            )?;
            send_defs(conn, bg_tx)?;
        }
        TagDbCmd::DeleteTag(id) => {
            conn.execute("DELETE FROM tags WHERE id = ?1", [id.0])?;
            send_defs(conn, bg_tx)?;
        }
        TagDbCmd::RenameTag { tag, name } => {
            conn.execute(
                "UPDATE OR IGNORE tags SET name = ?1 WHERE id = ?2",
                params![name, tag.0],
            )?;
            send_defs(conn, bg_tx)?;
        }
        TagDbCmd::AssignTag { tag, paths } => {
            let tx = conn.unchecked_transaction()?;
            {
                let mut stmt =
                    tx.prepare("INSERT OR IGNORE INTO file_tags (path, tag_id) VALUES (?1, ?2)")?;
                for p in &paths {
                    stmt.execute(params![path_key(p), tag.0])?;
                }
            }
            tx.commit()?;
        }
        TagDbCmd::UnassignTag { tag, paths } => {
            let tx = conn.unchecked_transaction()?;
            {
                let mut stmt =
                    tx.prepare("DELETE FROM file_tags WHERE path = ?1 AND tag_id = ?2")?;
                for p in &paths {
                    stmt.execute(params![path_key(p), tag.0])?;
                }
            }
            tx.commit()?;
        }
        TagDbCmd::RenamePath { old, new } => {
            let old_key = path_key(&old);
            let new_key = path_key(&new);
            // The path itself (a tagged file, or the moved directory if it was tagged).
            conn.execute(
                "UPDATE OR IGNORE file_tags SET path = ?2 WHERE path = ?1",
                params![old_key, new_key],
            )?;
            // Tagged descendants when a directory subtree is moved.
            conn.execute(
                "UPDATE OR IGNORE file_tags
                 SET path = ?2 || substr(path, length(?1) + 1)
                 WHERE path LIKE ?1 || '/%'",
                params![old_key, new_key],
            )?;
        }
        TagDbCmd::QueryTagsForPaths(paths) => {
            let mut map: HashMap<PathBuf, Vec<TagId>> = HashMap::with_capacity(paths.len());
            let mut stmt = conn.prepare("SELECT tag_id FROM file_tags WHERE path = ?1")?;
            for p in paths {
                let ids = stmt
                    .query_map([path_key(&p)], |row| Ok(TagId(row.get(0)?)))?
                    .filter_map(Result::ok)
                    .collect();
                map.insert(p, ids);
            }
            let _ = bg_tx.send(BgResult::TagsForPaths { tags: map });
        }
        TagDbCmd::QueryPathsForTag(tag) => {
            let mut stmt = conn.prepare("SELECT path FROM file_tags WHERE tag_id = ?1")?;
            let paths = stmt
                .query_map([tag.0], |row| {
                    let s: String = row.get(0)?;
                    Ok(PathBuf::from(s))
                })?
                .filter_map(Result::ok)
                .collect();
            let _ = bg_tx.send(BgResult::PathsForTag { tag, paths });
        }
    }
    Ok(())
}

fn send_defs(conn: &Connection, bg_tx: &flume::Sender<BgResult>) -> rusqlite::Result<()> {
    let defs = load_defs(conn)?;
    let _ = bg_tx.send(BgResult::TagDefinitions(defs));
    Ok(())
}

fn load_defs(conn: &Connection) -> rusqlite::Result<Vec<TagDef>> {
    let mut stmt = conn
        .prepare("SELECT id, name, color_r, color_g, color_b, profile FROM tags ORDER BY name")?;
    let defs = stmt
        .query_map([], |row| {
            Ok(TagDef {
                id: TagId(row.get(0)?),
                name: row.get(1)?,
                color: [row.get(2)?, row.get(3)?, row.get(4)?],
                profile: row.get(5)?,
            })
        })?
        .filter_map(Result::ok)
        .collect();
    Ok(defs)
}

/// Associations are keyed by the file's absolute path string. We deliberately do not
/// canonicalize (which would resolve symlinks and require the file to exist): every
/// path the UI sends comes from `DirEntry::path()`, so they are already absolute and
/// consistent between write and read.
fn path_key(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
