// Build scripts compile for the HOST, so `cfg(windows)` below selects the
// Windows host; the `CARGO_CFG_TARGET_OS` check additionally guards the
// target. On other hosts the crate reference stays compiled out, matching
// the target-gated `[target.'cfg(windows)'.build-dependencies]` entry.
#[cfg(windows)]
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/fadb.ico")
            .compile()
            .expect("embed the Windows exe icon");
    }
}

#[cfg(not(windows))]
fn main() {}
