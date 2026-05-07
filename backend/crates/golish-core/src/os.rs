/// Open a URL in the system default browser.
pub fn open_url(url: &str) -> std::io::Result<()> {
    golish_platform::open::open_url(url)
}

/// Open a directory in the system file manager.
pub fn reveal_path(path: &std::path::Path) -> std::io::Result<()> {
    golish_platform::open::reveal_path(path)
}
