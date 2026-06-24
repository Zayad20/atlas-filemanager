use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{img, AnyElement, Hsla, IntoElement, RenderImage, Styled};

use crate::app::App;
use crate::state::file_entry::FileKind;
use crate::ui::icons::file_type_icon;

#[derive(Clone)]
pub(crate) struct FileIconRef {
    pub(crate) path: PathBuf,
    pub(crate) kind: FileKind,
    pub(crate) extension: Option<String>,
}

#[derive(Clone)]
pub(crate) enum FileIcon {
    PreviewTexture(Arc<RenderImage>),
    IconTexture(Arc<RenderImage>),
    Builtin {
        kind: FileKind,
        extension: Option<String>,
    },
}

impl FileIcon {
    pub(crate) fn element(self, size: f32, _color: Hsla) -> AnyElement {
        match self {
            Self::PreviewTexture(tex) | Self::IconTexture(tex) => {
                img(tex).size(gpui::px(size)).flex_none().into_any_element()
            }
            Self::Builtin { kind, extension } => file_type_icon(&kind, extension.as_deref(), size),
        }
    }

    pub(crate) fn texture(&self) -> Option<Arc<RenderImage>> {
        match self {
            Self::PreviewTexture(tex) | Self::IconTexture(tex) => Some(tex.clone()),
            Self::Builtin { .. } => None,
        }
    }
}

impl App {
    pub(crate) fn file_icon_ref_for_path(path: PathBuf, is_dir: bool) -> FileIconRef {
        let kind = if is_dir {
            FileKind::Dir
        } else {
            FileKind::File
        };
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());
        FileIconRef {
            path,
            kind,
            extension,
        }
    }

    pub(crate) fn file_icon_for_entry(
        &self,
        path: &Path,
        kind: &FileKind,
        extension: Option<&str>,
    ) -> FileIcon {
        if let Some(tex) = self.grid_thumbnail(path) {
            return FileIcon::PreviewTexture(tex);
        }
        if let Some(tex) = self.icon_tex_for_entry(kind, extension) {
            return FileIcon::IconTexture(tex);
        }
        FileIcon::Builtin {
            kind: kind.clone(),
            extension: extension.map(str::to_string),
        }
    }

    pub(crate) fn file_icon(&self, file: &FileIconRef) -> FileIcon {
        self.file_icon_for_entry(&file.path, &file.kind, file.extension.as_deref())
    }

    pub(crate) fn maybe_request_file_icon(&mut self, file: &FileIconRef) {
        #[cfg(target_os = "windows")]
        if self.state.profiles.active().theme == "Atlas Windows" && file.kind == FileKind::Dir {
            self.request_shell_thumbnail_or_icon(file.path.clone());
        }
    }
}
