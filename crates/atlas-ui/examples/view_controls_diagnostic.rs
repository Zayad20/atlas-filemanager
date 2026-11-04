//! Hidden, isolated visual check of footer controls and listing transitions.
fn main() -> anyhow::Result<()> {
    let directory = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .expect("pass a diagnostic output directory");
    atlas_ui::ui::views::view_controls_diagnostic::run(directory)
}
