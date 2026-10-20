#[cfg(windows)]
fn main() {
    let mut res = winres::WindowsResource::new();
    res.set_icon("../../assets/icon.ico");
    res.set("FileDescription", "BitTorrent Desktop Client");
    res.set("ProductName", "BitTorrent Client");
    res.set("OriginalFilename", "torrent_client.exe");
    res.set("LegalCopyright", "Copyright (c) 2026");
    if let Err(e) = res.compile() {
        eprintln!("Failed to compile Windows resources: {}", e);
    }
}

#[cfg(not(windows))]
fn main() {}
