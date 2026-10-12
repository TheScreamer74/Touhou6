//! Durable score.dat storage shared by native and browser builds.
use th06_formats::score::ScoreData;

pub fn load_progress(path: &str) -> ScoreData {
    load_bytes(path)
        .and_then(|bytes| ScoreData::parse(&bytes).ok())
        .unwrap_or_default()
}

pub fn store_progress(data: &ScoreData, path: &str) -> Result<(), String> {
    store_bytes(path, &data.encode())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_bytes(path: &str) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn store_bytes(path: &str, bytes: &[u8]) -> Result<(), String> {
    let path = std::path::Path::new(path);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut temp = path.as_os_str().to_os_string();
    temp.push(".tmp");
    let temp = std::path::PathBuf::from(temp);
    // Preserve an existing original save before the first write by this port.
    let mut backup = path.as_os_str().to_os_string();
    backup.push(".bak");
    let backup = std::path::PathBuf::from(backup);
    if path.exists() && !backup.exists() {
        std::fs::copy(path, &backup).map_err(|e| e.to_string())?;
    }
    std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

#[cfg(target_arch = "wasm32")]
pub fn load_bytes(path: &str) -> Option<Vec<u8>> {
    let text = storage()?.get_item(&format!("th06:{path}")).ok()??;
    if text.len() % 2 != 0 || text.len() > (7 * 0x69780 + 80) * 2 {
        return None;
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|p| {
            let digit = |c: u8| match c {
                b'0'..=b'9' => Some(c - b'0'),
                b'a'..=b'f' => Some(c - b'a' + 10),
                _ => None,
            };
            Some(digit(p[0])? * 16 + digit(p[1])?)
        })
        .collect()
}

#[cfg(target_arch = "wasm32")]
pub fn store_bytes(path: &str, bytes: &[u8]) -> Result<(), String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 15) as usize] as char);
    }
    let storage = storage().ok_or("Browser storage is unavailable")?;
    storage
        .set_item(&format!("th06:{path}"), &text)
        .map_err(|_| "Could not save progress: browser storage is full or disabled".into())
}
