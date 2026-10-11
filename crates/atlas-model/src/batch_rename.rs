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
    MultiCursor,
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
            Self::MultiCursor => write!(f, "Multi-Cursor"),
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
    pub cursor_pos: usize,     // Character offset for the synchronized cursor
    pub multi_lines: Vec<String>, // Current edited filenames for each file
}

fn char_to_byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

fn prev_word_boundary(s: &str, char_pos: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    if len == 0 || char_pos == 0 {
        return 0;
    }
    let mut idx = char_pos.min(len);
    // 1. Skip non-alphanumeric chars
    while idx > 0 && !chars[idx - 1].is_alphanumeric() {
        idx -= 1;
    }
    // 2. Skip alphanumeric chars
    while idx > 0 && chars[idx - 1].is_alphanumeric() {
        idx -= 1;
    }
    idx
}

fn next_word_boundary(s: &str, char_pos: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    if len == 0 || char_pos >= len {
        return len;
    }
    let mut idx = char_pos;
    if chars[idx].is_alphanumeric() {
        while idx < len && chars[idx].is_alphanumeric() {
            idx += 1;
        }
        while idx < len && !chars[idx].is_alphanumeric() {
            idx += 1;
        }
    } else {
        while idx < len && !chars[idx].is_alphanumeric() {
            idx += 1;
        }
        while idx < len && chars[idx].is_alphanumeric() {
            idx += 1;
        }
    }
    idx
}

impl BatchRenameState {
    pub fn new(files: Vec<PathBuf>) -> Self {
        let mut s = Self {
            files,
            mode: RenameMode::MultiCursor,
            pattern: String::new(),
            replacement: String::new(),
            previews: Vec::new(),
            cursor_pos: 0,
            multi_lines: Vec::new(),
        };
        s.init_multicursor();
        s.update_previews();
        s
    }

    pub fn set_mode(&mut self, mode: RenameMode) {
        if mode == RenameMode::MultiCursor {
            self.init_multicursor();
        }
        self.mode = mode;
        self.update_previews();
    }

    pub fn init_multicursor(&mut self) {
        self.multi_lines = self
            .files
            .iter()
            .map(|path| {
                path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            })
            .collect();
        self.align_cursor_to_stem_end();
    }

    pub fn update_previews(&mut self) {
        if self.mode == RenameMode::MultiCursor {
            self.previews = self.multi_lines.clone();
            return;
        }

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
                    RenameMode::MultiCursor => name,
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

    pub fn insert_char(&mut self, c: char) {
        let mut buf = [0u8; 4];
        self.insert_str(c.encode_utf8(&mut buf));
    }

    pub fn insert_str(&mut self, s: &str) {
        let str_chars = s.chars().count();
        if str_chars == 0 {
            return;
        }
        for line in &mut self.multi_lines {
            let line_chars = line.chars().count();
            let pos = self.cursor_pos.min(line_chars);
            let byte_pos = char_to_byte_index(line, pos);
            line.insert_str(byte_pos, s);
        }
        self.cursor_pos += str_chars;
        self.update_previews();
    }

    pub fn backspace(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        for line in &mut self.multi_lines {
            let line_chars = line.chars().count();
            if line_chars == 0 {
                continue;
            }
            let pos = self.cursor_pos.min(line_chars);
            if pos > 0 {
                let start_byte = char_to_byte_index(line, pos - 1);
                let end_byte = char_to_byte_index(line, pos);
                line.replace_range(start_byte..end_byte, "");
            }
        }
        self.cursor_pos = self.cursor_pos.saturating_sub(1);
        self.update_previews();
    }

    pub fn delete_forward(&mut self) {
        for line in &mut self.multi_lines {
            let line_chars = line.chars().count();
            let pos = self.cursor_pos.min(line_chars);
            if pos < line_chars {
                let start_byte = char_to_byte_index(line, pos);
                let end_byte = char_to_byte_index(line, pos + 1);
                line.replace_range(start_byte..end_byte, "");
            }
        }
        self.update_previews();
    }

    pub fn move_cursor_left(&mut self) {
        self.cursor_pos = self.cursor_pos.saturating_sub(1);
    }

    pub fn move_cursor_right(&mut self) {
        let max_len = self
            .multi_lines
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0);
        if self.cursor_pos < max_len {
            self.cursor_pos += 1;
        }
    }

    pub fn move_cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    pub fn move_cursor_end(&mut self) {
        let max_len = self
            .multi_lines
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0);
        self.cursor_pos = max_len;
    }

    pub fn move_cursor_word_left(&mut self) {
        if let Some(reference) = self.multi_lines.first() {
            self.cursor_pos = prev_word_boundary(reference, self.cursor_pos);
        } else {
            self.cursor_pos = 0;
        }
    }

    pub fn move_cursor_word_right(&mut self) {
        if let Some(reference) = self.multi_lines.first() {
            self.cursor_pos = next_word_boundary(reference, self.cursor_pos);
        }
    }

    pub fn insert_counter(&mut self, padding: usize) {
        let count = self.multi_lines.len();
        for i in 0..count {
            let num_str = format!("{:0width$}", i + 1, width = padding);
            let line = &mut self.multi_lines[i];
            let line_chars = line.chars().count();
            let pos = self.cursor_pos.min(line_chars);
            let byte_pos = char_to_byte_index(line, pos);
            line.insert_str(byte_pos, &num_str);
        }
        let first_num_len = format!("{:0width$}", 1, width = padding).chars().count();
        self.cursor_pos += first_num_len;
        self.update_previews();
    }

    pub fn align_cursor_to_stem_end(&mut self) {
        if let Some(first) = self.multi_lines.first() {
            let stem_len = std::path::Path::new(first)
                .file_stem()
                .map(|s| s.to_string_lossy().chars().count())
                .unwrap_or_else(|| first.chars().count());
            self.cursor_pos = stem_len;
        } else {
            self.cursor_pos = 0;
        }
    }

    pub fn align_cursor_to_start(&mut self) {
        self.cursor_pos = 0;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multicursor_init() {
        let files = vec![
            PathBuf::from("/test/photo_1.png"),
            PathBuf::from("/test/photo_2.png"),
        ];
        let state = BatchRenameState::new(files);
        assert_eq!(state.mode, RenameMode::MultiCursor);
        assert_eq!(
            state.multi_lines,
            vec!["photo_1.png".to_string(), "photo_2.png".to_string()]
        );
        assert_eq!(state.cursor_pos, 7); // "photo_1" has 7 characters
        assert_eq!(state.previews, state.multi_lines);
    }

    #[test]
    fn test_multicursor_typing() {
        let files = vec![
            PathBuf::from("/test/doc_a.txt"),
            PathBuf::from("/test/doc_b.txt"),
        ];
        let mut state = BatchRenameState::new(files);
        assert_eq!(state.cursor_pos, 5); // "doc_a" length is 5

        state.insert_str("_v2");
        assert_eq!(
            state.multi_lines,
            vec!["doc_a_v2.txt".to_string(), "doc_b_v2.txt".to_string()]
        );
        assert_eq!(state.cursor_pos, 8);
        assert_eq!(state.previews, state.multi_lines);

        state.insert_char('!');
        assert_eq!(
            state.multi_lines,
            vec!["doc_a_v2!.txt".to_string(), "doc_b_v2!.txt".to_string()]
        );
        assert_eq!(state.cursor_pos, 9);
    }

    #[test]
    fn test_multicursor_backspace_and_delete() {
        let files = vec![
            PathBuf::from("/test/cat.png"),
            PathBuf::from("/test/dog.png"),
        ];
        let mut state = BatchRenameState::new(files);
        assert_eq!(state.cursor_pos, 3); // "cat" len is 3

        state.backspace();
        assert_eq!(
            state.multi_lines,
            vec!["ca.png".to_string(), "do.png".to_string()]
        );
        assert_eq!(state.cursor_pos, 2);

        // Delete forward deletes '.'
        state.delete_forward();
        assert_eq!(
            state.multi_lines,
            vec!["capng".to_string(), "dopng".to_string()]
        );
        assert_eq!(state.cursor_pos, 2);
    }

    #[test]
    fn test_multicursor_navigation() {
        let files = vec![
            PathBuf::from("/test/my_vacation_photo.jpg"),
            PathBuf::from("/test/my_vacation_other.jpg"),
        ];
        let mut state = BatchRenameState::new(files);
        // stem is "my_vacation_photo" (len 17)
        assert_eq!(state.cursor_pos, 17);

        state.move_cursor_home();
        assert_eq!(state.cursor_pos, 0);

        state.move_cursor_right();
        assert_eq!(state.cursor_pos, 1);

        state.move_cursor_left();
        assert_eq!(state.cursor_pos, 0);

        // Word navigation forward:
        // "my" -> skips alphanumeric, skips delimiter '_' -> pos 3
        state.move_cursor_word_right();
        assert_eq!(state.cursor_pos, 3);

        // "vacation" -> skips alphanumeric, skips delimiter '_' -> pos 12
        state.move_cursor_word_right();
        assert_eq!(state.cursor_pos, 12);

        // Word navigation backward:
        // skips delimiter '_', skips "vacation" -> pos 3
        state.move_cursor_word_left();
        assert_eq!(state.cursor_pos, 3);

        state.move_cursor_end();
        assert_eq!(state.cursor_pos, "my_vacation_photo.jpg".len());
    }

    #[test]
    fn test_multicursor_numbering() {
        let files = vec![
            PathBuf::from("/test/item.png"),
            PathBuf::from("/test/item.png"),
            PathBuf::from("/test/item.png"),
        ];
        let mut state = BatchRenameState::new(files);
        assert_eq!(state.cursor_pos, 4); // "item" len 4

        state.insert_str("_");
        state.insert_counter(2);
        assert_eq!(
            state.multi_lines,
            vec![
                "item_01.png".to_string(),
                "item_02.png".to_string(),
                "item_03.png".to_string(),
            ]
        );
        assert_eq!(state.cursor_pos, 7); // 4 + 1 + 2 = 7
    }

    #[test]
    fn test_multicursor_stem_and_start_align() {
        let files = vec![
            PathBuf::from("/test/readme.md"),
            PathBuf::from("/test/notes.md"),
        ];
        let mut state = BatchRenameState::new(files);
        assert_eq!(state.cursor_pos, 6);

        state.align_cursor_to_start();
        assert_eq!(state.cursor_pos, 0);

        state.align_cursor_to_stem_end();
        assert_eq!(state.cursor_pos, 6);
    }

    #[test]
    fn test_multicursor_collision_and_validation() {
        let files = vec![
            PathBuf::from("/test/a.txt"),
            PathBuf::from("/test/b.txt"),
        ];
        let mut state = BatchRenameState::new(files);
        // Initially a.txt and b.txt are unchanged (no-op, Ok)
        assert_eq!(state.issues(), vec![RenameIssue::Ok, RenameIssue::Ok]);
        assert!(!state.has_blocking_issues());

        // Replace both stems with "same"
        state.align_cursor_to_start();
        state.insert_str("same");
        // multi_lines: ["samea.txt", "sameb.txt"] -> no collision
        assert!(!state.has_blocking_issues());

        // Make both lines identical
        state.multi_lines = vec!["same.txt".to_string(), "same.txt".to_string()];
        state.update_previews();
        assert_eq!(
            state.issues(),
            vec![RenameIssue::Collision, RenameIssue::Collision]
        );
        assert!(state.has_blocking_issues());

        // Test invalid filename
        state.multi_lines = vec!["invalid/slash.txt".to_string(), "valid.txt".to_string()];
        state.update_previews();
        assert_eq!(
            state.issues(),
            vec![RenameIssue::Invalid, RenameIssue::Ok]
        );
        assert!(state.has_blocking_issues());
    }
}
