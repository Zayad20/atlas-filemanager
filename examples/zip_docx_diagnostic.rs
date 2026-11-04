//! Hidden native regression run. Pass its artifact directory as the first argument.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("pass an output directory")?;
    let directory = std::path::absolute(directory)?;
    std::env::set_var("ATLAS_PREVIEW_LAB", &directory);
    std::env::set_var("ATLAS_CONFIG_DIR", directory.join("config"));
    std::env::set_var("ATLAS_ZIP_DOCX_LAB", "1");
    // GPUI debug view trees need more than Windows' default 1 MiB stack.
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .max_blocking_threads(32)
                .enable_all()
                .build()
                .expect("diagnostic runtime");
            atlas_ui::run(
                runtime.handle().clone(),
                std::time::Instant::now(),
                None,
                Vec::new(),
            );
        })?
        .join()
        .map_err(|_| "diagnostic panicked")?;
    Ok(())
}
