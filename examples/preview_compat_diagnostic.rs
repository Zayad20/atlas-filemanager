//! Run prepare-preview-compat.py first, then pass the same artifact directory.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::path::absolute(
        std::env::args_os()
            .nth(1)
            .ok_or("pass an output directory")?,
    )?;
    std::env::set_var("ATLAS_PREVIEW_LAB", &directory);
    std::env::set_var("ATLAS_CONFIG_DIR", directory.join("config"));
    std::env::set_var("ATLAS_PREVIEW_COMPAT_LAB", "1");
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .max_blocking_threads(16)
                .enable_all()
                .build()
                .unwrap();
            atlas_ui::run(
                runtime.handle().clone(),
                std::time::Instant::now(),
                None,
                Vec::new(),
            );
        })?
        .join()
        .map_err(|_| "native lab panicked")?;
    if directory.join("failed.txt").exists() || !directory.join("passed.json").exists() {
        return Err("native checks failed; see failed.txt".into());
    }
    Ok(())
}
