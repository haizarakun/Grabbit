//! ダウンロード前の解析。URL からタイトル・サムネイル・選べる形式を調べる。

use crate::download::{command, network_args};
use crate::settings::Settings;
use crate::tools;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

/// 解析にかけてよい最大時間
const TIMEOUT: Duration = Duration::from_secs(90);
/// 画面に出す形式の最大数
const MAX_FORMATS: usize = 40;

#[derive(Serialize, Default)]
pub struct MediaInfo {
    pub url: String,
    pub title: String,
    pub uploader: String,
    pub thumbnail: String,
    /// 秒
    pub duration: Option<f64>,
    /// 抽出器の名前(YouTube, Twitter など)
    pub site: String,
    /// プレイリスト/チャンネルなら true
    pub is_playlist: bool,
    pub entry_count: usize,
    pub formats: Vec<FormatInfo>,
    /// 取得に使う手段: yt-dlp / gallery-dl / direct
    pub engine: String,
    /// 解析中に起きたが回避できた問題(例: cookie が読めなかった)
    pub notice: String,
    /// 字幕がある言語(手動で付けられたもの / 自動生成)
    pub subtitle_langs: Vec<String>,
    pub auto_caption_langs: Vec<String>,
    /// チャンネル・プレイリストなど、自動保存に登録できる URL か
    pub can_subscribe: bool,
    /// 選べるサムネイル(大きい順)
    pub thumbnails: Vec<Thumbnail>,
    /// プレイリスト・チャンネルの中身(選んで保存する用)
    pub entries: Vec<PlaylistEntry>,
    /// 配信予定・プレミア公開なら、始まる時刻(UNIX秒)
    pub starts_at: Option<u64>,
    /// 大きさの目安(バイト)
    pub filesize: Option<u64>,
    /// 今まさに配信中か
    pub live_now: bool,
}

#[derive(Serialize, Clone)]
pub struct PlaylistEntry {
    pub index: usize,
    pub url: String,
    pub title: String,
    pub thumbnail: String,
    pub duration: Option<f64>,
}

#[derive(Serialize, Clone)]
pub struct Thumbnail {
    pub url: String,
    pub width: u64,
    pub height: u64,
}

#[derive(Serialize)]
pub struct FormatInfo {
    /// yt-dlp の -f にそのまま渡せる指定
    pub selector: String,
    pub label: String,
    /// video(映像のみ) / audio(音声のみ) / both(映像+音声)
    pub kind: String,
    pub height: u64,
    /// 大きさ(バイト、分からなければ 0)
    pub filesize: u64,
}

/// URL を解析する。順に yt-dlp → (cookie なし) → (ブラウザ偽装) → gallery-dl → 直接ファイル を試す。
pub async fn analyze(settings: &Settings, url: &str) -> Result<MediaInfo> {
    let has_login = !settings.cookies_browser.is_empty();
    let mut use_login = has_login && settings.cookies_when == "always";
    let mut notice = String::new();
    let mut result = run_ytdlp(settings, url, false, use_login).await;

    // ログインが必要な内容の時だけ、ブラウザのログイン情報を使ってもう一度
    if let Err(e) = &result {
        if !use_login && has_login && crate::download::is_login_required(&e.to_string()) {
            use_login = true;
            result = run_ytdlp(settings, url, false, true).await;
        }
    }
    // ログイン情報を読めなかった(Chrome 起動中など)→ ログインなしで
    if let Err(e) = &result {
        if crate::download::is_cookie_error(&e.to_string()) {
            notice = e.to_string();
            use_login = false;
            result = run_ytdlp(settings, url, false, false).await;
        }
    }
    if result.is_err() && settings.impersonate == "auto" {
        if let Ok(json) = run_ytdlp(settings, url, true, use_login).await {
            result = Ok(json);
        }
    }
    let first_error = match result {
        Ok(json) => {
            let mut info = parse(url, &json);
            info.notice = notice;
            return Ok(info);
        }
        Err(e) => e,
    };

    // 画像・ギャラリー系
    if let Ok(count) = crate::fallback::gallery_count(settings, url).await {
        return Ok(MediaInfo {
            url: url.to_string(),
            title: url.to_string(),
            site: "gallery-dl".into(),
            is_playlist: count > 1,
            entry_count: count,
            engine: "gallery-dl".into(),
            notice,
            ..MediaInfo::default()
        });
    }
    // URL がファイルそのもの
    if let Ok((name, size)) = crate::fallback::probe_direct(settings, url).await {
        return Ok(MediaInfo {
            url: url.to_string(),
            title: name,
            site: "direct".into(),
            uploader: size.map(|b| format!("{:.1}MB", b as f64 / 1_048_576.0)).unwrap_or_default(),
            engine: "direct".into(),
            notice,
            ..MediaInfo::default()
        });
    }
    Err(first_error)
}

async fn run_ytdlp(settings: &Settings, url: &str, impersonate: bool, use_cookies: bool) -> Result<Value> {
    crate::stats::connection(url, "URL を調べる");
    let mut args = network_args(settings);
    if !use_cookies {
        args = crate::download::without_browser_cookies(&args);
    }
    let mut cmd = command(&tools::ytdlp_path());
    cmd.args(["-J", "--flat-playlist", "--no-warnings", "--no-colors", "--encoding", "utf-8"]).args(crate::download::resolve_login_flag(args));
    if impersonate {
        cmd.args(["--impersonate", "chrome"]);
    }
    cmd.args(["--", url]);

    let output = tokio::time::timeout(TIMEOUT, cmd.output())
        .await
        .context("解析がタイムアウトしました")??;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr
            .lines()
            .rev()
            .find(|l| l.contains("ERROR"))
            .unwrap_or("この URL には対応していないか、取得できませんでした")
            .replace("ERROR: ", "");
        bail!(reason);
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

// ───────────── キーワード検索 ─────────────

#[derive(Serialize)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    pub uploader: String,
    pub thumbnail: String,
    pub duration: Option<f64>,
}

/// キーワードで動画を探す。サービスごとの検索を使う(分からなければ YouTube)。
pub async fn search(settings: &Settings, query: &str, service: &str, count: usize) -> Result<Vec<SearchResult>> {
    let prefix = match service {
        "niconico" => "nicosearch",
        "soundcloud" => "scsearch",
        "bilibili" => "bilisearch",
        _ => "ytsearch",
    };
    let mut cmd = command(&tools::ytdlp_path());
    cmd.args(["-J", "--flat-playlist", "--no-warnings", "--encoding", "utf-8"])
        .args(crate::download::without_browser_cookies(&network_args(settings)))
        .args(["--", &format!("{prefix}{count}:{query}")]);
    let output = tokio::time::timeout(TIMEOUT, cmd.output()).await.context("検索がタイムアウトしました")??;
    let json: Value = serde_json::from_slice(&output.stdout).context("検索できませんでした")?;
    let entries = json.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(entries
        .iter()
        .filter_map(|e| {
            let url = [text(e, "url"), text(e, "webpage_url")].into_iter().find(|u| u.starts_with("http"))?;
            Some(SearchResult {
                url,
                title: text(e, "title"),
                uploader: [text(e, "uploader"), text(e, "channel")].into_iter().find(|u| !u.is_empty()).unwrap_or_default(),
                thumbnail: best_thumbnail(e),
                duration: e.get("duration").and_then(Value::as_f64),
            })
        })
        .collect())
}

// ───────────── 言語ごとのサムネイル ─────────────

#[derive(Serialize, Clone)]
pub struct LocalizedThumbnail {
    /// どの言語で見た時のサムネイルか
    pub lang: String,
    pub url: String,
}

/// 言語ごとに違うサムネイル(YouTube の多言語サムネイルなど)を探す。
/// 言語を変えて調べ直し、他と違う画像が出たものだけを返す。同時に4言語ずつ調べる。
pub async fn localized_thumbnails(settings: &Settings, url: &str, langs: &[String]) -> Vec<LocalizedThumbnail> {
    let limit = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
    let tasks: Vec<_> = langs
        .iter()
        .map(|lang| {
            let settings = Settings { metadata_lang: lang.clone(), ..settings.clone() };
            let (url, lang, limit) = (url.to_string(), lang.clone(), limit.clone());
            tokio::spawn(async move {
                let _permit = limit.acquire().await.ok()?;
                let mut cmd = command(&tools::ytdlp_path());
                cmd.args(["--skip-download", "--no-warnings", "--no-playlist", "--encoding", "utf-8", "--print", "%(thumbnail)s"])
                    .args(crate::download::without_browser_cookies(&network_args(&settings)))
                    .args(["--", &url]);
                let output = tokio::time::timeout(TIMEOUT, cmd.output()).await.ok()?.ok()?;
                let thumbnail = String::from_utf8_lossy(&output.stdout).trim().to_string();
                thumbnail.starts_with("http").then(|| LocalizedThumbnail { lang, url: thumbnail })
            })
        })
        .collect();

    let mut found: Vec<LocalizedThumbnail> = Vec::new();
    for task in tasks {
        if let Ok(Some(thumb)) = task.await {
            // 同じ画像は最初に見つかった言語だけ残す(URL の ? 以降は同じ画像でも変わるので比べない)
            let key = |u: &str| u.split('?').next().unwrap_or(u).to_string();
            if !found.iter().any(|f| key(&f.url) == key(&thumb.url)) {
                found.push(thumb);
            }
        }
    }
    found
}

// ───────────── すぐ出す簡易プレビュー ─────────────

#[derive(Serialize, Default)]
pub struct QuickInfo {
    pub title: String,
    pub thumbnail: String,
    pub site: String,
    pub uploader: String,
}

/// ページの見出し情報(og:title / og:image)だけを直接読んで、すぐ見せる。
/// 本格的な解析は時間がかかるので、その間にこれを出しておく。
pub async fn quick_preview(url: &str) -> Option<QuickInfo> {
    // 対応しているサイトは、公式の軽い問い合わせ口(oEmbed)を先に使う。ページ全体を読むより速い
    if let Some(info) = oembed(url).await {
        return Some(info);
    }
    // 家の中の機器(ルーターなど)へ勝手に問い合わせないよう、ページの直接読み込みは外のサイトだけ
    if is_local_address(url) {
        return None;
    }
    page_meta(url).await
}

/// localhost・社内/家庭内の IP アドレスを指す URL か
pub fn is_local_address(url: &str) -> bool {
    let Ok(parsed) = url.parse::<tauri::Url>() else { return true };
    let Some(host) = parsed.host_str() else { return true };
    match host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified(),
        Ok(std::net::IpAddr::V6(ip)) => {
            let first = ip.segments()[0];
            ip.is_loopback() || ip.is_unspecified() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
        Err(_) => host == "localhost" || host.ends_with(".local") || host.ends_with(".localhost") || !host.contains('.'),
    }
}

/// サイト公式の oEmbed(タイトル・投稿者・サムネイルだけを返す軽い問い合わせ口)
async fn oembed(url: &str) -> Option<QuickInfo> {
    let host = url.split('/').nth(2)?.trim_start_matches("www.").trim_start_matches("m.").to_string();
    let (endpoint, site) = match host.as_str() {
        "youtube.com" | "youtu.be" | "music.youtube.com" => ("https://www.youtube.com/oembed", "YouTube"),
        "vimeo.com" => ("https://vimeo.com/api/oembed.json", "Vimeo"),
        "tiktok.com" => ("https://www.tiktok.com/oembed", "TikTok"),
        "soundcloud.com" => ("https://soundcloud.com/oembed", "SoundCloud"),
        "dailymotion.com" | "dai.ly" => ("https://www.dailymotion.com/services/oembed", "Dailymotion"),
        "x.com" | "twitter.com" => ("https://publish.twitter.com/oembed", "X"),
        "reddit.com" => ("https://www.reddit.com/oembed", "Reddit"),
        "flickr.com" => ("https://www.flickr.com/services/oembed", "Flickr"),
        _ => return None,
    };
    // プロキシの設定があれば通す(素早いプレビューでも、設定より先に直接つながないように)
    let client = crate::net::client(std::time::Duration::from_secs(4), "Grabbit").ok()?;
    crate::stats::connection(endpoint, "URL の簡易プレビュー");
    let body = client
        .get(endpoint)
        .query(&[("url", url), ("format", "json")])
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .bytes()
        .await
        .ok()?;
    let json: Value = serde_json::from_slice(&body).ok()?;
    let title = text(&json, "title");
    // X は本文しか返さないので、タイトルは投稿者名にする
    let title = if title.is_empty() { text(&json, "author_name") } else { title };
    if title.is_empty() {
        return None;
    }
    Some(QuickInfo {
        title,
        thumbnail: text(&json, "thumbnail_url"),
        site: site.to_string(),
        uploader: text(&json, "author_name"),
    })
}

/// ページの見出し情報(og:title / og:image)を読む
async fn page_meta(url: &str) -> Option<QuickInfo> {
    let client = crate::net::client(
        std::time::Duration::from_secs(6),
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36",
    )
    .ok()?;
    crate::stats::connection(url, "URL の簡易プレビュー");
    let response = client.get(url).send().await.ok()?;
    let is_html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("html"));
    if !is_html {
        return None;
    }
    // 見出し情報はページの先頭にあるので、大きなページでも最初の部分だけ読む
    let mut body = Vec::new();
    let mut response = response;
    while let Ok(Some(chunk)) = response.chunk().await {
        body.extend_from_slice(&chunk);
        if body.len() > 512 * 1024 {
            break;
        }
    }
    let html = String::from_utf8_lossy(&body);
    let title = meta_content(&html, "og:title").or_else(|| title_tag(&html))?;
    Some(QuickInfo {
        title: decode_entities(&title),
        thumbnail: meta_content(&html, "og:image").map(|u| decode_entities(&u)).unwrap_or_default(),
        site: meta_content(&html, "og:site_name").map(|s| decode_entities(&s)).unwrap_or_default(),
        uploader: String::new(),
    })
}

/// <meta property="名前" content="…"> の content を取り出す(属性の順番が逆でもよい)
fn meta_content(html: &str, name: &str) -> Option<String> {
    for tag in html.split("<meta").skip(1) {
        let tag = &tag[..tag.find('>').unwrap_or(tag.len())];
        let has_name = [format!("property=\"{name}\""), format!("name=\"{name}\""), format!("property='{name}'")]
            .iter()
            .any(|needle| tag.contains(needle.as_str()));
        if has_name {
            let start = tag.find("content=")? + "content=".len();
            let quote = tag[start..].chars().next()?;
            let rest = &tag[start + 1..];
            return rest.find(quote).map(|end| rest[..end].to_string()).filter(|v| !v.is_empty());
        }
    }
    None
}

fn title_tag(html: &str) -> Option<String> {
    let start = html.find("<title")?;
    let open_end = html[start..].find('>')? + start + 1;
    let close = html[open_end..].find("</title>")? + open_end;
    Some(html[open_end..close].trim().to_string()).filter(|t| !t.is_empty())
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

fn text(json: &Value, key: &str) -> String {
    json.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn parse(url: &str, json: &Value) -> MediaInfo {
    let entries = json.get("entries").and_then(Value::as_array);
    let uploader = [text(json, "uploader"), text(json, "channel"), text(json, "uploader_id")]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or_default();

    MediaInfo {
        url: url.to_string(),
        title: text(json, "title"),
        uploader,
        thumbnail: best_thumbnail(json),
        duration: json.get("duration").and_then(Value::as_f64),
        site: text(json, "extractor_key"),
        is_playlist: entries.is_some(),
        entry_count: entries.map_or(0, Vec::len),
        engine: "yt-dlp".into(),
        notice: String::new(),
        subtitle_langs: keys_of(json, "subtitles"),
        auto_caption_langs: keys_of(json, "automatic_captions"),
        can_subscribe: entries.is_some(),
        thumbnails: thumbnails_of(json),
        entries: entries.map(|list| playlist_entries(list)).unwrap_or_default(),
        live_now: text(json, "live_status") == "is_live" || json.get("is_live").and_then(Value::as_bool).unwrap_or(false),
        starts_at: (text(json, "live_status") == "is_upcoming")
            .then(|| json.get("release_timestamp").and_then(Value::as_u64))
            .flatten(),
        filesize: json.get("filesize").or_else(|| json.get("filesize_approx")).and_then(Value::as_f64).map(|v| v as u64),
        formats: json
            .get("formats")
            .and_then(Value::as_array)
            .map(|list| parse_formats(list))
            .unwrap_or_default(),
    }
}

/// 中身の一覧(多すぎると画面が重くなるので先頭 1000 件まで)
fn playlist_entries(list: &[Value]) -> Vec<PlaylistEntry> {
    list.iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let url = [text(e, "url"), text(e, "webpage_url")].into_iter().find(|u| u.starts_with("http"))?;
            Some(PlaylistEntry {
                index: i + 1,
                url,
                title: text(e, "title"),
                thumbnail: best_thumbnail(e),
                duration: e.get("duration").and_then(Value::as_f64),
            })
        })
        .take(1000)
        .collect()
}

/// 字幕の言語一覧(live_chat などの字幕でないものは除く)
fn keys_of(json: &Value, key: &str) -> Vec<String> {
    let mut langs: Vec<String> = json
        .get(key)
        .and_then(Value::as_object)
        .map(|map| map.keys().filter(|k| *k != "live_chat").cloned().collect())
        .unwrap_or_default();
    langs.sort();
    langs
}

/// サムネイルの一覧。同じ画像は1つにまとめ、大きい順に並べる。
fn thumbnails_of(json: &Value) -> Vec<Thumbnail> {
    let mut list: Vec<Thumbnail> = json
        .get("thumbnails")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|t| {
                    let url = text(t, "url");
                    (!url.is_empty()).then(|| Thumbnail {
                        url,
                        width: t.get("width").and_then(Value::as_u64).unwrap_or(0),
                        height: t.get("height").and_then(Value::as_u64).unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    // 大きい順。大きさ不明のものは後ろ
    list.sort_by(|a, b| (b.width * b.height).cmp(&(a.width * a.height)));
    let mut seen = std::collections::HashSet::new();
    list.retain(|t| seen.insert(t.url.split('?').next().unwrap_or(&t.url).to_string()));
    list.truncate(24);
    list
}

/// 待ち行列の表示用に、タイトルとサムネイルだけを軽く調べる。
pub async fn quick_info(settings: &Settings, url: &str) -> Option<(String, String, u64)> {
    let mut cmd = command(&tools::ytdlp_path());
    cmd.args(["--simulate", "--no-warnings", "--flat-playlist", "--playlist-items", "1", "--encoding", "utf-8"])
        .args(["--print", "%(title)s", "--print", "%(thumbnail)s", "--print", "%(filesize,filesize_approx|0)s"])
        .args(crate::download::without_browser_cookies(&network_args(settings)))
        .args(["--", url]);
    let output = tokio::time::timeout(TIMEOUT, cmd.output()).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines().map(str::trim);
    let title = lines.next().filter(|t| !t.is_empty() && *t != "NA")?.to_string();
    let thumbnail = lines.next().filter(|t| t.starts_with("http")).unwrap_or_default().to_string();
    let size = lines.next().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0) as u64;
    Some((title, thumbnail, size))
}

fn best_thumbnail(json: &Value) -> String {
    let direct = text(json, "thumbnail");
    if !direct.is_empty() {
        return direct;
    }
    // プレイリストは thumbnails 配列の最後(最大)を使う
    json.get("thumbnails")
        .and_then(Value::as_array)
        .and_then(|list| list.last())
        .map(|t| text(t, "url"))
        .unwrap_or_default()
}

/// 形式の一覧を「画質が高い順・見て分かるラベル」に整える。
fn parse_formats(list: &[Value]) -> Vec<FormatInfo> {
    let mut formats: Vec<FormatInfo> = list
        .iter()
        .filter_map(|f| {
            let id = text(f, "format_id");
            let has_video = !matches!(text(f, "vcodec").as_str(), "none" | "");
            let has_audio = !matches!(text(f, "acodec").as_str(), "none" | "");
            // ストーリーボード(サムネ画像の束)などは除く
            if id.is_empty() || (!has_video && !has_audio && text(f, "ext") == "mhtml") {
                return None;
            }
            let kind = match (has_video, has_audio) {
                (true, true) => "both",
                (true, false) => "video",
                (false, true) => "audio",
                // 不明な形式は単体で落とせるものとして扱う
                (false, false) => "both",
            };
            // 映像のみの形式には最高音質を足して1本にする
            let selector = if kind == "video" { format!("{id}+bestaudio/{id}") } else { id.clone() };
            Some(FormatInfo {
                label: format_label(f, kind),
                selector,
                kind: kind.to_string(),
                height: f.get("height").and_then(Value::as_u64).unwrap_or(0),
                filesize: f.get("filesize").or_else(|| f.get("filesize_approx")).and_then(Value::as_f64).unwrap_or(0.0) as u64,
            })
        })
        .collect();

    formats.sort_by(|a, b| {
        let audio_last = |f: &FormatInfo| f.kind == "audio";
        audio_last(a).cmp(&audio_last(b)).then(b.height.cmp(&a.height))
    });
    formats.truncate(MAX_FORMATS);
    formats
}

fn format_label(f: &Value, kind: &str) -> String {
    let mut parts = Vec::new();
    match kind {
        "audio" => {
            parts.push("音声".to_string());
            if let Some(abr) = f.get("abr").and_then(Value::as_f64) {
                parts.push(format!("{abr:.0}kbps"));
            }
        }
        _ => {
            let height = f.get("height").and_then(Value::as_u64);
            parts.push(height.map_or_else(|| text(f, "format_note"), |h| format!("{h}p")));
            if let Some(fps) = f.get("fps").and_then(Value::as_f64).filter(|fps| *fps > 30.0) {
                parts.push(format!("{fps:.0}fps"));
            }
        }
    }
    parts.push(text(f, "ext"));
    let codec = if kind == "audio" { text(f, "acodec") } else { text(f, "vcodec") };
    if let Some(short) = codec.split('.').next().filter(|c| !c.is_empty()) {
        parts.push(short.to_string());
    }
    let size = f
        .get("filesize")
        .or_else(|| f.get("filesize_approx"))
        .and_then(Value::as_f64);
    if let Some(bytes) = size {
        parts.push(format!("{:.0}MB", bytes / 1_048_576.0));
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_addresses_are_detected() {
        assert!(is_local_address("http://192.168.1.1/admin"));
        assert!(is_local_address("http://localhost:8080/"));
        assert!(is_local_address("http://[::1]/"));
        assert!(!is_local_address("https://www.youtube.com/watch?v=x"));
    }
    use serde_json::json;

    #[test]
    fn formats_are_sorted_and_labeled() {
        let info = parse(
            "u",
            &json!({
                "title": "t", "extractor_key": "Youtube", "uploader": "ch",
                "formats": [
                    {"format_id": "sb0", "ext": "mhtml", "vcodec": "none", "acodec": "none"},
                    {"format_id": "140", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "abr": 129.5},
                    {"format_id": "137", "ext": "mp4", "vcodec": "avc1.640028", "acodec": "none", "height": 1080, "filesize": 104857600},
                    {"format_id": "18", "ext": "mp4", "vcodec": "avc1", "acodec": "mp4a", "height": 360}
                ]
            }),
        );
        let selectors: Vec<_> = info.formats.iter().map(|f| f.selector.as_str()).collect();
        assert_eq!(selectors, ["137+bestaudio/137", "18", "140"]);
        assert_eq!(info.formats[0].label, "1080p · mp4 · avc1 · 100MB");
        assert_eq!(info.formats[2].label, "音声 · 130kbps · m4a · mp4a");
        assert_eq!(info.site, "Youtube");
    }

    #[test]
    fn meta_tags_are_read_in_any_order() {
        let html = r#"<head><title>x</title><meta content="Hello &amp; bye" property="og:title"><meta property="og:image" content="https://i/a.jpg"></head>"#;
        assert_eq!(meta_content(html, "og:title").map(|t| decode_entities(&t)).as_deref(), Some("Hello & bye"));
        assert_eq!(meta_content(html, "og:image").as_deref(), Some("https://i/a.jpg"));
        assert_eq!(title_tag("<title> Page </title>").as_deref(), Some("Page"));
    }

    #[test]
    fn playlist_entries_are_listed() {
        let info = parse("u", &json!({"title": "list", "entries": [
            {"url": "https://a/1", "title": "one"}, {"title": "no url"}, {"url": "https://a/3", "title": "three"}
        ]}));
        let titles: Vec<_> = info.entries.iter().map(|e| (e.index, e.title.as_str())).collect();
        assert_eq!(titles, [(1, "one"), (3, "three")]);
    }

    #[test]
    fn playlist_is_detected() {
        let info = parse("u", &json!({"title": "list", "entries": [{}, {}, {}]}));
        assert!(info.is_playlist);
        assert_eq!(info.entry_count, 3);
    }
}
