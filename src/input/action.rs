use crate::state::file_entry::{SortColumn, SortDir};
use crate::state::pane::{PaneId, TabId};
use crate::state::settings::ViewMode;
use std::path::PathBuf;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Action {
    Navigate {
        tab: TabId,
        path: PathBuf,
    },
    NavigateBack(TabId),
    NavigateForward(TabId),
    NavigateUp(TabId),
    SpringNavigate {
        tab: TabId,
        path: PathBuf,
    },
    SpringFocusTab(TabId),
    EditLocation(TabId),
    NewTab(PaneId),
    NewTabFocused,
    CloseTab(TabId),
    CloseFocusedTab,
    FocusTab(TabId),
    SplitHorizontal(PaneId),
    SplitVertical(PaneId),
    SplitHorizontalFocused,
    SplitVerticalFocused,
    ClosePane(PaneId),
    CloseFocusedPane,
    FocusNextPane,
    Open(Vec<PathBuf>),
    Copy(Vec<PathBuf>),
    Cut(Vec<PathBuf>),
    Paste {
        dest: PathBuf,
    },
    Move {
        paths: Vec<PathBuf>,
        dest: PathBuf,
    },
    CopyTo {
        paths: Vec<PathBuf>,
        dest: PathBuf,
    },
    Delete {
        paths: Vec<PathBuf>,
        to_trash: bool,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    CreateFolder {
        parent: PathBuf,
        name: String,
    },
    Select {
        tile: TabId,
        idx: usize,
        shift: bool,
        ctrl: bool,
    },
    ClearSelection(TabId),
    Activate {
        tile: TabId,
        idx: usize,
    },
    SelectAll,
    InvertSelection,
    ToggleHidden,
    SetViewMode(ViewMode),
    SetSort {
        col: SortColumn,
        dir: SortDir,
    },
    OpenGotoBar,
    ClosePalette,
    OpenPalette,
    CloseOverlays,
    ToggleInspector,
    ToggleSidebar,
    OpenBatchRename,
    OpenSettings,
    ToggleDropShelf,
    Refresh(TabId),
    CopyPathToClipboard(PathBuf),
    Undo,
    Redo,
    SwitchProfile(usize),
    DeleteActiveProfile,
}
