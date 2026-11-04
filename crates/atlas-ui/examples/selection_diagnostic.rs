//! Hidden native selection and scroll regression checks with isolated fixtures.
fn main() -> anyhow::Result<()> {
    let directory = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .expect("pass a diagnostic output directory");
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || atlas_ui::ui::views::selection_diagnostic::run(directory))?
        .join()
        .map_err(|_| anyhow::anyhow!("selection diagnostic panicked"))?
}
