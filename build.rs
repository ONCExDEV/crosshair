fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("res/icon.ico");
        res.set("ProductName", "Crosshair");
        res.set("FileDescription", "Crosshair overlay");
        res.set("LegalCopyright", "MIT License");
        res.compile().expect("failed to embed Windows resources");
    }
}
