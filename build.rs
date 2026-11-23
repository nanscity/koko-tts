fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Kokoro TTS");
        res.set("FileDescription", "Kokoro-82M Text-to-Speech System Tray App");
        if let Err(e) = res.compile() {
            eprintln!("Warning: failed to compile Windows resource: {}", e);
        }
    }
}
