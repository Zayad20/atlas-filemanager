fn main() -> anyhow::Result<()> {
    atlas_ui::run_vault_diagnostic(
        std::env::args_os()
            .nth(1)
            .map(std::path::PathBuf::from)
            .expect("pass an isolated output directory"),
    )
}
