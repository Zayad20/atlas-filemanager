//! Hidden native regression check; uses an isolated configuration and fixtures.
fn main() -> anyhow::Result<()> {
    let directory = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .expect("pass a diagnostic output directory");
    atlas_ui::ui::shell_context_menu::diagnostic::run(directory)
}
