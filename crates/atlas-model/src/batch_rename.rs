use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum RenameMode {
    Find,
    Prefix,
    Suffix,
    Number,
}

impl std::fmt::Display for RenameMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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
}

impl BatchRenameState {
    pub fn new(files: Vec<PathBuf>) -> Self {
        let mut s = Self {
            files,
            mode: RenameMode::Find,
            pattern: String::new(),
            replacement: String::new(),
            previews: Vec::new(),
        };
        s.update_previews();
        s
    }

    pub fn update_previews(&mut self) {
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
}
