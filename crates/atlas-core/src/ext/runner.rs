//! Execute declarative extension commands: substitute placeholders in the `run`
//! template and spawn the process(es). No shell is used — the template is split
//! into program + args and placeholders are substituted per token, so paths with
//! spaces are safe and there is no shell-injection surface.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::registry::CommandRef;
use atlas_model::app_state::BgResult;

#[derive(Serialize)]
struct WasmPayload {
    paths: Vec<String>,
}

/// Substitute placeholders for one input path into a single argument token.
// reason: "{dir}"/"{stem}"/"{ext}" are literal template placeholders for .replace(),
// not format-macro args (they happen to match local variable names).
#[allow(clippy::literal_string_with_formatting_args)]
fn subst_token(token: &str, path: &Path, answers: &HashMap<String, String>) -> String {
    let dir = path
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = token
        .replace("{input}", &path.to_string_lossy())
        .replace("{dir}", &dir)
        .replace("{stem}", &stem)
        .replace("{ext}", &ext)
        .replace("{output}", &format!("{dir}/{stem}_out.{ext}"));
    for (name, value) in answers {
        out = out.replace(&format!("{{prompt:{name}}}"), value);
    }
    out
}

/// Build the concrete argv for one invocation over `inputs` (one path for
/// per-file commands, all paths for batch commands).
fn build_argv(run: &str, inputs: &[PathBuf], answers: &HashMap<String, String>) -> Vec<String> {
    let primary = inputs.first().cloned().unwrap_or_default();
    let mut argv = Vec::new();
    for token in run.split_whitespace() {
        if token == "{input}" && inputs.len() > 1 {
            // Batch: expand a bare {input} to every path as separate args.
            for p in inputs {
                argv.push(p.to_string_lossy().into_owned());
            }
        } else {
            argv.push(subst_token(token, &primary, answers));
        }
    }
    argv
}

/// Run a thumbnailer command for one `input`: substitute `{input}`/`{output}` (a temp
/// PNG) plus `{dir}`/`{stem}`/`{ext}`, spawn the process, then decode the produced PNG
/// into a downscaled RGBA thumbnail. Returns `None` on any failure (no shell is used).
/// Blocking — call from a blocking task.
// reason: the temp-PNG cleanup uses sync fs on the blocking pool, never the gpui render
// thread; the sync-fs ban targets UI-foreground blocking (same as fs/preview.rs).
#[allow(clippy::disallowed_methods)]
pub fn run_thumbnail(
    input: &Path,
    run_tmpl: &str,
    max_edge: u32,
) -> Option<(Vec<u8>, usize, usize)> {
    let output = std::env::temp_dir().join(format!("atlas-extthumb-{}.png", uuid::Uuid::new_v4()));
    let argv = build_thumb_argv(run_tmpl, input, &output);
    let (program, args) = argv.split_first()?;
    let ran = std::process::Command::new(program).args(args).output();
    let ok = matches!(&ran, Ok(out) if out.status.success());
    let decoded = if ok {
        image::open(&output).ok().map(|img| {
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

// reason: literal template placeholders for .replace(), not format-macro args.
#[allow(clippy::literal_string_with_formatting_args)]
fn build_thumb_argv(run: &str, input: &Path, output: &Path) -> Vec<String> {
    let dir = input
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = input
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    run.split_whitespace()
        .map(|token| {
            token
                .replace("{input}", &input.to_string_lossy())
                .replace("{output}", &output.to_string_lossy())
                .replace("{dir}", &dir)
                .replace("{stem}", &stem)
                .replace("{ext}", &ext)
        })
        .collect()
}

/// Run a declarative command over `paths`, reporting completion via `BgResult`.
/// Sidecar commands are not yet executed (declarative covers the shipped tools).
pub fn run(
    rt: &tokio::runtime::Handle,
    bg_tx: flume::Sender<BgResult>,
    cmd: CommandRef,
    paths: Vec<PathBuf>,
    answers: HashMap<String, String>,
) {
    let ext_id = cmd.ext_id.clone();

    // Si la extensión es WASM, delegamos la ejecución al nuevo manager genérico
    if cmd.kind == atlas_model::ext_model::ExtKind::Wasm {
        rt.spawn_blocking(move || {
            let data_dir = crate::util::config::data_dir()
                .join("extensions")
                .join(&ext_id);

            // TODO: In production we'd load the real .wasm file declared in Manifest.
            // For now, assume it's in the extension directory:
            let wasm_path = cmd.ext_dir.join("plugin.wasm");
            #[allow(clippy::disallowed_methods)] // reason: already inside spawn_blocking
            let wasm_bytes = std::fs::read(&wasm_path).unwrap_or_default();

            let manager = crate::ext::wasm_manager::WasmPluginManager::new(
                wasm_bytes,
                data_dir,
                bg_tx.clone(),
            );

            let wasm_payload = WasmPayload {
                paths: paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
            };
            let payload_json = serde_json::to_string(&wasm_payload).unwrap_or_default();

            tracing::info!("Delegando comando WASM {} a {}", cmd.command.id, ext_id);
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

    // Directories to refresh once the command finishes.
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
            let argv = build_argv(&run_tmpl, &batch, &answers);
            let Some((program, args)) = argv.split_first() else {
                continue;
            };
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
