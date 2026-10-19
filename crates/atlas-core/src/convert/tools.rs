use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tool {
    Magick,
    Ffmpeg,
    Pandoc,
    Office,
    PdfText,
    SevenZip,
    Calibre,
}

impl Tool {
    pub const ALL: [Self; 7] = [
        Self::Magick,
        Self::Ffmpeg,
        Self::Pandoc,
        Self::Office,
        Self::PdfText,
        Self::SevenZip,
        Self::Calibre,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Magick => "ImageMagick",
            Self::Ffmpeg => "FFmpeg",
            Self::Pandoc => "Pandoc",
            Self::Office => "LibreOffice",
            Self::PdfText => "Poppler",
            Self::SevenZip => "7-Zip",
            Self::Calibre => "Calibre",
        }
    }
    pub const fn executable(self) -> &'static str {
        match self {
            Self::Magick => "magick",
            Self::Ffmpeg => "ffmpeg",
            Self::Pandoc => "pandoc",
            Self::Office => "soffice",
            Self::PdfText => "pdftotext",
            Self::SevenZip => "7z",
            Self::Calibre => "ebook-convert",
        }
    }
    pub const fn url(self) -> &'static str {
        match self {
            Self::Magick => "https://imagemagick.org/script/download.php",
            Self::Ffmpeg => "https://ffmpeg.org/download.html",
            Self::Pandoc => "https://pandoc.org/installing.html",
            Self::Office => "https://www.libreoffice.org/download/download-libreoffice/",
            Self::PdfText => "https://poppler.freedesktop.org/",
            Self::SevenZip => "https://www.7-zip.org/",
            Self::Calibre => "https://calibre-ebook.com/download",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemAudio {
    pub wave: bool,
    pub mp3: bool,
    pub aac: bool,
}

impl SystemAudio {
    pub fn supports(self, input: &str, output: &str) -> bool {
        self.wave
            && [
                "wav", "aiff", "aifc", "au", "mp3", "m4a", "aac", "wma", "flac", "mp4", "mov",
                "m4v",
            ]
            .contains(&input)
            && match output {
                "wav" | "aiff" | "au" | "flac" => true,
                "mp3" => self.mp3,
                "m4a" | "aac" => self.aac,
                _ => false,
            }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Tools(pub BTreeMap<Tool, PathBuf>, pub SystemAudio);

impl Tools {
    pub fn has(&self, tool: Tool) -> bool {
        self.0.contains_key(&tool)
    }
    pub fn get(&self, tool: Tool) -> Option<&Path> {
        self.0.get(&tool).map(PathBuf::as_path)
    }

    /// Scan executable paths and query system codecs off the interactive lane.
    /// Never start an external process, download a payload or scan recursively.
    pub fn discover() -> Self {
        Self::discover_with(std::iter::empty())
    }

    /// Registry paths are an immutable snapshot of enabled tool providers.
    /// Validate them on this worker lane and prefer them over system installs.
    pub fn discover_with(managed: impl IntoIterator<Item = (String, PathBuf)>) -> Self {
        let mut directories: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| {
                std::env::split_paths(&p)
                    .filter(|p| p.is_absolute())
                    .collect()
            })
            .unwrap_or_default();
        for root in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(root).map(PathBuf::from) {
                for suffix in [
                    "LibreOffice/program",
                    "7-Zip",
                    "Calibre2",
                    "Pandoc",
                    "Programs/Pandoc",
                    "Microsoft/WinGet/Links",
                    "Atlas/tools",
                ] {
                    directories.push(base.join(suffix));
                }
                if let Ok(entries) = std::fs::read_dir(&base) {
                    for entry in entries.flatten().take(512) {
                        if entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with("ImageMagick-")
                        {
                            directories.push(entry.path());
                        }
                    }
                }
            }
        }
        #[cfg(target_os = "macos")]
        directories.extend([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/Applications/LibreOffice.app/Contents/MacOS"),
            PathBuf::from("/Applications/calibre.app/Contents/MacOS"),
        ]);
        let mut tools = Self::default();
        #[cfg(target_os = "windows")]
        {
            let audio = atlas_win_space::media::audio_capabilities();
            tools.1 = SystemAudio {
                wave: audio.wave,
                mp3: audio.mp3,
                aac: audio.aac,
            };
        }
        for tool in Tool::ALL {
            let executable = format!("{}{}", tool.executable(), std::env::consts::EXE_SUFFIX);
            if let Some(path) = directories
                .iter()
                .map(|p| p.join(&executable))
                .find(|p| p.is_file())
            {
                tools.0.insert(tool, path);
            }
        }
        for (name, path) in managed {
            if let Some(tool) = Tool::ALL.into_iter().find(|tool| tool.executable() == name) {
                if path.is_absolute() && path.is_file() {
                    tools.0.insert(tool, path);
                }
            }
        }
        tools
    }
}
