// The native GPUI workspace constructs its scene graph on the Windows UI thread.
// Reserve an 8 MiB stack for the desktop executable only. This does not commit
// 8 MiB of RAM and does not change the CLI, probe, or non-Windows build.
// Windows MSVC and MinGW/GNU linkers require different stack-size syntax.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
            Ok("msvc") => println!("cargo:rustc-link-arg-bin=slicebridge-desktop=/STACK:8388608"),
            Ok("gnu") => {
                println!("cargo:rustc-link-arg-bin=slicebridge-desktop=-Wl,--stack,8388608")
            }
            _ => {}
        }
    }
}
