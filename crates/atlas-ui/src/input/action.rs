use atlas_model::file_entry::{SortColumn, SortDir};
use atlas_model::location::Location;
use atlas_model::pane::{PaneId, TabId};
use atlas_model::settings::ViewMode;
use atlas_model::tag_cache::TagId;
use std::path::PathBuf;

// reason: several variants are constructed only under other-platform cfg
// (e.g. Mount/UnmountDevice from the Linux udisks sidebar), so per-cfg dead-code
// analysis flags them on Windows. Keep the allow scoped to the enum.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Action {
    /// Navigate to a *filesystem path*, reinterpreted by tab context: inside a
    /// Remote tab the path is treated as a remote path (used by goto results).
    /// For an explicit target use `NavigateLocation`.
    Navigate {
        tab: TabId,
        path: PathBuf,
    },
    NavigateLocation {
        tab: TabId,
        location: Location,
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
    SplitHorizontalFocused,
    SplitVerticalFocused,
    CloseFocusedPane,
    FocusNextPane,
    Open(Vec<PathBuf>),
    Copy(Vec<PathBuf>),
    Cut(Vec<PathBuf>),
    Paste {
        dest: PathBuf,
    },
    /// Resolve the pending transfer-conflict dialog. `None` cancels.
    ResolveTransferConflict(Option<atlas_model::ops::ConflictPolicy>),
    /// Fully-specified transfer (used by the OLE drop queue, which knows the
    /// origin and kind); `Move`/`CopyTo` below are convenience forms.
    Transfer(atlas_model::ops::TransferRequest),
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
    CreateFile {
        parent: PathBuf,
        name: String,
    },
    OpenTerminal {
        dir: PathBuf,
    },
    /// Compress `paths` into a new zip next to the first item (auto-named,
    /// uniqued against collisions).
    CompressToZip(Vec<PathBuf>),
    // ── Recycle Bin ──
    /// Restore the focused pane's selected recycled items to their original paths.
    RestoreSelectedFromRecycleBin,
    /// Permanently delete the focused pane's selected recycled items ($R + $I).
    PurgeSelectedFromRecycleBin,
    /// Permanently delete everything in every accessible recycle bin.
    EmptyRecycleBin,
    Select {
        tile: TabId,
        idx: usize,
        shift: bool,
        ctrl: bool,
    },
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
    /// Begin recording the next keypress as a command-palette shortcut.
    /// (Cancel/set/remove happen via direct methods in `input::shortcuts`.)
    StartCommandShortcutCapture(String),
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
    ToggleSidebarCollapsed,
    OpenBatchRename,
    OpenSettings,
    OpenExtManager,
    /// Toggle the space-analyzer overlay. Currently unreachable: no palette
    /// entry or shortcut constructs it — re-wire when the analyzer UI lands.
    #[allow(dead_code)]
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
    /// Rename a specific profile.
    RenameProfile {
        index: usize,
        name: String,
    },
    /// Open the rename-active-profile dialog (seeded with the current name).
    OpenRenameProfile,
    /// Open the rename dialog for a specific profile.
    OpenRenameProfileAt(usize),
    DuplicateProfile(usize),
    DeleteActiveProfile,
    DeleteProfile(usize),
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
    RenameTag {
        tag: TagId,
        name: String,
    },
    OpenCreateTagOverlay {
        global: bool,
    },
    OpenRenameTagOverlay {
        tag: TagId,
        name: String,
        color: [u8; 3],
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
    // ── Zoom ──
    ZoomIn,
    ZoomOut,
    ZoomReset,
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
    // ── Preview editor ──
    /// Save the current preview editor's content back to its source file.
    SavePreviewFile,
    /// Toggle the preview editor between read-only (disabled) and editable mode.
    /// The preference is persisted per profile.
    TogglePreviewEditMode,
    /// Toggle line-number gutter in the preview code editor.
    /// The preference is persisted per profile.
    TogglePreviewLineNumbers,
    /// Zoom in on the previewed image (same factor as scroll-zoom).
    PreviewZoomIn,
    /// Zoom out on the previewed image.
    PreviewZoomOut,
    /// Reset the previewed image to fit-in-panel zoom and center pan.
    PreviewZoomFit,
}
