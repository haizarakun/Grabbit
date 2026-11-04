//! アプリ自身の通信(部品の取得・通知・曲の情報など)の共通の入口。
//! 「プロキシだけを通す」設定の時は、プロキシが無ければつながない。

use std::sync::RwLock;
use std::time::Duration;

struct Route {
    proxy: String,
    proxy_only: bool,
    mirror: String,
}

static ROUTE: RwLock<Route> = RwLock::new(Route { proxy: String::new(), proxy_only: false, mirror: String::new() });

/// 設定が変わったら呼ぶ
pub fn configure(settings: &crate::settings::Settings) {
    let mut route = ROUTE.write().unwrap();
    route.proxy = settings.proxy.trim().to_string();
    route.proxy_only = settings.proxy_only;
    route.mirror = settings.github_mirror.trim().trim_end_matches('/').to_string();
}

/// 通信に使う道具を作る。プロキシの指定があれば通す
pub fn client(timeout: Duration, user_agent: &str) -> Result<reqwest::Client, String> {
    let route = ROUTE.read().unwrap();
    let mut builder = reqwest::Client::builder().timeout(timeout).user_agent(user_agent);
    if !route.proxy.is_empty() {
        builder = builder.proxy(reqwest::Proxy::all(&route.proxy).map_err(|_| "プロキシの書き方が正しくありません".to_string())?);
    } else if route.proxy_only {
        return Err("「プロキシだけを通す」がオンですが、プロキシが入っていません".into());
    }
    builder.build().map_err(|e| e.to_string())
}

/// プロキシだけを通す設定なのに、プロキシが無いか(ダウンロードを始める前の確認用)
pub fn blocked_without_proxy() -> bool {
    let route = ROUTE.read().unwrap();
    route.proxy_only && route.proxy.is_empty()
}

/// GitHub の配布物の URL を、代わりの入口(ミラー)経由にする。
/// 中身はハッシュで確かめているので、入口が変わっても差し替えられた物は使わない
pub fn via_mirror(url: &str) -> String {
    let route = ROUTE.read().unwrap();
    // 照合用のハッシュは必ず GitHub から直接取る(ミラーが中身とハッシュを両方すり替えても見抜けるように)
    let is_checksum = ["SHA2-256SUMS", "checksums.sha256", ".sha256sum", ".sha256"].iter().any(|n| url.ends_with(n));
    if route.mirror.starts_with("https://") && url.starts_with("https://github.com/") && !is_checksum {
        format!("{}/{}", route.mirror, url)
    } else {
        url.to_string()
    }
}
