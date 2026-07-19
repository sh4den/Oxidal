fn main() {
    println!("cargo:rerun-if-changed=packaging/windows/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("packaging/windows/icon.ico");
        res.compile().expect("failed to embed windows resources");
    }
}
