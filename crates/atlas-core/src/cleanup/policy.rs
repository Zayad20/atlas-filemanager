use std::path::{Component, Path, PathBuf};

/// Deliberately excludes network/device paths and ambiguous drive-relative paths.
pub fn is_drive_root(path: &Path) -> bool {
    let value = path.as_os_str().to_string_lossy();
    let bytes = value.as_bytes();
    bytes.len() == 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

pub fn user_temp_root() -> Option<PathBuf> {
    dirs::data_local_dir().map(|local| local.join("Temp"))
}

/// A user-facing group of regenerable data. Every root is a directory whose
/// old files may be offered for cleanup; nothing outside these roots ever is.
pub struct Category {
    pub key: &'static str,
    pub roots: Vec<PathBuf>,
}

pub fn categories() -> Vec<Category> {
    let local = dirs::data_local_dir();
    let home = dirs::home_dir();
    let join = |base: &Option<PathBuf>, rel: &str| base.as_ref().map(|b| b.join(rel));
    let mut browsers = Vec::new();
    for base in [
        "Google/Chrome",
        "Microsoft/Edge",
        "BraveSoftware/Brave-Browser",
        "Vivaldi",
    ] {
        for cache in ["Cache", "Code Cache", "GPUCache"] {
            browsers.extend(join(&local, &format!("{base}/User Data/Default/{cache}")));
        }
    }
    if let Some(profiles) = join(&local, "Mozilla/Firefox/Profiles") {
        if let Ok(entries) = std::fs::read_dir(profiles) {
            browsers.extend(entries.flatten().map(|e| e.path().join("cache2")));
        }
    }
    let list = [
        ("cleanup_cat_temp", user_temp_root().into_iter().collect()),
        ("cleanup_cat_browser", browsers),
        (
            "cleanup_cat_crash",
            [
                join(&local, "CrashDumps"),
                join(&local, "Microsoft/Windows/WER/ReportArchive"),
                join(&local, "Microsoft/Windows/WER/ReportQueue"),
            ]
            .into_iter()
            .flatten()
            .collect(),
        ),
        (
            "cleanup_cat_dev",
            [
                join(&local, "npm-cache"),
                join(&local, "pip/Cache"),
                join(&local, "Yarn/Cache"),
                join(&home, ".cargo/registry/cache"),
            ]
            .into_iter()
            .flatten()
            .collect(),
        ),
    ];
    list.into_iter()
        .map(|(key, roots): (&'static str, Vec<PathBuf>)| Category {
            key,
            roots: roots.into_iter().filter(|r| r.is_dir()).collect(),
        })
        .filter(|c| !c.roots.is_empty())
        .collect()
}

pub fn is_cleanup_root(path: &Path) -> bool {
    categories()
        .iter()
        .any(|c| c.roots.iter().any(|r| r == path))
}

/// Conservative component policy shared by discovery and execution. This is an
/// extra exclusion; filesystem identity checks remain the mutation authority.
pub fn protected(path: &Path) -> bool {
    path.components().any(|part| match part {
        Component::Normal(name) => {
            let name = name.to_string_lossy().to_ascii_lowercase();
            matches!(
                name.as_str(),
                ".ssh"
                    | ".gnupg"
                    | ".aws"
                    | ".azure"
                    | ".kube"
                    | ".docker"
                    | ".git"
                    | ".bb"
                    | ".agents"
                    | ".gemini"
                    | ".env"
                    | ".npmrc"
                    | ".pypirc"
                    | ".netrc"
                    | ".claude"
                    | ".codex"
                    | ".cursor"
                    | "credentials"
                    | "protect"
                    | "crypto"
                    | "vault"
                    | "systemcertificates"
                    | "login data"
                    | "cookies"
                    | "web data"
                    | "sessions"
                    | "start menu"
                    | "$recycle.bin"
                    | "system volume information"
            )
        }
        _ => false,
    })
}

pub(super) fn valid_temp_candidate(path: &Path, temp: &Path) -> bool {
    path.strip_prefix(temp).is_ok_and(|relative| {
        relative
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
            && relative.components().next().is_some()
    }) && !protected(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drive_scope_rejects_unc_relative_and_nested_paths() {
        for value in ["C:\\", "d:/"] {
            assert!(is_drive_root(Path::new(value)));
        }
        for value in ["C:", "C:\\Users", "\\\\server\\share", "\\\\?\\C:\\", "/"] {
            assert!(!is_drive_root(Path::new(value)), "{value}");
        }
    }
    #[test]
    fn temp_scope_is_component_bounded_and_protected() {
        let root = Path::new("local/Temp");
        assert!(valid_temp_candidate(Path::new("local/Temp/a/b.tmp"), root));
        for path in [
            "local/Temporary/file",
            "local/Temp",
            "local/Temp/../a",
            "local/Temp/.ssh/id",
            "local/Temp/.codex/sessions/a",
            "local/Temp/.env",
            "local/Temp/.bb/state",
        ] {
            assert!(!valid_temp_candidate(Path::new(path), root), "{path}");
        }
    }
}
