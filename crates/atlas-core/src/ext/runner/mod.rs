#![allow(clippy::disallowed_methods)]
// reason: extension commands/previews run on blocking workers and must touch files.
//! Execute declarative extension commands: substitute placeholders in the `run`
//! template and spawn the process(es). No shell is used — the template is split
//! into program + args and placeholders are substituted per token, so paths with
//! spaces are safe and there is no shell-injection surface.

mod argv;
mod ext_preview;
mod img;
mod office;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::registry::CommandRef;
use atlas_model::app_state::BgResult;
use atlas_model::ext_model::ExtKind;
use atlas_model::pane::TabId;

/// Run a thumbnailer command for one `input`: substitute `{input}`/`{output}` (a temp
/// PNG) plus `{dir}`/`{stem}`/`{ext}`, spawn the process, then decode the produced PNG
/// into a downscaled RGBA thumbnail. Returns `None` on any failure.
/// Blocking — call from a blocking task.
pub fn run_thumbnail(
    input: &Path,
    run_tmpl: &str,
    tools: &HashMap<String, PathBuf>,
    max_edge: u32,
) -> Option<(Vec<u8>, usize, usize)> {
    let output = std::env::temp_dir()
        .join(format!("atlas-extthumb-{}.png", uuid::Uuid::new_v4()));
    let argv = argv::build_thumb_argv(run_tmpl, input, &output);
    let (program, args) = argv.split_first()?;
    let program = tools
        .get(program)
        .map_or_else(|| PathBuf::from(program), PathBuf::clone);
    let ran = std::process::Command::new(program).args(args).output();
    let ok = matches!(&ran, Ok(out) if out.status.success());
    let decoded = if ok {
        ::image::open(&output).ok().map(|img| {
            let thumb = img.thumbnail(max_edge, max_edge).to_rgba8();
            let (w, h) = (thumb.width() as usize, thumb.height() as usize);
            (thumb.into_raw(), w, h)
        })
    } else {
        None
    };
    let _ = std::fs::remove_file(&output);
    decoded
}

pub fn run_preview(
    rt: &tokio::runtime::Handle,
    bg_tx: flume::Sender<BgResult>,
    cmd: CommandRef,
    tab: TabId,
    path: PathBuf,
) {
    rt.spawn_blocking(move || {
        let result = if cmd.ext_id == "document-previews" {
            office::builtin_document_preview(&path, &cmd.tools)
        } else if cmd.kind == ExtKind::Wasm {
            ext_preview::run_wasm_preview(&cmd, &path)
                .unwrap_or_else(|e| atlas_model::preview_cache::PreviewResult::Error(e.to_string()))
        } else {
            ext_preview::run_declarative_preview(&cmd, &path)
        };
        let _ = bg_tx.send(BgResult::PreviewReady { tab, path, result });
    });
}

/// Run a declarative command over `paths`, reporting completion via `BgResult`.
pub fn run(
    rt: &tokio::runtime::Handle,
    bg_tx: flume::Sender<BgResult>,
    cmd: CommandRef,
    paths: Vec<PathBuf>,
    answers: HashMap<String, String>,
) {
    let ext_id = cmd.ext_id.clone();

    if cmd.kind == ExtKind::Wasm {
        rt.spawn_blocking(move || {
            let data_dir = crate::util::config::data_dir()
                .join("extensions")
                .join(&ext_id);

            let wasm_path = cmd.ext_dir.join("plugin.wasm");
            let wasm_bytes = match std::fs::read(&wasm_path) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!("WASM plugin not found at {wasm_path:?}: {e}");
                    return;
                }
            };

            let manager = crate::ext::wasm_manager::WasmPluginManager::new(
                wasm_bytes,
                data_dir,
                bg_tx.clone(),
                cmd.tools.clone(),
            );

            let payload = ext_preview::WasmPayload {
                paths: paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
            };
            let payload_json = serde_json::to_string(&payload).unwrap_or_default();

            tracing::info!("dispatching WASM command {} to extension {}", cmd.command.id, ext_id);
            if let Err(e) = manager.execute_command(&cmd.command.id, &payload_json) {
                tracing::error!("Error executing WASM: {:#}", e);
            }

            let dirs: Vec<PathBuf> = paths
                .iter()
                .filter_map(|p| p.parent().map(Path::to_path_buf))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();

            let _ = bg_tx.send(BgResult::ExtCommandDone {
                ext_id,
                error: None,
                dirs,
            });
        });
        return;
    }

    let run_tmpl = cmd.command.run.clone();
    if run_tmpl.trim().is_empty() {
        return;
    }
    let per_file = cmd.command.per_file;
    let ext_id = cmd.ext_id;

    let dirs: HashSet<PathBuf> = paths
        .iter()
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .collect();

    rt.spawn_blocking(move || {
        let batches: Vec<Vec<PathBuf>> = if per_file {
            paths.iter().map(|p| vec![p.clone()]).collect()
        } else {
            vec![paths.clone()]
        };

        let mut errors = Vec::new();
        for batch in batches {
            let argv_vec = argv::build_argv(&run_tmpl, &batch, &answers);
            let Some((program, args)) = argv_vec.split_first() else {
                continue;
            };
            let program = cmd
                .tools
                .get(program)
                .map_or_else(|| PathBuf::from(program), PathBuf::clone);
            match std::process::Command::new(program).args(args).output() {
                Ok(out) if out.status.success() => {}
                Ok(out) => errors.push(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                Err(e) => errors.push(e.to_string()),
            }
        }

        let _ = bg_tx.send(BgResult::ExtCommandDone {
            ext_id,
            error: errors.into_iter().find(|e| !e.is_empty()),
            dirs: dirs.into_iter().collect(),
        });
    });
}
