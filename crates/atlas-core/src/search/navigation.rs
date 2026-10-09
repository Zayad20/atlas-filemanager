//! Filename and location preferences for the interactive navigation picker.
use std::path::Path;

/// Treat word separators alike without changing the displayed filesystem name.
pub fn normalized_name(name: &str, sensitive: bool) -> String {
    let mut result = String::new();
    let mut separator = false;
    for character in name.chars() {
        if character.is_whitespace() || matches!(character, '-' | '_') {
            separator = !result.is_empty();
            continue;
        }
        if separator {
            result.push(' ');
            separator = false;
        }
        if sensitive {
            result.push(character);
        } else {
            result.extend(character.to_lowercase());
        }
    }
    result
}

pub fn filename_match_quality(name: &str, query: &str) -> u8 {
    if query.is_empty() {
        return 0;
    }
    let sensitive = query.chars().any(char::is_uppercase);
    let name = normalized_name(name, sensitive);
    let query = normalized_name(query, sensitive);
    if query.is_empty() {
        return 0;
    }
    if name == query {
        4
    } else if name.starts_with(&query) {
        3
    } else if name.contains(&query) {
        2
    } else {
        1
    }
}

/// Prefer user folders over dependency trees, caches, and hidden histories.
/// An explicitly requested component remains searchable at its usual rank.
pub fn component_noise(name: &str, query: &str) -> u8 {
    let query = query.to_lowercase();
    let name = name.to_lowercase();
    if query
        .split(['/', '\\'])
        .any(|part| !part.is_empty() && part == name)
    {
        return 0;
    }
    // CLI histories encode an absolute path as a directory name. It is a
    // history bucket, not the original project. Preserve deliberate searches
    // for the encoded name, but omit these aliases from ordinary navigation.
    let encoded_drive = name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && name
            .get(1..)
            .is_some_and(|rest| rest.starts_with("--users-") || rest.starts_with("--home-"));
    if encoded_drive
        && !(query.len() >= 3 && query.as_bytes().get(1..3) == Some(b"--") && name.contains(&query))
    {
        return 3;
    }
    if name.starts_with('.') && name != "." && name != ".." {
        2
    } else if matches!(
        name.as_str(),
        "appdata"
            | "node_modules"
            | "target"
            | "__pycache__"
            | "$recycle.bin"
            | "system volume information"
    ) {
        1
    } else {
        0
    }
}

pub fn path_noise(path: &Path, query: &str) -> u8 {
    path.components()
        .map(|part| component_noise(&part.as_os_str().to_string_lossy(), query))
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_word_separators_match_without_erasing_punctuation() {
        for name in ["My Project", "my-project", "my_project"] {
            assert_eq!(filename_match_quality(name, "my-project"), 4);
        }
        assert_eq!(filename_match_quality("My Project heartbeats", "my-project"), 3);
        assert_eq!(filename_match_quality("Cargo.toml", "Cargotoml"), 1);
    }

    #[test]
    fn hidden_ancestors_and_dependency_trees_are_deprioritized() {
        assert_eq!(path_noise(Path::new("home/projects/My Project"), "my-project"), 0);
        assert_eq!(
            path_noise(Path::new("home/.claude/projects/my-project"), "my-project"),
            2
        );
        assert_eq!(
            path_noise(Path::new("home/AppData/Local/my-project"), "my-project"),
            1
        );
        assert_eq!(
            path_noise(Path::new("home/project/node_modules/my-project"), "my-project"),
            1
        );
        assert_eq!(path_noise(Path::new("home/.claude/projects"), ".claude"), 0);
        assert_eq!(component_noise("C--Users-user-my-project", "my-project"), 3);
        assert_eq!(component_noise("C--Users-user-my-project", "C--Users"), 0);
    }
}
