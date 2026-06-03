//! Execute declarative extension commands: substitute placeholders in the `run`
//! template and spawn the process(es). No shell is used — the template is split
//! into program + args and placeholders are substituted per token, so paths with
//! spaces are safe and there is no shell-injection surface.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::registry::CommandRef;
use crate::state::app_state::BgResult;

/// Substitute placeholders for one input path into a single argument token.
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

/// Run a declarative command over `paths`, reporting completion via `BgResult`.
/// Sidecar commands are not yet executed (declarative covers the shipped tools).
pub fn run(
    rt: &tokio::runtime::Handle,
    bg_tx: flume::Sender<BgResult>,
    cmd: CommandRef,
    paths: Vec<PathBuf>,
    answers: HashMap<String, String>,
) {
    let run_tmpl = cmd.command.run.clone();
    if run_tmpl.trim().is_empty() {
        return;
    }
    let per_file = cmd.command.per_file;
    let ext_id = cmd.ext_id.clone();

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
