// Resolve WinFsp only on first mount so Atlas still starts without its driver.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let dll = match arch.as_str() {
        "x86_64" => "winfsp-x64.dll",
        "aarch64" => "winfsp-a64.dll",
        "x86" => "winfsp-x86.dll",
        _ => return,
    };
    println!("cargo:rustc-link-lib=dylib=delayimp");
    println!("cargo:rustc-link-arg=/DELAYLOAD:{dll}");
}
