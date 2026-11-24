//! Each account's Roblox avatar: the Roblox account signed in on that Windows account (Roblox keeps it in
//! `AppData\Local\Roblox\LocalStorage\appStorage.json`), its headshot from Roblox's thumbnail API, kept in
//! ProgramData\Potato\avatars for a day so a restart doesn't ask again.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use serde_json::Value;

/// The Roblox user id and username signed in on a Windows account.
pub fn roblox_user(local: &Path) -> Option<(u64, String)> {
    parse_app_storage(&std::fs::read_to_string(local.join("Roblox").join("LocalStorage").join("appStorage.json")).ok()?)
}

fn parse_app_storage(text: &str) -> Option<(u64, String)> {
    let v: Value = serde_json::from_str(text).ok()?;
    let id = match &v["UserId"] {
        Value::Number(n) => n.as_u64()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    (id > 0).then(|| (id, v["Username"].as_str().unwrap_or_default().to_string()))
}

/// The picture's address from thumbnails.roblox.com (None while Roblox is still drawing it).
fn image_url(json: &[u8]) -> Option<String> {
    let v: Value = serde_json::from_slice(json).ok()?;
    let d = &v["data"][0];
    (d["state"] == "Completed").then(|| d["imageUrl"].as_str().map(String::from)).flatten()
}

fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::GrayscaleAlpha => buf[..w * h * 2].chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        _ => return None,
    };
    Some(egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba))
}

fn headshot(id: u64) -> Option<Vec<u8>> {
    let cache = crate::engine::config_path().with_file_name("avatars").join(format!("{id}.png"));
    let fresh = std::fs::metadata(&cache).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().unwrap_or_default() < Duration::from_secs(24 * 3600));
    if !fresh {
        let api = format!("https://thumbnails.roblox.com/v1/users/avatar-headshot?userIds={id}&size=150x150&format=Png&isCircular=false");
        if let Some(png) = crate::engine::fetch(&api, 15).and_then(|j| image_url(&j)).and_then(|u| crate::engine::fetch(&u, 15)) {
            let _ = std::fs::create_dir_all(cache.parent().unwrap_or(Path::new(".")));
            let _ = std::fs::write(&cache, &png);
            return Some(png);
        }
    }
    std::fs::read(&cache).ok()   // offline: yesterday's picture beats none
}

#[derive(Default)]
struct Found {
    username: String,
    image: Option<egui::ColorImage>,
}

/// Avatars by Windows account name (lowercase), loaded in the background as accounts show up.
#[derive(Default)]
pub struct Avatars {
    found: Arc<Mutex<HashMap<String, Found>>>,
    asked: HashSet<String>,
    textures: HashMap<String, egui::TextureHandle>,
}

impl Avatars {
    /// Look up every account not looked up yet: `(Windows name, its AppData\Local)`.
    pub fn load(&mut self, ctx: &egui::Context, accounts: impl IntoIterator<Item = (String, PathBuf)>) {
        for (name, local) in accounts {
            let key = name.to_lowercase();
            if !self.asked.insert(key.clone()) {
                continue;
            }
            let (found, ctx) = (self.found.clone(), ctx.clone());
            std::thread::spawn(move || {
                let Some((id, username)) = roblox_user(&local) else { return };
                let image = headshot(id).and_then(|b| decode(&b));
                found.lock().unwrap().insert(key, Found { username, image });
                ctx.request_repaint();
            });
        }
    }

    /// The Roblox username signed in on this Windows account, once known.
    pub fn username(&self, name: &str) -> Option<String> {
        self.found.lock().unwrap().get(&name.to_lowercase()).map(|f| f.username.clone()).filter(|u| !u.is_empty())
    }

    pub fn texture(&mut self, ctx: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
        let key = name.to_lowercase();
        if let Some(t) = self.textures.get(&key) {
            return Some(t.clone());
        }
        let image = self.found.lock().unwrap().get_mut(&key)?.image.take()?;
        let t = ctx.load_texture(format!("avatar-{key}"), image, egui::TextureOptions::LINEAR);
        self.textures.insert(key, t.clone());
        Some(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_roblox_files() {
        assert_eq!(parse_app_storage(r#"{"UserId":"156","Username":"builderman","DisplayName":"x"}"#), Some((156, "builderman".into())));
        assert_eq!(parse_app_storage(r#"{"UserId":42,"Username":"a"}"#), Some((42, "a".into())));
        assert_eq!(parse_app_storage(r#"{"UserId":"","Username":""}"#), None, "signed out");
        assert_eq!(parse_app_storage("not json"), None);
        let done = br#"{"data":[{"targetId":1,"state":"Completed","imageUrl":"https://tr.rbxcdn.com/x/150/150/AvatarHeadshot/Png"}]}"#;
        assert_eq!(image_url(done).as_deref(), Some("https://tr.rbxcdn.com/x/150/150/AvatarHeadshot/Png"));
        assert_eq!(image_url(br#"{"data":[{"targetId":1,"state":"Pending","imageUrl":null}]}"#), None);
    }
}
