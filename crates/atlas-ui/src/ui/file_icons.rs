use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{img, AnyElement, Hsla, IntoElement, RenderImage, Styled};

use crate::app::App;
use crate::ui::icons::{file_type_icon, is_windows_theme};
use atlas_model::file_entry::FileKind;

#[derive(Clone)]
pub(crate) struct FileIconRef {
    pub(crate) path: PathBuf,
    pub(crate) kind: FileKind,
    pub(crate) extension: Option<gpui::SharedString>,
}

#[derive(Clone)]
pub(crate) enum IconTexture {
    Single(Arc<RenderImage>),
    Multi(Arc<crate::icon_theme::MultiResIcon>),
}

impl IconTexture {
    #[inline]
    pub(crate) fn for_size(&self, px: f32) -> Arc<RenderImage> {
        match self {
            Self::Single(tex) => tex.clone(),
            Self::Multi(multi) => multi.for_size(px),
        }
    }

    #[inline]
    #[allow(dead_code)]
    pub(crate) fn single(&self) -> Arc<RenderImage> {
        match self {
            Self::Single(tex) => tex.clone(),
            Self::Multi(multi) => multi.full.clone(),
        }
    }
}

impl From<Arc<RenderImage>> for IconTexture {
    fn from(tex: Arc<RenderImage>) -> Self {
        Self::Single(tex)
    }
}

impl From<Arc<crate::icon_theme::MultiResIcon>> for IconTexture {
    fn from(multi: Arc<crate::icon_theme::MultiResIcon>) -> Self {
        Self::Multi(multi)
    }
}

#[derive(Clone)]
pub(crate) enum FileIcon {
    PreviewTexture(Arc<RenderImage>),
    IconTexture(IconTexture),
    Builtin {
        kind: FileKind,
        extension: Option<gpui::SharedString>,
    },
}

impl FileIcon {
    pub(crate) fn element(self, size: f32, _color: Hsla) -> AnyElement {
        match self {
            Self::PreviewTexture(tex) => img(tex)
                .size(crate::ui::rem_px(size))
                .flex_none()
                .into_any_element(),
            Self::IconTexture(tex) => img(tex.for_size(crate::ui::BASE_PX * size))
                .size(crate::ui::rem_px(size))
                .flex_none()
                .into_any_element(),
            Self::Builtin { kind, extension } => file_type_icon(&kind, extension.as_deref(), size),
        }
    }

    pub(crate) fn texture(&self) -> Option<Arc<RenderImage>> {
        self.texture_for_size(24.0)
    }

    pub(crate) fn texture_for_size(&self, px: f32) -> Option<Arc<RenderImage>> {
        match self {
            Self::PreviewTexture(tex) => Some(tex.clone()),
            Self::IconTexture(tex) => Some(tex.for_size(px)),
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
            extension: extension.map(gpui::SharedString::from),
        }
    }

    /// Icon for compact rows (details/list/tree/miller/palette): in Windows themes
    /// always the plain Shell file-type icon — never a thumbnail or content peek.
    pub(crate) fn row_icon_for_entry(
        &self,
        path: &Path,
        kind: &FileKind,
        extension: Option<&str>,
    ) -> FileIcon {
        #[cfg(target_os = "windows")]
        if is_windows_theme(&self.state.profiles.active().theme) {
            let tex = if matches!(kind, FileKind::Dir) {
                self.shell_folder_icon()
            } else {
                self.shell_type_icon(path)
            };
            if let Some(tex) = tex {
                return FileIcon::IconTexture(IconTexture::Multi(tex));
            }
        } else if let Some(tex) = self.grid_thumbnail(path) {
            return FileIcon::PreviewTexture(tex);
        }
        self.file_icon_fallback(kind, extension)
    }

    fn file_icon_fallback(&self, kind: &FileKind, extension: Option<&str>) -> FileIcon {
        if let Some(multi) = self.multi_res_icon_for_entry(kind, extension) {
            return FileIcon::IconTexture(IconTexture::Multi(multi));
        }
        FileIcon::IconTexture(IconTexture::Single(self.liquid_glass_icon(kind, extension)))
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
        #[cfg(target_os = "windows")]
        if matches!(kind, FileKind::Dir) && is_windows_theme(&self.state.profiles.active().theme) {
            if let Some(tex) = self.shell_folder_icon() {
                return FileIcon::IconTexture(IconTexture::Multi(tex));
            }
        }
        #[cfg(target_os = "windows")]
        if !matches!(kind, FileKind::Dir) && is_windows_theme(&self.state.profiles.active().theme) {
            if let Some(tex) = self.shell_type_icon(path) {
                return FileIcon::IconTexture(IconTexture::Multi(tex));
            }
        }
        self.file_icon_fallback(kind, extension)
    }

    pub(crate) fn file_icon(&self, file: &FileIconRef) -> FileIcon {
        self.row_icon_for_entry(&file.path, &file.kind, file.extension.as_deref())
    }

    pub(crate) fn maybe_request_file_icon(&mut self, file: &FileIconRef) {
        #[cfg(target_os = "windows")]
        if is_windows_theme(&self.state.profiles.active().theme) {
            if matches!(file.kind, FileKind::Dir) {
                self.request_shell_folder_icon();
            } else {
                self.request_shell_type_icon(&file.path);
            }
        }
    }
}
