//! Account-relative opaque object addresses. Each component keeps the provider's
//! immutable ID and its display name; parent traversal keeps the original IDs.
const PREFIX: &str = "~atlas-object~";

#[derive(Debug, PartialEq, Eq)]
pub struct ObjectComponent {
    pub id: String,
    pub name: String,
    pub is_dir: bool,
}

fn hex(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn unhex(value: &str) -> Option<String> {
    if value.len() % 2 != 0 {
        return None;
    }
    let bytes = (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(value.get(index..index + 2)?, 16).ok())
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok()
}
pub fn join(parent: &str, id: &str, name: &str, is_dir: bool) -> String {
    let component = format!(
        "{PREFIX}{}~{}~{}",
        hex(id),
        if is_dir { "d" } else { "f" },
        hex(name)
    );
    if parent.is_empty() {
        component
    } else {
        format!("{parent}/{component}")
    }
}
pub fn component(value: &str) -> Option<ObjectComponent> {
    let mut parts = value.strip_prefix(PREFIX)?.split('~');
    let id = unhex(parts.next()?)?;
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return None;
    }
    let kind = parts.next()?;
    if !matches!(kind, "d" | "f") {
        return None;
    }
    let name = unhex(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(ObjectComponent {
        id,
        name,
        is_dir: kind == "d",
    })
}
pub fn last(path: &str) -> Option<ObjectComponent> {
    component(path.rsplit('/').next()?)
}
pub fn file_name(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    component(name).map_or_else(|| name.to_string(), |component| component.name)
}
pub fn display(path: &str) -> String {
    path.split('/').map(file_name).collect::<Vec<_>>().join("/")
}

pub fn safe_local_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return false;
    }
    if cfg!(windows) {
        if name.contains([':', '<', '>', '"', '|', '?', '*']) || name.ends_with(['.', ' ']) {
            return false;
        }
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit()
                && stem.as_bytes()[3] != b'0')
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_names_have_distinct_addresses_and_parent_ids_survive_navigation() {
        let parent = join("", "folder-id", "Fotos á/2026", true);
        let first = join(&parent, "object-1", "same.docx", false);
        let second = join(&parent, "object-2", "same.docx", false);
        assert_ne!(first, second);
        assert_eq!(file_name(&first), "same.docx");
        assert_eq!(last(&first).unwrap().id, "object-1");
        assert_eq!(first.rsplit_once('/').unwrap().0, parent);
        assert_eq!(last(&parent).unwrap().id, "folder-id");
        assert!(component("~atlas-object~2e2e2f6576696c~f~61").is_none());
    }

    #[test]
    fn remote_names_cannot_escape_a_local_download_destination() {
        for invalid in ["../outside", "..", "folder/file", "folder\\file", "a\0b"] {
            assert!(!safe_local_name(invalid));
        }
        assert!(safe_local_name("Documento á.docx"));
        if cfg!(windows) {
            for invalid in ["C:escape", "NUL.txt", "COM1.txt", "file."] {
                assert!(!safe_local_name(invalid));
            }
        }
    }
}
