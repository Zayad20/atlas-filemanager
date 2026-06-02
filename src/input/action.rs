use crate::state::file_entry::{SortColumn, SortDir};
use crate::state::pane::{PaneId, TabId};
use crate::state::settings::ViewMode;
use crate::state::tag_cache::TagId;
use std::path::PathBuf;

#[derive(Debug, Clone)]

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
    NewTabWithPath {
        pane: PaneId,
        path: PathBuf,
    },
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
    /// Resolve the pending paste-conflict dialog. `None` cancels the paste.
    ResolvePasteConflict(Option<crate::state::ops::Resolution>),
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
    MillerInit {
        tile: TabId,
        path: PathBuf,
    },
    MillerSelect {
        tile: TabId,
        column: usize,
        idx: usize,
    },
    MillerActivate {
        tile: TabId,
        column: usize,
        idx: usize,
    },
    MillerFocusColumn {
        tile: TabId,
        column: usize,
    },
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
    ToggleAnalyzer,
    ToggleDropShelf,
    Refresh(TabId),
    CopyPathToClipboard(PathBuf),
    Undo,
    Redo,
    SwitchProfile(usize),
    DeleteActiveProfile,
    ToggleSidebarSection(usize),
    MoveSidebarSectionUp(usize),
    MoveSidebarSectionDown(usize),
    RemoveBookmark(usize),
    AddBookmark { name: String, path: PathBuf },
    // ── Tags ──
    /// Create a new tag. `profile = None` makes it global (visible in every profile).
    CreateTag {
        name: String,
        color: [u8; 3],
        profile: Option<String>,
    },
    DeleteTag(TagId),
    AssignTagToFiles {
        tag: TagId,
        paths: Vec<PathBuf>,
    },
    UnassignTagFromFiles {
        tag: TagId,
        paths: Vec<PathBuf>,
    },
    /// Open a virtual tab listing every file carrying `tag`.
    OpenTagResults {
        pane: PaneId,
        tag: TagId,
    },
}
