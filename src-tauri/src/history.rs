//! 保存したものの一覧(ライブラリ)。保存が終わるたびに1件ずつ記録する。

use crate::settings::{load_json, save_json};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

const HISTORY_FILE: &str = "history.json";
/// 記録しておく最大件数(古いものから消える)
const MAX_ENTRIES: usize = 20_000;

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub id: u64,
    pub title: String,
    pub url: String,
    pub file_path: String,
    pub thumbnail: String,
    /// 保存したもの: video / audio / thumbnail / subtitles / info / file
    pub kind: String,
    /// 保存した時刻(UNIX秒)
    pub saved_at: u64,
    pub size: u64,
    /// ファイルが今もあるか(一覧を返す時に調べ直す)
    #[serde(default)]
    pub exists: bool,
    /// ファイルが入っているドライブ(外付けなど)が、今つながっていない
    #[serde(default)]
    pub drive_missing: bool,
    /// 投稿者(チャンネル名)
    #[serde(default)]
    pub uploader: String,
    /// ごみ箱へ移した(ごみ箱から戻せるうちは記録を残す)
    #[serde(default)]
    pub trashed: bool,
    /// 長さ(秒)と画質(高さ)。分からなければ 0
    #[serde(default)]
    pub duration: u64,
    #[serde(default)]
    pub height: u64,
}

/// パスのドライブ(Windows の「E:\」や、Mac・Linux の /Volumes/… /media/… の入口)が今あるか
fn drive_present(path: &str) -> bool {
    let p = std::path::Path::new(path);
    if cfg!(windows) {
        return p.components().next().map(|c| std::path::Path::new(&format!("{}\\", c.as_os_str().to_string_lossy())).exists()).unwrap_or(true);
    }
    // /Volumes/名前 や /media/ユーザー/名前 、/mnt/名前 までが外付けの入口
    let parts: Vec<_> = p.components().take(4).collect();
    let mount_depth = match parts.get(1).map(|c| c.as_os_str().to_string_lossy().into_owned()).as_deref() {
        Some("Volumes") | Some("mnt") => 3,
        Some("media") | Some("run") => 4,
        _ => return true,
    };
    let mount: std::path::PathBuf = p.components().take(mount_depth).collect();
    mount.exists()
}

pub struct History {
    entries: Mutex<Vec<Entry>>,
}

impl History {
    pub fn load() -> Self {
        Self { entries: Mutex::new(load_json(HISTORY_FILE)) }
    }

    pub fn add(&self, mut entry: Entry) {
        entry.size = std::fs::metadata(&entry.file_path).map(|m| m.len()).unwrap_or(0);
        let mut entries = self.entries.lock().unwrap();
        // 同じファイルを保存し直した時は、古い記録を置き換える
        entries.retain(|e| e.file_path != entry.file_path);
        entries.push(entry);
        if entries.len() > MAX_ENTRIES {
            let extra = entries.len() - MAX_ENTRIES;
            entries.drain(..extra);
        }
        save_json(HISTORY_FILE, &*entries);
    }

    /// 消した記録を戻す(元に戻す用)
    pub fn restore(&self, restored: Vec<Entry>) {
        let mut entries = self.entries.lock().unwrap();
        for entry in restored {
            if !entries.iter().any(|e| e.id == entry.id) {
                entries.push(entry);
            }
        }
        entries.sort_by_key(|e| e.saved_at);
        save_json(HISTORY_FILE, &*entries);
    }

    /// 印だけ付け直す(ごみ箱から戻した時など)
    pub fn set_trashed(&self, id: u64, trashed: bool) {
        let mut entries = self.entries.lock().unwrap();
        if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
            e.trashed = trashed;
        }
        save_json(HISTORY_FILE, &*entries);
    }

    /// ファイルの名前を変えた時に、記録の場所も書き換える
    pub fn set_path(&self, id: u64, path: &str) {
        let mut entries = self.entries.lock().unwrap();
        if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
            e.file_path = path.to_string();
        }
        save_json(HISTORY_FILE, &*entries);
    }

    /// 新しい順に返す。消されたファイルも印を付けて返す。
    pub fn list(&self) -> Vec<Entry> {
        let mut list = self.entries.lock().unwrap().clone();
        for entry in &mut list {
            entry.exists = std::path::Path::new(&entry.file_path).exists();
            entry.drive_missing = !entry.exists && !drive_present(&entry.file_path);
        }
        list.reverse();
        list
    }

    /// 記録を消す。`delete_file` なら保存したファイルも消す。
    pub fn remove(&self, ids: &[u64], delete_file: bool) -> Result<(), String> {
        let mut entries = self.entries.lock().unwrap();
        let mut errors = Vec::new();
        let mut trashed = Vec::new();
        if delete_file {
            for entry in entries.iter().filter(|e| ids.contains(&e.id)) {
                let path = std::path::Path::new(&entry.file_path);
                // 消したファイルはごみ箱へ(間違えても戻せるように)。ごみ箱が使えない環境では削除する
                if path.exists() {
                    if trash::delete(path).is_ok() {
                        trashed.push(entry.id);
                    } else if let Err(e) = std::fs::remove_file(path) {
                        errors.push(format!("{}: {e}", entry.file_path));
                    }
                }
            }
        }
        // ごみ箱へ移したものは「ごみ箱にある」として記録を残す(戻せるように)
        for entry in entries.iter_mut().filter(|e| trashed.contains(&e.id)) {
            entry.trashed = true;
        }
        entries.retain(|e| !ids.contains(&e.id) || trashed.contains(&e.id));
        save_json(HISTORY_FILE, &*entries);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }
}
