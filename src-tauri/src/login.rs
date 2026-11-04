//! アプリ内ログイン。
//! Chrome / Edge は 2024 年以降ログイン情報(cookie)を外から読めなくなったため、
//! 同種のアプリ(Seal など)と同じく「アプリの中の小さなブラウザでログインしてもらい、
//! その cookie を cookies.txt に書き出してダウンロードに使う」方式にする。
//!
//! ログイン画面のウィンドウには、アプリの機能を呼ぶ権限を一切与えない(capabilities は main だけ)。

use crate::settings::data_dir;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent};

const LABEL: &str = "login";

/// ログイン情報を最後に使った時刻を記録する(使わないまま長く経ったら消すため)
pub fn touch() {
    let _ = std::fs::write(data_dir().join("login_last_used"), now_secs().to_string());
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 決めた日数使わなかったログイン情報を消す。消したら true
pub fn expire_unused(days: u32) -> bool {
    if days == 0 || !cookie_file().exists() {
        return false;
    }
    let last: u64 = std::fs::read_to_string(data_dir().join("login_last_used")).ok().and_then(|t| t.trim().parse().ok()).unwrap_or_else(|| {
        touch(); // 記録が無ければ今から数える
        now_secs()
    });
    if now_secs().saturating_sub(last) < u64::from(days) * 86_400 {
        return false;
    }
    forget("");
    let _ = std::fs::remove_file(encrypted_file());
    true
}

/// アプリ内ログインで集めた cookie の保存先
pub fn cookie_file() -> PathBuf {
    data_dir().join("login_cookies.txt")
}

/// ログインした印になる cookie か(見ただけで付く追跡用の cookie と区別する)。
/// 主なサイトの名前と、session / auth / token などを含む名前を見る。
fn is_login_cookie(name: &str) -> bool {
    const KNOWN: &[&str] = &[
        "SAPISID", "__Secure-3PSID", "LOGIN_INFO", "user_session", "auth_token", "sessionid", "auth-token",
        "c_user", "FANBOXSESSID", "session_id", "SESSDATA", "PHPSESSID",
    ];
    let lower = name.to_lowercase();
    KNOWN.contains(&name) || ["session", "auth", "token", "login"].iter().any(|w| lower.contains(w))
}

/// ログイン済みのサイトと、ログインの期限
#[derive(serde::Serialize)]
pub struct LoginSite {
    pub site: String,
    /// いちばん早く切れるログイン情報の期限(UNIX秒)。0 はブラウザを閉じるまで(期限なし)
    pub expires: i64,
}

/// ログイン済みのサイトの一覧(期限が早い順)
pub fn logged_in_sites() -> Vec<LoginSite> {
    let mut by_site: BTreeMap<String, i64> = BTreeMap::new();
    for e in read_entries().into_values().filter(|e| is_login_cookie(&e.name)) {
        let earliest = by_site.entry(site_of(&e.domain)).or_insert(e.expires);
        if e.expires > 0 && (*earliest == 0 || e.expires < *earliest) {
            *earliest = e.expires;
        }
    }
    let mut sites: Vec<LoginSite> = by_site.into_iter().map(|(site, expires)| LoginSite { site, expires }).collect();
    sites.sort_by_key(|s| if s.expires == 0 { i64::MAX } else { s.expires });
    sites
}

/// 指定したサイトのログイン情報を消す(空なら全部)
pub fn forget(site: &str) {
    let kept: BTreeMap<_, _> = read_entries().into_iter().filter(|(_, e)| !site.is_empty() && site_of(&e.domain) != site).collect();
    write_entries(&kept);
}

/// ログイン用の小さなブラウザを開く。閉じた時に cookie を保存して "login-saved" を送る。
pub fn open(app: &AppHandle, url: &str, title: &str) -> Result<(), String> {
    let url: Url = url.parse().map_err(|_| "URL が正しくありません".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("http / https の URL だけ開けます".into());
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.navigate(url);
        let _ = window.set_focus();
        return Ok(());
    }
    // まず空のページで開き、ログイン情報をこのアプリの保存内容にそろえてから目的のページへ移る
    // (「ログアウト」で消したサイトが、小さなブラウザ側に残ってログインしたままにならないように)
    let blank: Url = "about:blank".parse().map_err(|_| "内部エラー".to_string())?;
    let title = if title.trim().is_empty() { "Login" } else { title };
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(blank))
        .title(title)
        .inner_size(900.0, 760.0)
        // ログイン画面からは普通の Web ページ以外(file: や独自のスキーム)へは移動させない
        .on_navigation(|url| matches!(url.scheme(), "http" | "https" | "about"))
        .build()
        .map_err(|e| e.to_string())?;
    let sync_window = window.clone();
    tauri::async_runtime::spawn(async move {
        sync_cookies_into(&sync_window);
        let _ = sync_window.navigate(url);
    });

    let app = app.clone();
    let target = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            // Windows では画面の処理中に cookie を読むと止まるので、別スレッドで読んでから閉じる
            api.prevent_close();
            let (app, window) = (app.clone(), target.clone());
            // cookie が読めずに待ち続けても、3 秒で必ず閉じる
            std::thread::spawn(move || {
                let (tx, rx) = std::sync::mpsc::channel();
                let reader = window.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(reader.cookies().map(|cookies| save(&cookies)).unwrap_or(false));
                });
                let logged_in = rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap_or(false);
                let _ = window.destroy();
                let _ = app.emit("login-saved", logged_in);
            });
        }
    });
    Ok(())
}

// ───────────── 保存時の暗号化(Windows) ─────────────
// アプリを閉じている間は、ログイン情報を Windows の暗号化(DPAPI: このユーザーでしか開けない)で保存する。
// 動いている間だけ、ダウンロード機能が読める形(cookies.txt)に戻す。

fn encrypted_file() -> PathBuf {
    data_dir().join("login_cookies.enc")
}

/// 起動時: 暗号化して保存したものを、読める形に戻す
pub fn unlock_at_startup() {
    let plain = cookie_file();
    let sealed = encrypted_file();
    // 前回きちんと閉じられなかった時は、読める形の方が新しいのでそれを使う
    if plain.exists() {
        return;
    }
    if let Ok(data) = std::fs::read(&sealed) {
        if let Some(text) = dpapi::unprotect(&data) {
            let _ = std::fs::write(&plain, text);
            restrict_permissions(&plain);
        }
    }
}

/// 終了時: 読める形のものを暗号化して保存し、読める形は消す(暗号化できない環境ではそのまま)
pub fn lock_at_exit() {
    let plain = cookie_file();
    let Ok(text) = std::fs::read(&plain) else { return };
    if let Some(sealed) = dpapi::protect(&text) {
        if std::fs::write(encrypted_file(), sealed).is_ok() {
            let _ = std::fs::remove_file(&plain);
        }
    }
}

/// ほかのファイル(設定・記録)の暗号化にも同じしくみを使う
pub fn protect_bytes(data: &[u8]) -> Option<Vec<u8>> {
    dpapi::protect(data)
}

pub fn unprotect_bytes(data: &[u8]) -> Option<Vec<u8>> {
    dpapi::unprotect(data)
}

fn restrict_permissions(_path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(_path, std::fs::Permissions::from_mode(0o600));
    }
}

#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB};

    fn run(data: &[u8], protect: bool) -> Option<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: 入力は data を指すだけで書き換えない。出力は Windows が確保し、読んだ後 LocalFree で返す
        let ok = unsafe {
            if protect {
                CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut output)
            } else {
                CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut output)
            }
        };
        if ok == 0 || output.pbData.is_null() {
            return None;
        }
        let result = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
        unsafe { LocalFree(output.pbData as _) };
        Some(result)
    }

    pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
        run(data, true)
    }

    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        run(data, false)
    }
}

#[cfg(not(windows))]
mod dpapi {
    // Windows 以外では暗号化しない(ファイルの権限で本人以外は読めないようにしている)
    pub fn protect(_data: &[u8]) -> Option<Vec<u8>> {
        None
    }

    pub fn unprotect(_data: &[u8]) -> Option<Vec<u8>> {
        None
    }
}

// ───────────── cookies.txt(Netscape 形式)の読み書き ─────────────

struct Entry {
    domain: String,
    path: String,
    secure: bool,
    expires: i64,
    name: String,
    value: String,
}

/// 同じ (ドメイン, パス, 名前) は 1 つにまとめる
type Key = (String, String, String);

fn site_of(domain: &str) -> String {
    let parts: Vec<&str> = domain.trim_start_matches('.').split('.').collect();
    // example.co.jp のような国別の2段目(co / com / ne など)は3段まで見る
    let second_level = ["co", "com", "ne", "net", "or", "org", "ac", "go", "gov", "edu"];
    let n = parts.len();
    let keep = if n >= 3 && parts[n - 1].len() == 2 && second_level.contains(&parts[n - 2]) { 3 } else { 2 };
    parts[n.saturating_sub(keep)..].join(".")
}

fn read_entries() -> BTreeMap<Key, Entry> {
    let text = std::fs::read_to_string(cookie_file()).unwrap_or_default();
    text.lines()
        .filter(|line| !line.starts_with('#') || line.starts_with("#HttpOnly_"))
        .filter_map(|line| {
            let f: Vec<&str> = line.trim_start_matches("#HttpOnly_").split('\t').collect();
            (f.len() == 7).then(|| Entry {
                domain: f[0].into(),
                path: f[2].into(),
                secure: f[3] == "TRUE",
                expires: f[4].parse().unwrap_or(0),
                name: f[5].into(),
                value: f[6].into(),
            })
        })
        .map(|e| ((e.domain.clone(), e.path.clone(), e.name.clone()), e))
        .collect()
}

fn write_entries(entries: &BTreeMap<Key, Entry>) {
    let mut text = String::from("# Netscape HTTP Cookie File\n");
    for e in entries.values() {
        let subdomains = if e.domain.starts_with('.') { "TRUE" } else { "FALSE" };
        let secure = if e.secure { "TRUE" } else { "FALSE" };
        text += &format!("{}\t{subdomains}\t{}\t{secure}\t{}\t{}\t{}\n", e.domain, e.path, e.expires, e.name, e.value);
    }
    let _ = std::fs::write(cookie_file(), text);
    // ログイン情報なので、ほかのユーザーから読めないようにする
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(cookie_file(), std::fs::Permissions::from_mode(0o600));
    }
}

/// 小さなブラウザの cookie を、保存してある内容と同じにする(消してから入れ直す)
fn sync_cookies_into(window: &tauri::WebviewWindow) {
    if let Ok(existing) = window.cookies() {
        for cookie in existing {
            let _ = window.delete_cookie(cookie);
        }
    }
    for e in read_entries().into_values() {
        let mut cookie = tauri::webview::Cookie::build((e.name, e.value)).domain(e.domain).path(e.path).secure(e.secure);
        if e.expires > 0 {
            if let Ok(at) = tauri::webview::cookie::time::OffsetDateTime::from_unix_timestamp(e.expires) {
                cookie = cookie.expires(at);
            }
        }
        let _ = window.set_cookie(cookie.build());
    }
}

/// 読み取った cookie で保存内容を置き換える(小さなブラウザ側が正しい状態)。
/// 期限切れは捨てる。ログインした印の cookie があれば true。
fn save(cookies: &[tauri::webview::Cookie<'static>]) -> bool {
    // 何も読めなかった時は、保存してある内容を消さない
    if cookies.is_empty() {
        return false;
    }
    let previous = read_entries();
    let mut entries = BTreeMap::new();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let mut logged_in = false;
    for c in cookies {
        let Some(domain) = c.domain() else { continue };
        // 改行やタブが入っていると cookies.txt が壊れるので飛ばす
        if [c.name(), c.value(), domain].iter().any(|s| s.contains(['\t', '\n', '\r'])) {
            continue;
        }
        let domain = if domain.starts_with('.') { domain.to_string() } else { format!(".{domain}") };
        let entry = Entry {
            domain,
            path: c.path().unwrap_or("/").to_string(),
            secure: c.secure().unwrap_or(false),
            expires: c.expires_datetime().map(|d| d.unix_timestamp()).unwrap_or(0),
            name: c.name().to_string(),
            value: c.value().to_string(),
        };
        let key = (entry.domain.clone(), entry.path.clone(), entry.name.clone());
        let mut entry = entry;
        // ブラウザが期限を返さなかった時は、前に保存した期限を引き継ぐ
        if entry.expires == 0 {
            entry.expires = previous.get(&key).map(|p| p.expires).unwrap_or(0);
        }
        if entry.expires > 0 && entry.expires < now {
            continue;
        }
        logged_in |= is_login_cookie(&entry.name);
        entries.insert(key, entry);
    }
    write_entries(&entries);
    logged_in
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_is_the_last_two_labels() {
        assert_eq!(site_of(".www.youtube.com"), "youtube.com");
        assert_eq!(site_of("nicovideo.jp"), "nicovideo.jp");
        assert_eq!(site_of(".www.example.co.jp"), "example.co.jp");
        assert!(is_login_cookie("SAPISID") && is_login_cookie("my_session") && !is_login_cookie("_ga"));
    }
}
