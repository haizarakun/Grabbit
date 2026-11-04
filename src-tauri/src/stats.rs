//! 記録と統計: 外への通信の一覧、失敗の統計。
//! どちらもパソコンの中だけに置き、外へは送らない。

use crate::settings::{load_json, save_json};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;

/// 外への通信の記録(新しい方 500 件。アプリを閉じると消える)
static CONNECTIONS: Mutex<VecDeque<Connection>> = Mutex::new(VecDeque::new());

#[derive(Serialize, Clone)]
pub struct Connection {
    pub time: u64,
    pub host: String,
    /// 何のための通信か(更新・ダウンロード・通知 など)
    pub purpose: String,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn host_of(url: &str) -> String {
    url.split('/').nth(2).unwrap_or(url).split('@').last().unwrap_or_default().to_string()
}

/// 外へつないだことを記録する
pub fn connection(url: &str, purpose: &str) {
    let mut list = CONNECTIONS.lock().unwrap();
    list.push_back(Connection { time: now(), host: host_of(url), purpose: purpose.to_string() });
    while list.len() > 500 {
        list.pop_front();
    }
}

pub fn connections() -> Vec<Connection> {
    CONNECTIONS.lock().unwrap().iter().rev().cloned().collect()
}

// ───────────── 失敗の統計 ─────────────

const ERRORS_FILE: &str = "error_stats.json";

#[derive(Serialize, Deserialize, Clone)]
pub struct Failure {
    pub time: u64,
    pub site: String,
    /// 失敗の種類: login / geo / network / unavailable / blocked / other
    pub kind: String,
}

/// 失敗の理由を、大まかな種類に分ける
pub fn classify(message: &str) -> &'static str {
    let m = message.to_lowercase();
    if crate::download::is_login_required(message) {
        "login"
    } else if crate::download::is_geo_blocked(message) {
        "geo"
    } else if crate::download::is_network_error(message) {
        "network"
    } else if m.contains("403") || m.contains("429") || m.contains("bot") {
        "blocked"
    } else if m.contains("unavailable") || m.contains("removed") || m.contains("private") || m.contains("404") {
        "unavailable"
    } else {
        "other"
    }
}

pub fn record_failure(url: &str, message: &str) {
    let mut list: Vec<Failure> = load_json(ERRORS_FILE);
    let site = crate::methods::site_key(url);
    list.push(Failure { time: now(), site, kind: classify(message).into() });
    let extra = list.len().saturating_sub(2000);
    list.drain(..extra);
    save_json(ERRORS_FILE, &list);
}

/// 画面用にまとめる: サイト別・時間帯別(0〜23時・この PC の時刻)・種類別
pub fn summary() -> serde_json::Value {
    use chrono::{Local, TimeZone, Timelike};
    let list: Vec<Failure> = load_json(ERRORS_FILE);
    let mut by_site = std::collections::BTreeMap::<String, u32>::new();
    let mut by_kind = std::collections::BTreeMap::<String, u32>::new();
    let mut by_hour = [0u32; 24];
    for f in &list {
        *by_site.entry(f.site.clone()).or_default() += 1;
        *by_kind.entry(f.kind.clone()).or_default() += 1;
        if let Some(t) = Local.timestamp_opt(f.time as i64, 0).single() {
            by_hour[t.hour() as usize] += 1;
        }
    }
    let mut sites: Vec<_> = by_site.into_iter().collect();
    sites.sort_by(|a, b| b.1.cmp(&a.1));
    sites.truncate(15);
    serde_json::json!({ "total": list.len(), "sites": sites, "kinds": by_kind, "hours": by_hour })
}

pub fn clear_failures() {
    save_json(ERRORS_FILE, &Vec::<Failure>::new());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_are_classified() {
        assert_eq!(classify("ERROR: Sign in to confirm your age"), "login");
        assert_eq!(classify("HTTP Error 429: Too Many Requests"), "blocked");
        assert_eq!(classify("Video unavailable"), "unavailable");
    }
}
