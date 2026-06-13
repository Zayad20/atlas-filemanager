#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
use crate::state::file_entry::{SortColumn, SortDir};
use crate::state::location::Location;
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
    /// Resolve the pending transfer-conflict dialog. `None` cancels.
    ResolveTransferConflict(Option<crate::state::ops::ConflictPolicy>),
    /// Fully-specified transfer (used by the OLE drop queue, which knows the
    /// origin and kind); `Move`/`CopyTo` below are convenience forms.
    Transfer(crate::state::ops::TransferRequest),
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
        location: Location,
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
    /// Switch the active profile's theme by registered theme name.
    SetTheme(String),
    SetSort {
        col: SortColumn,
        dir: SortDir,
    },
    OpenGotoBar,
    ClosePalette,
    OpenPalette,
    SubmitExtPalette(String),
    /// Pin/unpin a command-palette command (by stable id) to the "Quick" section.
    ToggleCommandPin(String),
    OpenSearchInPlace,
    CloseSearchInPlace,
    ToggleSearchInSubfolders,
    // ── Per-pane bottom-bar toggles ──
    ToggleFoldersFirst(TabId),
    ToggleShowFiles(TabId),
    ToggleShowFolders(TabId),
    CloseOverlays,
    ToggleInspector,
    ToggleSidebar,
    OpenBatchRename,
    OpenSettings,
    ToggleAnalyzer,
    Refresh(TabId),
    CopyPathToClipboard(PathBuf),
    Undo,
    Redo,
    SwitchProfile(usize),
    /// Create a new profile with the given name, seeded from the current view, and
    /// switch to it.
    CreateProfile(String),
    /// Rename the active profile.
    RenameActiveProfile(String),
    /// Open the rename-active-profile dialog (seeded with the current name).
    OpenRenameProfile,
    DeleteActiveProfile,
    ToggleSidebarSection(usize),
    MoveSidebarSectionUp(usize),
    MoveSidebarSectionDown(usize),
    SetSidebarSectionHidden {
        index: usize,
        hidden: bool,
    },
    ResetSidebarSections,
    RemoveBookmark(usize),
    AddBookmark {
        name: String,
        path: PathBuf,
    },
    // ── Tags ──
    /// Create a new tag. `profile = None` makes it global (visible in every profile).
    CreateTag {
        name: String,
        color: [u8; 3],
        profile: Option<String>,
    },
    OpenCreateTagOverlay {
        global: bool,
    },
    CloseCreateTagOverlay,
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
    // ── Removable disks (udisks2) ──
    /// Mount a block device (e.g. `/dev/sdb1`) via udisksctl, then navigate to it.
    MountDevice(String),
    /// Unmount/eject a block device via udisksctl.
    UnmountDevice(String),
    // ── Archives ──
    /// Navigate inside an archive, entering the `inner` directory within it.
    /// Pass `inner = PathBuf::new()` to open at the archive root.
    NavigateIntoArchive {
        tab: TabId,
        archive: PathBuf,
        inner: PathBuf,
    },
    /// Extract the selected virtual entries (`archive_root` + `inner_path` pairs)
    /// to the given destination directory.
    ExtractArchiveTo {
        tab: TabId,
        items: Vec<(PathBuf, PathBuf)>,
        dest: PathBuf,
    },
    // ── Cloud / network mounting (feature `cloud`) ──
    #[cfg(feature = "cloud")]
    Cloud(crate::app::cloud::CloudAction),
    // ── Extensions ──
    Ext(crate::app::ext::ExtAction),
}
