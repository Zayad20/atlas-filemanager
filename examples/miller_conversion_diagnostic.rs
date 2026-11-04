//! Native regression run for Miller wheel events and conversion/preview crashes.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("pass an output directory")?;
    let directory = std::path::absolute(directory)?;
    std::env::set_var("ATLAS_PREVIEW_LAB", &directory);
    std::env::set_var("ATLAS_CONFIG_DIR", directory.join("config"));
    std::env::set_var("ATLAS_MILLER_CONVERT_LAB", "1");
    std::env::set_var("ATLAS_PERF", "1");
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .max_blocking_threads(8)
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
    if directory.join("failed.txt").exists() {
        return Err("native checks failed; see failed.txt".into());
    }
    if !directory.join("passed.json").exists() {
        return Err("native checks did not finish".into());
    }
    Ok(())
}
