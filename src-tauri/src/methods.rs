//! サイトごとに「うまくいった取り方」を覚える。
//! 例: このサイトはログイン情報が要る / ブラウザ偽装で通った / 予備のプロキシが要る / 画像用の方法で取れた。
//! 次からは、覚えた方法を最初から使う(失敗して遠回りする時間を減らす)。

use crate::settings::{load_json, save_json};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

const FILE: &str = "site_methods.json";

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(default)]
pub struct Method {
    /// ログイン情報を最初から使う
    pub cookies: bool,
    /// ブラウザ偽装を最初から使う
    pub impersonate: bool,
    /// 地域制限用のプロキシを最初から使う
    pub proxy: bool,
    /// 動画用ではなく別の方法で取れた: "" / "gallery-dl" / "direct"
    pub engine: String,
}

impl Method {
    pub fn is_plain(&self) -> bool {
        *self == Method::default()
    }
}

static CACHE: Mutex<Option<BTreeMap<String, Method>>> = Mutex::new(None);

fn with_map<R>(f: impl FnOnce(&mut BTreeMap<String, Method>) -> R) -> R {
    let mut cache = CACHE.lock().unwrap();
    let map = cache.get_or_insert_with(|| load_json(FILE));
    f(map)
}

/// URL からサイトの名前(example.com の形)を取り出す
pub fn site_key(url: &str) -> String {
    let host = url.split('/').nth(2).unwrap_or_default().split(':').next().unwrap_or_default().to_lowercase();
    let parts: Vec<&str> = host.split('.').collect();
    let keep = if parts.len() >= 3 && parts[parts.len() - 1].len() == 2 && ["co", "com", "ne", "or", "ac", "go"].contains(&parts[parts.len() - 2]) { 3 } else { 2 };
    parts[parts.len().saturating_sub(keep)..].join(".")
}

/// 覚えている方法(無ければ普通の方法)
pub fn get(url: &str) -> Method {
    with_map(|map| map.get(&site_key(url)).cloned().unwrap_or_default())
}

/// うまくいった方法を覚える。普通の方法で取れたら、覚えていたものを消す
pub fn remember(url: &str, method: Method) {
    let key = site_key(url);
    if key.is_empty() {
        return;
    }
    with_map(|map| {
        let changed = if method.is_plain() { map.remove(&key).is_some() } else { map.insert(key, method.clone()).as_ref() != Some(&method) };
        if changed {
            save_json(FILE, &*map);
        }
    });
}

/// 覚えた方法の一覧(設定画面で見せる・消す用)
pub fn list() -> BTreeMap<String, Method> {
    with_map(|map| map.clone())
}

/// 覚えた方法を消す(空なら全部)
pub fn forget(site: &str) {
    with_map(|map| {
        if site.is_empty() {
            map.clear();
        } else {
            map.remove(site);
        }
        save_json(FILE, &*map);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_keys() {
        assert_eq!(site_key("https://www.youtube.com/watch?v=x"), "youtube.com");
        assert_eq!(site_key("https://video.example.co.jp:8080/a"), "example.co.jp");
    }
}
