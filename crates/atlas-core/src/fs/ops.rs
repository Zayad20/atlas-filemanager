use atlas_model::app_state::BgResult;
use atlas_model::pane::TabId;
use std::path::{Path, PathBuf};
use crate::util::display::display_name;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// Copy/move live in the unified transfer engine (`super::transfer`); this
// module keeps the simpler one-shot operations (delete, rename, create dir).

pub async fn delete_files(
    paths: Vec<PathBuf>,
    to_trash: bool,
    op_id: u64,
    tx: flume::Sender<BgResult>,
    cancel: Arc<AtomicBool>,
    affected_tabs: Vec<TabId>,
) {
    let total = paths.len() as u64;
    let mut errors: Vec<String> = Vec::new();
    let mut cancelled = false;
    for (i, path) in paths.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        let _ = tx.send(BgResult::OpProgress {
            id: op_id,
            done: i as u64,
            total,
            bytes_done: 0,
            bytes_total: 0,
            current_file: path.clone(),
        });
        if !path.exists() {
            errors.push(format!("{}: Ya no existe", display_name(path)));
            continue;
        }
        let result = if to_trash {
            let p = path.clone();
            tokio::task::spawn_blocking(move || trash::delete(&p).map_err(|e| e.to_string()))
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
        } else {
            remove_path(path).await.map_err(|e| e.to_string())
        };
        if let Err(e) = result {
            errors.push(format!("{}: {}", display_name(path), e));
        }
    }
    if cancelled {
        errors.push("Eliminación cancelada".to_string());
    }
    let result = if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    };
    let _ = tx.send(BgResult::OpComplete {
        id: op_id,
        result,
        affected_tabs,
    });
}


pub async fn rename_file(
    from: PathBuf,
    to: PathBuf,
    op_id: u64,
    tx: flume::Sender<BgResult>,
    affected_tabs: Vec<TabId>,
) {
    let result = tokio::fs::rename(&from, &to).await.map_err(|e| {
        format!(
            "{}: {}",
            display_name(&from),
            super::transfer::friendly_io_detail(&e)
        )
    });
    let _ = tx.send(BgResult::OpComplete {
        id: op_id,
        result,
        affected_tabs,
    });
}

pub async fn create_dir(
    path: PathBuf,
    op_id: u64,
    tx: flume::Sender<BgResult>,
    affected_tabs: Vec<TabId>,
) {
    let result = tokio::fs::create_dir_all(&path).await.map_err(|e| {
        format!(
            "{}: {}",
            display_name(&path),
            super::transfer::friendly_io_detail(&e)
        )
    });
    let _ = tx.send(BgResult::OpComplete {
        id: op_id,
        result,
        affected_tabs,
    });
}

async fn remove_path(path: &Path) -> anyhow::Result<()> {
    if path.is_dir() {
        tokio::fs::remove_dir_all(path).await?;
    } else {
        tokio::fs::remove_file(path).await?;
    }
    Ok(())
}
