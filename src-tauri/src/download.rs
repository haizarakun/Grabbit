//! ダウンロードジョブの実行。yt-dlp を子プロセスで動かし、進捗を画面へ送る。

use crate::settings::{archive_path, Settings};
use crate::tools;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Notify;

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Waiting,
    Running,
    Done,
    Failed,
    Canceled,
    /// 一時停止中(再開すると途中から続ける)
    Paused,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Job {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub status: Status,
    pub percent: f32,
    pub speed: String,
    pub eta: String,
    /// 失敗時の理由、または完了時の補足
    pub message: String,
    /// 登録チャンネルの自動確認から来たジョブなら true
    pub from_subscription: bool,
    /// プレビューで選んだ形式(yt-dlp の -f)。None なら設定の画質プリセット
    pub format: Option<String>,
    /// 保存されたファイルの場所(完了後)
    pub file_path: String,
    pub thumbnail: String,
    /// 実際に使った取得方法: yt-dlp / gallery-dl / direct
    pub engine: String,
    /// 細かい段階: preparing / downloading / processing
    pub phase: String,
    /// 予約: この時刻(UNIX秒)になるまで始めない。0 ならすぐ
    pub start_at: u64,
    /// この1件だけの保存方法(画質・ファイル名・字幕など)。設定より優先する
    pub overrides: serde_json::Map<String, serde_json::Value>,
    /// 回線のエラーで自動的にやり直した回数
    #[serde(default)]
    pub auto_retries: u32,
    /// 大きさの目安(バイト、分からなければ 0)。「小さい順・大きい順」の並びに使う
    #[serde(default)]
    pub size_hint: u64,
    /// 投稿者(チャンネル名)
    #[serde(default)]
    pub uploader: String,
    /// 長さ(秒)と画質(高さ)。分からなければ 0
    #[serde(default)]
    pub duration: u64,
    #[serde(default)]
    pub height: u64,
    /// 書き込み中だったファイル(中止した時に、途中のファイルを消すため)
    #[serde(default, skip_serializing)]
    pub partials: Vec<String>,
    #[serde(skip)]
    pub cancel: Arc<Notify>,
}

impl Job {
    pub fn new(
        id: u64,
        url: String,
        from_subscription: bool,
        format: Option<String>,
        overrides: serde_json::Map<String, serde_json::Value>,
    ) -> Self {
        Self {
            id,
            title: url.clone(),
            url,
            status: Status::Waiting,
            percent: 0.0,
            speed: String::new(),
            eta: String::new(),
            message: String::new(),
            from_subscription,
            format,
            file_path: String::new(),
            thumbnail: String::new(),
            engine: String::new(),
            phase: "preparing".into(),
            start_at: 0,
            overrides,
            auto_retries: 0,
            size_hint: 0,
            uploader: String::new(),
            duration: 0,
            height: 0,
            partials: Vec::new(),
            cancel: Arc::new(Notify::new()),
        }
    }
}

/// yt-dlp 等を起動する共通の Command。Windows で黒い窓が出ないようにする。
pub fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.env("PATH", tools::child_path_env())
        .env("PYTHONIOENCODING", "utf-8")
        // 出力をためずにすぐ流させる(進捗が止まって見えるのを防ぐ)
        .env("PYTHONUNBUFFERED", "1")
        // Windows の既定(Shift_JIS 等)ではなく UTF-8 で読み書きさせる。
        // これが無いとタイトルが文字化けし、絵文字などで「Errno 22 Invalid argument」になる
        .env("PYTHONUTF8", "1")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

/// 進捗行の目印。yt-dlp の通常ログと区別するために付ける。
const PROGRESS_TAG: &str = "GRB_PROGRESS";
const TITLE_TAG: &str = "GRB_TITLE";
const FILE_TAG: &str = "GRB_FILE";
const THUMB_TAG: &str = "GRB_THUMB";
const UPLOADER_TAG: &str = "GRB_UPLOADER";
const META_TAG: &str = "GRB_META";

/// ブラウザのログイン情報(cookie)を読めずに失敗したか。Chrome 起動中などで起こる
pub fn is_cookie_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("cookie") && (lower.contains("could not") || lower.contains("failed") || lower.contains("decrypt"))
}

/// ログインしないと見られない内容か(有料・メンバー限定・非公開・年齢制限など)
pub fn is_login_required(message: &str) -> bool {
    let lower = message.to_lowercase();
    [
        "sign in", "log in", "login", "--cookies", "authentication", "private video", "members",
        "member-only", "premium", "subscriber", "purchase", "paid", "confirm your age", "age-restricted",
        "inappropriate", "registered users",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

/// ファイル名・パスが原因で書き込めなかったか(Windows の Errno 22 / 長すぎるパス など)
pub fn is_filename_error(message: &str) -> bool {
    message.contains("Errno 22") || message.contains("Invalid argument") || message.contains("WinError 123")
        || message.contains("WinError 206") || message.contains("File name too long") || message.contains("Errno 36")
}

/// 保存する前に、できるファイルの場所を yt-dlp に聞くための引数(実際には落とさない)
pub fn predict_filename_args(settings: &Settings, url: &str) -> Vec<String> {
    let mut args: Vec<String> = ["--simulate", "--no-playlist", "--no-warnings", "--encoding", "utf-8", "-O", "filename"]
        .iter().map(|s| s.to_string()).collect();
    args.extend(without_browser_cookies(&network_args(settings)));
    args.extend(["-P".into(), settings.output_dir.clone(), "-o".into(), settings.filename_template.clone()]);
    if settings.restrict_filenames {
        args.push("--restrict-filenames".into());
    }
    args.extend(["--windows-filenames".into(), "--trim-filenames".into(), "180".into(), "--".into(), url.into()]);
    resolve_login_flag(args)
}

/// 同じ名前(拡張子違いも含む)のファイルが、そのフォルダにもうあるか。あればその場所を返す
pub fn existing_same_name(predicted: &Path) -> Option<PathBuf> {
    let dir = predicted.parent()?;
    let stem = predicted.file_stem()?.to_string_lossy().to_lowercase();
    std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
        p.is_file()
            && p.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() == stem)
            && !p.extension().is_some_and(|e| matches!(e.to_string_lossy().to_lowercase().as_str(), "part" | "ytdl" | "srt" | "vtt" | "ass" | "jpg" | "webp" | "png" | "json" | "description"))
    })
}

/// 同じ名前を避けて「名前 (2).拡張子」の形で保存するよう、出力先の指定を差し替える。
/// existing は保存先にある同じ名前のファイル。
pub fn with_numbered_output(args: Vec<String>, existing: &Path) -> Vec<String> {
    let (Some(dir), Some(stem)) = (existing.parent(), existing.file_stem()) else { return args };
    let stem = stem.to_string_lossy();
    let mut n = 2;
    let taken = |n: u32| {
        std::fs::read_dir(dir).map(|entries| {
            entries.filter_map(|e| e.ok()).any(|e| e.path().file_stem().is_some_and(|s| s.to_string_lossy() == format!("{stem} ({n})")))
        }).unwrap_or(false)
    };
    while taken(n) {
        n += 1;
    }
    // テンプレートとして読まれるので、名前に含まれる % は %% にする
    let template = dir.join(format!("{} ({n}).%(ext)s", stem.replace('%', "%%"))).to_string_lossy().into_owned();
    let mut args = args;
    if let Some(i) = args.iter().position(|a| a == "-o") {
        if i + 1 < args.len() {
            args[i + 1] = template;
        }
    }
    args
}

/// 回線が原因の失敗か(時間をおけば直る見込みがある)
pub fn is_network_error(message: &str) -> bool {
    let m = message.to_lowercase();
    ["timed out", "timeout", "connection reset", "connection aborted", "connection refused", "network is unreachable",
     "temporary failure in name resolution", "getaddrinfo failed", "name or service not known", "no route to host",
     "unable to connect", "incompleteread", "remote end closed", "connection broken", "ssl: unexpected_eof",
     "http error 500", "http error 502", "http error 503", "http error 504", "winerror 10054", "winerror 10060"]
        .iter()
        .any(|p| m.contains(p))
}

/// 自動でやり直すまでの待ち時間(秒)。1回目 30 秒、2回目 2 分、3回目以降 10 分
pub fn auto_retry_delay(attempt: u32) -> u64 {
    match attempt {
        0 => 30,
        1 => 120,
        _ => 600,
    }
}

/// サムネイルが取れずに止まったか(サイトがサムネイルの場所を「/thumb.jpg」のように書いている時など)
pub fn is_thumbnail_error(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("unsupported url scheme") || m.contains("unable to download thumbnail") || (m.contains("thumbnail") && m.contains("unable"))
}

/// サムネイルの保存・埋め込みを外す(サムネイルのせいで動画まで保存できない時の再挑戦用)
pub fn without_thumbnails(args: Vec<String>) -> Vec<String> {
    let mut result = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
        } else if arg == "--convert-thumbnails" {
            skip_next = true;
        } else if !matches!(arg.as_str(), "--embed-thumbnail" | "--write-thumbnail" | "--write-all-thumbnails") {
            result.push(arg);
        }
    }
    result
}

/// 地域制限で弾かれたか
pub fn is_geo_blocked(message: &str) -> bool {
    let m = message.to_lowercase();
    ["not available in your country", "not available in your location", "geo restrict", "geo-restrict",
     "blocked it in your country", "not made this video available in your country", "available in your region"]
        .iter()
        .any(|p| m.contains(p))
}

/// プロキシを差し替える(地域制限の再挑戦用)
pub fn with_proxy(args: Vec<String>, proxy: &str) -> Vec<String> {
    let mut result = Vec::with_capacity(args.len() + 2);
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
        } else if arg == "--proxy" {
            skip_next = true;
        } else {
            result.push(arg);
        }
    }
    let at = result.iter().position(|a| a == "--").unwrap_or(result.len());
    result.splice(at..at, ["--proxy".to_string(), proxy.to_string()]);
    result
}

/// ファイル名を英数字だけの短い形にする(書き込めなかった時の再挑戦用)
pub fn with_safe_filenames(mut args: Vec<String>) -> Vec<String> {
    let at = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let extra = ["--restrict-filenames", "--trim-filenames", "80", "-o", "%(title).60B [%(id)s].%(ext)s"];
    args.splice(at..at, extra.iter().map(|s| s.to_string()));
    args
}

/// 文章から URL を取り出す。空白や改行が無く URL がくっついていても分ける
/// (例: "…?si=abchttps://youtu.be/…")。ただし "?url=https://…" のような URL の中の URL は分けない。
/// 共有用に付く追跡用の値(si=)は外す(同じ動画の重複を見分けるため)。
pub fn extract_urls(text: &str) -> Vec<String> {
    let mut spaced = String::with_capacity(text.len() + 16);
    let mut previous = ' ';
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let starts_url = rest.starts_with("https://") || rest.starts_with("http://");
        if starts_url && !previous.is_whitespace() && !"=&?/:#%(\"'".contains(previous) {
            spaced.push(' ');
        }
        spaced.push(c);
        previous = c;
        rest = &rest[c.len_utf8()..];
    }
    spaced
        .split_whitespace()
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
        .map(remove_tracking)
        .collect()
}

fn remove_tracking(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else { return url.to_string() };
    let kept: Vec<&str> = query.split('&').filter(|p| !p.starts_with("si=")).collect();
    if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    }
}

/// yt-dlp が対応していない URL か
pub fn is_unsupported(message: &str) -> bool {
    message.contains("Unsupported URL") || message.contains("No video formats found") || message.contains("no suitable")
}

/// アプリ内ログインの cookie を渡す時の印(実行直前に --cookies へ置き換える)
pub const IN_APP_LOGIN_FLAG: &str = "--grabbit-login-cookies";

/// 印を本物の引数に直す
pub fn resolve_login_flag(args: Vec<String>) -> Vec<String> {
    args.into_iter().map(|a| if a == IN_APP_LOGIN_FLAG { "--cookies".into() } else { a }).collect()
}

/// ブラウザの cookie 指定を外す(読めない時の再挑戦用)。
pub fn without_browser_cookies(args: &[String]) -> Vec<String> {
    let mut result = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
        } else if arg == "--cookies-from-browser" || arg == IN_APP_LOGIN_FLAG {
            skip_next = true;
        } else {
            result.push(arg.clone());
        }
    }
    result
}

/// 通信まわりの引数。ダウンロードと事前解析の両方で使う。
pub fn network_args(settings: &Settings) -> Vec<String> {
    // パソコンに置かれた yt-dlp の設定ファイルは読まない(中身によって保存先や動きが変わったり、
    // 知らないうちにコマンドを実行する指定が入っていたりしないように)
    let mut args: Vec<String> = vec!["--ignore-config".into()];
    let mut push = |items: &[&str]| args.extend(items.iter().map(|s| s.to_string()));
    // "grabbit" = アプリ内でログインした情報を使う
    let login_file = crate::login::cookie_file().to_string_lossy().into_owned();
    let (cookie_flag, cookie_value) = if settings.cookies_browser == "grabbit" && !crate::login::cookie_file().exists() {
        ("", &login_file) // まだ一度もログインしていない
    } else if settings.cookies_browser == "grabbit" {
        (IN_APP_LOGIN_FLAG, &login_file)
    } else {
        ("--cookies-from-browser", &settings.cookies_browser)
    };
    let optional_values = [
        (cookie_flag, cookie_value),
        ("--cookies", &settings.cookies_file),
        ("--proxy", &settings.proxy),
        ("--user-agent", &settings.user_agent),
    ];
    for (flag, value) in optional_values {
        if !flag.is_empty() && !value.is_empty() {
            push(&[flag, value]);
        }
    }
    // 地域: 指定した国からの接続として扱う(地域ごとに違うタイトル・サムネイルもこれで変わる)
    if !settings.region.is_empty() {
        push(&["--xff", &settings.region]);
    } else if settings.geo_bypass {
        push(&["--xff", "default"]);
    }
    // 動画の情報(タイトル・サムネイル等)をどの言語版で取るか
    if !settings.metadata_lang.is_empty() {
        push(&["--extractor-args", &format!("youtube:lang={}", settings.metadata_lang)]);
    }
    if settings.force_ipv4 {
        push(&["--force-ipv4"]);
    }
    if settings.socket_timeout > 0 {
        push(&["--socket-timeout", &settings.socket_timeout.to_string()]);
    }
    // "auto" は失敗時だけ偽装して再挑戦する(with_impersonate)
    if !matches!(settings.impersonate.as_str(), "" | "auto" | "off") {
        push(&["--impersonate", &settings.impersonate]);
    }
    args
}

/// ブラウザになりすまして再挑戦するための引数を差し込む(Cloudflare 等の対策)。
pub fn with_impersonate(mut args: Vec<String>) -> Vec<String> {
    let at = args.iter().position(|a| a == "--").unwrap_or(args.len());
    args.splice(at..at, ["--impersonate".to_string(), "chrome".to_string()]);
    args
}

/// 設定から yt-dlp の引数を組み立てる。
pub fn build_args(settings: &Settings, url: &str, from_subscription: bool, format: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| args.extend(items.iter().map(|s| s.to_string()));

    // --print を使うと何も表示しなくなるので、--no-quiet で記録(ログ)に経過を残す
    push(&["--newline", "--progress", "--no-simulate", "--no-quiet", "--no-colors", "--encoding", "utf-8"]);
    push(&["--progress-template", &format!(
        "download:{PROGRESS_TAG} %(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s"
    )]);
    push(&["--print", &format!("video:{TITLE_TAG} %(title)s")]);
    push(&["--print", &format!("after_move:{FILE_TAG} %(filepath)s")]);
    push(&["--print", &format!("video:{THUMB_TAG} %(thumbnail)s")]);
    push(&["--print", &format!("video:{UPLOADER_TAG} %(uploader,channel,creator|)s")]);
    push(&["--print", &format!("video:{META_TAG} %(duration|0)s|%(height|0)s")]);
    // FFmpeg をオフにしている時は場所を教えない(PC に入っている FFmpeg があればそれが使われる)
    if tools::is_enabled("ffmpeg") {
        push(&["--ffmpeg-location", &tools::bin_dir().to_string_lossy()]);
    }
    // 「保存済みの記録」は自動保存でだけ使う。
    // 手動の時に使うと、もう一度保存しようとしても何も言わずに飛ばされてしまうため
    // (手動の重複は画面側で確認する)
    if settings.use_archive && from_subscription {
        push(&["--download-archive", &archive_path().to_string_lossy()]);
    }
    push(&["-P", &settings.output_dir, "-o", &settings.filename_template]);
    // 速さ優先: 細かく分けて同時に取る(分けられない形式でも 10MB ずつ区切って取る)
    if settings.fast_mode {
        push(&["--concurrent-fragments", "16", "--http-chunk-size", "10M"]);
    }
    // プレイリストの曲に番号を振る(音楽プレーヤーで順番どおりに並ぶ)
    if settings.music_track_numbers && settings.mode == "audio" {
        push(&["--parse-metadata", "%(playlist_index|)s:%(meta_track)s"]);
    }
    if settings.playlist_order == "reverse" {
        push(&["--playlist-reverse"]);
    }
    // 保存日と元のページを、ファイルの情報(コメント欄など)に入れる
    if settings.embed_source_info && settings.embed_metadata {
        push(&["--parse-metadata", "%(webpage_url)s:%(meta_comment)s"]);
        push(&["--parse-metadata", "%(epoch>%Y-%m-%d)s:%(meta_saved_date)s"]);
    }
    if settings.on_same_name == "overwrite" {
        push(&["--force-overwrites"]);
    }
    if !settings.temp_dir.is_empty() {
        push(&["-P", &format!("temp:{}", settings.temp_dir)]);
    }
    push(&["--concurrent-fragments", &settings.concurrent_fragments.to_string()]);
    push(&["--retries", &settings.retries.to_string()]);
    push(&["--fragment-retries", &settings.retries.to_string()]);
    push(&["--no-mtime", "--ignore-errors"]);
    // Windows で使えない文字・長すぎるパスを避ける(「Errno 22 Invalid argument」の原因になる)
    push(&["--windows-filenames", "--trim-filenames", "180"]);

    // 何を保存するか(動画 / 音声 / サムネイルだけ / 字幕だけ / 情報だけ)
    match settings.mode.as_str() {
        "thumbnail" => push(&["--skip-download", "--write-thumbnail"]),
        "subtitles" => push(&["--skip-download", "--write-subs", "--write-auto-subs", "--sub-langs", &settings.subtitle_langs]),
        "info" => push(&["--skip-download", "--write-description", "--write-info-json"]),
        "audio" => {
            let quality = if settings.audio_bitrate.is_empty() { "0" } else { &settings.audio_bitrate };
            if let Some(format) = format {
                push(&["-f", format]);
            }
            push(&["-x", "--audio-format", &settings.audio_format, "--audio-quality", quality]);
        }
        _ => match format {
            // プレビューで形式を選んだ場合はそれを優先する
            Some(format) => push(&["-f", format, "--merge-output-format", &settings.container]),
            None => push_preset(settings, &mut push),
        },
    }

    if !settings.recode_video.is_empty() {
        push(&["--recode-video", &settings.recode_video]);
    }
    if settings.prefer_free_formats {
        push(&["--prefer-free-formats"]);
    }
    if !settings.download_sections.is_empty() {
        push(&["--download-sections", &settings.download_sections]);
    }
    if settings.no_part {
        push(&["--no-part"]);
    }
    if !settings.convert_thumbnail.is_empty() {
        push(&["--convert-thumbnails", &settings.convert_thumbnail]);
    }
    if !settings.subtitle_format.is_empty() {
        push(&["--convert-subs", &settings.subtitle_format]);
    }


    if settings.embed_thumbnail {
        push(&["--embed-thumbnail"]);
    }
    if settings.embed_metadata {
        push(&["--embed-metadata", "--embed-chapters"]);
    }
    if settings.burn_subtitles && matches!(settings.mode.as_str(), "video") {
        // 焼き付け用に字幕を SRT で用意する(焼き付けはダウンロード後に行う)
        push(&["--write-subs", "--write-auto-subs", "--sub-langs", &settings.subtitle_langs, "--convert-subs", "srt"]);
    }
    // 焼き付ける時は埋め込まない(埋め込むと字幕ファイルが消えて焼き付けられなくなる)
    if settings.embed_subtitles && !settings.burn_subtitles {
        push(&["--write-subs", "--embed-subs", "--sub-langs", &settings.subtitle_langs]);
    }
    if settings.sponsorblock_remove && !settings.sponsorblock_categories.is_empty() {
        push(&["--sponsorblock-remove", &settings.sponsorblock_categories]);
    }
    if settings.keep_original {
        push(&["-k"]);
    }
    if settings.write_thumbnail_file {
        push(&["--write-thumbnail"]);
    }
    if settings.write_subtitle_file {
        push(&["--write-subs", "--sub-langs", &subtitle_langs(settings)]);
    }
    if settings.auto_subtitles && (settings.write_subtitle_file || settings.embed_subtitles) {
        push(&["--write-auto-subs"]);
    }
    if settings.write_description {
        push(&["--write-description"]);
    }
    if settings.write_info_json || settings.write_comments {
        push(&["--write-info-json"]);
    }
    if settings.write_comments {
        push(&["--write-comments"]);
    }
    if settings.embed_info_json {
        push(&["--embed-info-json"]);
    }
    if settings.write_url_link {
        push(&["--write-link"]);
    }
    if settings.write_all_thumbnails {
        push(&["--write-all-thumbnails"]);
    }
    if settings.write_playlist_files {
        push(&["--write-playlist-metafiles"]);
    } else {
        push(&["--no-write-playlist-metafiles"]);
    }
    if settings.split_chapters {
        push(&["--split-chapters"]);
    }
    if settings.restrict_filenames {
        push(&["--restrict-filenames"]);
    }

    // 絞り込み
    for filter in match_filters(settings) {
        push(&["--match-filter", &filter]);
    }
    let optional_values = [
        ("--dateafter", &settings.date_after),
        ("--min-filesize", &settings.min_filesize),
        ("--max-filesize", &settings.max_filesize),
        ("--playlist-items", &settings.playlist_items),
    ];
    for (flag, value) in optional_values {
        if !flag.is_empty() && !value.is_empty() {
            push(&[flag, value]);
        }
    }
    if settings.max_downloads > 0 {
        push(&["--max-downloads", &settings.max_downloads.to_string()]);
    }
    if settings.sleep_seconds > 0 {
        push(&["--sleep-interval", &settings.sleep_seconds.to_string()]);
    }
    if settings.live_from_start {
        push(&["--live-from-start"]);
    }
    if settings.wait_for_live {
        push(&["--wait-for-video", "60"]);
    }
    if !settings.rate_limit.is_empty() {
        push(&["-r", &settings.rate_limit]);
    }
    if from_subscription {
        // 既に持っている動画に当たったら、その先(古い動画)は見ない
        push(&["--break-on-existing", "--lazy-playlist"]);
    }

    args.extend(network_args(settings));
    args.extend(split_args(&settings.extra_args));
    args.push("--".into());
    args.push(url.into());
    args
}

/// 記録(ログ)に書く前に、パスワードなどを伏せる。
/// プロキシの「ユーザー:パスワード@」と、ログイン情報・認証の値を *** にする。
pub fn redact_args(args: &[String]) -> Vec<String> {
    const SECRET_FLAGS: &[&str] = &["--password", "--video-password", "--ap-password", "--twofactor", "--add-header", "--add-headers"];
    let mut hide_next = false;
    args.iter()
        .map(|arg| {
            if hide_next {
                hide_next = false;
                return "***".to_string();
            }
            if SECRET_FLAGS.contains(&arg.as_str()) {
                hide_next = true;
                return arg.clone();
            }
            // scheme://user:pass@host → scheme://***@host
            match (arg.find("://"), arg.find('@')) {
                (Some(scheme), Some(at)) if at > scheme && !arg[scheme + 3..at].contains('/') => {
                    format!("{}***{}", &arg[..scheme + 3], &arg[at..])
                }
                _ => arg.clone(),
            }
        })
        .collect()
}

/// 設定の画質プリセットを引数にする。
fn push_preset(settings: &Settings, push: &mut impl FnMut(&[&str])) {
    match settings.preset.as_str() {
        "audio_mp3" | "audio_m4a" => {
            let format = settings.preset.trim_start_matches("audio_");
            let quality = if settings.audio_bitrate.is_empty() { "0" } else { &settings.audio_bitrate };
            push(&["-x", "--audio-format", format, "--audio-quality", quality]);
        }
        preset => {
            let limit = match preset {
                "2160" | "1440" | "1080" | "720" | "480" | "360" => format!("[height<={preset}]"),
                _ => String::new(),
            };
            // 音声: 全部のトラック / 決めた言語を優先 / 標準
            let audio_lang = settings.audio_language.trim();
            let format = if settings.all_audio_tracks {
                format!("bv*{limit}+mergeall[vcodec=none]/b{limit}/b")
            } else if !audio_lang.is_empty() && audio_lang.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                format!("bv*{limit}+ba[language^={audio_lang}]/bv*{limit}+ba/b{limit}/b")
            } else {
                format!("bv*{limit}+ba/b{limit}/b")
            };
            if settings.all_audio_tracks {
                push(&["--audio-multistreams"]);
            }
            if settings.vr_mode {
                // VR・360°動画: 8K などの高解像度は VP9/AV1 だけのことが多いので、形式を絞らず MKV にまとめる
                // (MKV は 360°の向き情報をそのまま残せる)
                push(&["-f", &format, "--merge-output-format", "mkv", "-S", "res,fps"]);
            } else {
                // 音声を全部入れる時は、何本でも入る MKV にする
                let container = if settings.all_audio_tracks { "mkv" } else { settings.container.as_str() };
                push(&["-f", &format, "--merge-output-format", container]);
                if let Some(sort) = codec_sort(&settings.video_codec, &settings.container) {
                    push(&["-S", &sort]);
                }
            }
        }
    }
}

/// 字幕として取る言語。ライブのチャットも保存する指定なら live_chat を足す。
fn subtitle_langs(settings: &Settings) -> String {
    if settings.write_live_chat && !settings.subtitle_langs.contains("live_chat") {
        format!("{},live_chat", settings.subtitle_langs)
    } else {
        settings.subtitle_langs.clone()
    }
}

/// 映像コーデックの優先順位(-S)。mp4 では再生できる組み合わせを優先する。
fn codec_sort(codec: &str, container: &str) -> Option<String> {
    let audio = if container == "mp4" { ",acodec:m4a" } else { "" };
    match codec {
        "h264" | "vp9" | "av1" => Some(format!("vcodec:{codec}{audio}")),
        _ if container == "mp4" => Some("vcodec:h264,acodec:m4a".into()),
        _ => None,
    }
}

/// チェックボックスの絞り込みと、手入力の条件をまとめる。
fn match_filters(settings: &Settings) -> Vec<String> {
    let mut conditions = Vec::new();
    if settings.skip_shorts {
        conditions.push("original_url!*=/shorts/ & url!*=/shorts/".to_string());
    }
    if settings.skip_live {
        conditions.push("!is_live & live_status!=is_upcoming".to_string());
    }
    if !settings.match_filter.is_empty() {
        conditions.push(settings.match_filter.clone());
    }
    // 複数の --match-filter は「どれか一致」になるので、& で1本にまとめる
    if conditions.is_empty() {
        Vec::new()
    } else {
        vec![conditions.join(" & ")]
    }
}

/// 追加引数を空白で分割する。"…" で囲めば空白を含められる。
fn split_args(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    result.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

/// 実行中に届く出来事。呼び出し側がジョブ一覧へ反映する。
pub enum Event {
    Title(String),
    /// 保存が終わったファイルの場所
    File(String),
    Thumbnail(String),
    /// 投稿者(チャンネル名)
    Uploader(String),
    /// 長さ(秒)と画質(高さ)
    Meta { duration: u64, height: u64 },
    /// 書き込み中のファイル(中止した時に片付ける)
    Destination(String),
    Progress { percent: f32, speed: String, eta: String },
    /// 今の段階: downloading(ダウンロード中) / processing(結合・変換・埋め込み中)
    Phase(&'static str),
}

/// yt-dlp の後処理(結合・変換・埋め込み)を表す行の頭
const PROCESSING_PREFIXES: [&str; 13] = [
    "[Merger]", "[ExtractAudio]", "[VideoConvertor]", "[VideoRemuxer]", "[EmbedThumbnail]",
    "[EmbedSubtitle]", "[Metadata]", "[FixupM3u8]", "[FixupM4a]", "[SponsorBlock]",
    "[ModifyChapters]", "[SplitChapters]", "[ThumbnailsConvertor]",
];

pub enum Outcome {
    Done,
    Failed(String),
    Canceled,
}

/// 1行読む。文字コードが崩れた行があっても止まらずに読み続ける(読めない文字は置き換える)。
/// 出力が終わったら None。
pub async fn read_line_lossy<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Option<String> {
    let mut buf = Vec::new();
    match reader.read_until(b'\n', &mut buf).await {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(String::from_utf8_lossy(&buf).trim_end_matches(['\r', '\n']).to_string()),
    }
}

/// 1件のダウンロードを最後まで実行する。
/// `--print` を使うと yt-dlp は進捗を標準エラー側へ出すので、両方の出力を読む。
/// 出力はすべて `log` に書き残す(失敗した時に画面から開ける)。
pub async fn run(args: Vec<String>, cancel: Arc<Notify>, log: &Path, on_event: &impl Fn(Event)) -> Outcome {
    let mut cmd = command(&tools::ytdlp_path());
    cmd.args(resolve_login_flag(args.clone())).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return Outcome::Failed(format!("起動できません: {e}")),
    };

    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<String>();
    let forward = |stream: Box<dyn tokio::io::AsyncRead + Unpin + Send>| {
        let sender = sender.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stream);
            while let Some(line) = read_line_lossy(&mut reader).await {
                let _ = sender.send(line);
            }
        });
    };
    forward(Box::new(child.stdout.take().expect("stdout is piped")));
    forward(Box::new(child.stderr.take().expect("stderr is piped")));
    drop(sender); // 両方の出力が閉じたら受信側も終わる

    let mut log_file = tokio::fs::File::create(log).await.ok();
    if let Some(file) = log_file.as_mut() {
        let _ = file.write_all(format!("> yt-dlp {}\n", redact_args(&args).join(" ")).as_bytes()).await;
    }
    let mut last_error = String::new();
    let read_output = async {
        while let Some(line) = receiver.recv().await {
            if let Some(file) = log_file.as_mut() {
                let _ = file.write_all(format!("{line}\n").as_bytes()).await;
            }
            if let Some(message) = line.strip_prefix("ERROR: ") {
                last_error = message.to_string();
            }
            if let Some(event) = parse_line(&line) {
                on_event(event);
            }
        }
    };

    tokio::select! {
        _ = read_output => {}
        _ = cancel.notified() => {
            let _ = child.kill().await;
            return Outcome::Canceled;
        }
    }

    match child.wait().await {
        // 101 = --break-on-existing / --max-downloads で途中終了(正常)
        Ok(s) if s.success() || s.code() == Some(101) => Outcome::Done,
        Ok(_) if last_error.is_empty() => Outcome::Failed("不明なエラー".into()),
        Ok(_) => Outcome::Failed(last_error),
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

fn parse_line(line: &str) -> Option<Event> {
    if let Some(title) = line.strip_prefix(TITLE_TAG) {
        return Some(Event::Title(title.trim().to_string()));
    }
    if let Some(url) = line.strip_prefix(THUMB_TAG) {
        let url = url.trim();
        return (url != "NA").then(|| Event::Thumbnail(url.to_string()));
    }
    if let Some(meta) = line.strip_prefix(META_TAG) {
        let (duration, height) = meta.trim().split_once('|')?;
        return Some(Event::Meta { duration: duration.parse::<f64>().unwrap_or(0.0) as u64, height: height.parse().unwrap_or(0) });
    }
    if let Some(name) = line.strip_prefix(UPLOADER_TAG) {
        let name = name.trim();
        return (!name.is_empty() && name != "NA").then(|| Event::Uploader(name.to_string()));
    }
    if let Some(path) = line.strip_prefix(FILE_TAG) {
        return Some(Event::File(path.trim().to_string()));
    }
    if let Some(path) = line.strip_prefix("[download] Destination: ") {
        return Some(Event::Destination(path.trim().to_string()));
    }
    if PROCESSING_PREFIXES.iter().any(|p| line.starts_with(p)) {
        return Some(Event::Phase("processing"));
    }
    let rest = line.strip_prefix(PROGRESS_TAG)?;
    let mut parts = rest.trim().split('|');
    let percent = parts.next()?.trim().trim_end_matches('%').parse().unwrap_or(0.0);
    let speed = parts.next().unwrap_or("").trim().to_string();
    let eta = parts.next().unwrap_or("").trim().to_string();
    Some(Event::Progress { percent, speed, eta })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|w| w[0] == flag && w[1] == value)
    }

    #[test]
    fn url_is_last_after_separator() {
        let args = build_args(&Settings::default(), "https://example.com/v", false, None);
        let n = args.len();
        assert_eq!(args[n - 2], "--");
        assert_eq!(args[n - 1], "https://example.com/v");
    }

    #[test]
    fn video_preset_limits_height() {
        let settings = Settings { preset: "1080".into(), ..Settings::default() };
        let args = build_args(&settings, "u", false, None);
        assert!(has_pair(&args, "-f", "bv*[height<=1080]+ba/b[height<=1080]/b"));
        assert!(has_pair(&args, "--merge-output-format", "mp4"));
    }

    #[test]
    fn audio_preset_extracts_mp3() {
        let settings = Settings { preset: "audio_mp3".into(), ..Settings::default() };
        let args = build_args(&settings, "u", false, None);
        assert!(args.contains(&"-x".to_string()));
        assert!(has_pair(&args, "--audio-format", "mp3"));
        assert!(!args.contains(&"--merge-output-format".to_string()));
    }

    #[test]
    fn archive_is_only_for_subscriptions() {
        let manual = build_args(&Settings::default(), "u", false, None);
        assert!(!manual.contains(&"--download-archive".to_string()));
        let auto = build_args(&Settings::default(), "u", true, None);
        assert!(auto.contains(&"--download-archive".to_string()));
    }

    #[test]
    fn subscription_stops_at_existing() {
        let args = build_args(&Settings::default(), "u", true, None);
        assert!(args.contains(&"--break-on-existing".to_string()));
        let args = build_args(&Settings::default(), "u", false, None);
        assert!(!args.contains(&"--break-on-existing".to_string()));
    }

    #[test]
    fn optional_flags_follow_settings() {
        let settings = Settings {
            cookies_browser: "chrome".into(),
            rate_limit: "5M".into(),
            sponsorblock_remove: true,
            ..Settings::default()
        };
        let args = build_args(&settings, "u", false, None);
        assert!(has_pair(&args, "--cookies-from-browser", "chrome"));
        assert!(has_pair(&args, "-r", "5M"));
        assert!(has_pair(&args, "--sponsorblock-remove", "sponsor,selfpromo,interaction"));
    }

    #[test]
    fn filters_are_combined_into_one() {
        let settings = Settings { skip_shorts: true, skip_live: true, ..Settings::default() };
        let args = build_args(&settings, "u", false, None);
        let count = args.iter().filter(|a| *a == "--match-filter").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn archive_can_be_disabled() {
        let settings = Settings { use_archive: false, ..Settings::default() };
        assert!(!build_args(&settings, "u", true, None).contains(&"--download-archive".to_string()));
    }

    #[test]
    fn audio_bitrate_is_used() {
        let settings = Settings { preset: "audio_m4a".into(), audio_bitrate: "192K".into(), ..Settings::default() };
        let args = build_args(&settings, "u", false, None);
        assert!(has_pair(&args, "--audio-quality", "192K"));
        assert!(has_pair(&args, "--audio-format", "m4a"));
    }

    #[test]
    fn chosen_format_overrides_preset() {
        let settings = Settings { preset: "audio_mp3".into(), ..Settings::default() };
        let args = build_args(&settings, "u", false, Some("137+bestaudio/137"));
        assert!(has_pair(&args, "-f", "137+bestaudio/137"));
        assert!(!args.contains(&"-x".to_string()));
    }

    #[test]
    fn impersonate_is_inserted_before_url() {
        let args = with_impersonate(build_args(&Settings::default(), "u", false, None));
        let n = args.len();
        assert_eq!(&args[n - 4..], ["--impersonate", "chrome", "--", "u"]);
    }

    #[test]
    fn joined_urls_are_split() {
        let urls = extract_urls("https://youtu.be/mlowXZ1rcoU?si=45Rn9xS03OZQH9g8https://youtu.be/KwH53_e2fhM?si=IR8qiAbac5oMboS6");
        assert_eq!(urls, ["https://youtu.be/mlowXZ1rcoU", "https://youtu.be/KwH53_e2fhM"]);
        // URL の中の URL は分けない
        assert_eq!(extract_urls("https://a.com/r?u=https://b.com/x"), ["https://a.com/r?u=https://b.com/x"]);
        assert_eq!(extract_urls("見て https://www.youtube.com/watch?v=1&si=x&t=5 です"), ["https://www.youtube.com/watch?v=1&t=5"]);
    }

    #[test]
    fn filename_errors_are_recognized() {
        assert!(is_filename_error("[Errno 22] Invalid argument: 'C:\\x'"));
        assert!(!is_filename_error("HTTP Error 403"));
        let args = with_safe_filenames(build_args(&Settings::default(), "u", false, None));
        assert!(args.contains(&"--restrict-filenames".to_string()));
        assert_eq!(args.last().unwrap(), "u");
    }

    #[test]
    fn region_and_language_are_passed() {
        let settings = Settings { region: "JP".into(), metadata_lang: "ja".into(), ..Settings::default() };
        let args = build_args(&settings, "u", false, None);
        assert!(has_pair(&args, "--xff", "JP"));
        assert!(has_pair(&args, "--extractor-args", "youtube:lang=ja"));
    }

    #[test]
    fn vr_mode_uses_mkv_without_codec_limit() {
        let settings = Settings { vr_mode: true, ..Settings::default() };
        let args = build_args(&settings, "u", false, None);
        assert!(has_pair(&args, "--merge-output-format", "mkv"));
        assert!(has_pair(&args, "-S", "res,fps"));
    }

    #[test]
    fn processing_lines_change_phase() {
        assert!(matches!(parse_line("[Merger] Merging formats into \"a.mp4\""), Some(Event::Phase("processing"))));
    }

    #[test]
    fn live_chat_is_added_to_subtitles() {
        let settings = Settings { write_subtitle_file: true, write_live_chat: true, ..Settings::default() };
        assert!(has_pair(&build_args(&settings, "u", false, None), "--sub-langs", "ja,en,live_chat"));
    }

    #[test]
    fn thumbnail_mode_skips_video() {
        let settings = Settings { mode: "thumbnail".into(), ..Settings::default() };
        let args = build_args(&settings, "u", false, None);
        assert!(args.contains(&"--skip-download".to_string()));
        assert!(args.contains(&"--write-thumbnail".to_string()));
        assert!(!args.contains(&"-f".to_string()));
    }

    #[test]
    fn audio_mode_uses_audio_format() {
        let settings = Settings { mode: "audio".into(), audio_format: "flac".into(), ..Settings::default() };
        assert!(has_pair(&build_args(&settings, "u", false, None), "--audio-format", "flac"));
    }

    #[test]
    fn login_errors_are_recognized() {
        assert!(is_login_required("Sign in to confirm your age"));
        assert!(is_login_required("This video is available to this channel's members"));
        assert!(is_login_required("Use --cookies-from-browser or --cookies for the authentication"));
        assert!(!is_login_required("Unsupported URL: https://example.com"));
    }

    #[test]
    fn browser_cookies_can_be_removed() {
        let settings = Settings { cookies_browser: "chrome".into(), ..Settings::default() };
        let args = without_browser_cookies(&build_args(&settings, "u", false, None));
        assert!(!args.iter().any(|a| a == "--cookies-from-browser" || a == "chrome"));
        assert_eq!(args.last().unwrap(), "u");
    }

    #[test]
    fn cookie_errors_are_recognized() {
        assert!(is_cookie_error("Could not copy Chrome cookie database. See https://github.com/yt-dlp/yt-dlp/issues/7271"));
        assert!(is_cookie_error("Failed to decrypt with DPAPI cookie"));
        assert!(!is_cookie_error("Unsupported URL: https://example.com"));
    }

    #[test]
    fn numbered_output_avoids_existing_names() {
        let dir = std::env::temp_dir().join("grabbit-numbered");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("a 100%.mp4"), b"x").unwrap();
        std::fs::write(dir.join("a 100% (2).mp4"), b"x").unwrap();
        let args: Vec<String> = ["-o", "%(title)s.%(ext)s", "--", "u"].iter().map(|s| s.to_string()).collect();
        let out = with_numbered_output(args, &dir.join("a 100%.mp4"));
        assert_eq!(out[1], dir.join("a 100%% (3).%(ext)s").to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn network_errors_are_detected() {
        assert!(is_network_error("ERROR: Unable to download webpage: <urlopen error [Errno 11001] getaddrinfo failed>"));
        assert!(is_network_error("ERROR: HTTP Error 503: Service Unavailable"));
        assert!(!is_network_error("ERROR: Private video"));
    }

    #[test]
    fn proxy_is_swapped_for_geo_blocks() {
        assert!(is_geo_blocked("ERROR: The uploader has not made this video available in your country"));
        let args: Vec<String> = ["--proxy", "a", "-f", "b", "--", "u"].iter().map(|s| s.to_string()).collect();
        assert_eq!(with_proxy(args, "socks5://p"), ["-f", "b", "--proxy", "socks5://p", "--", "u"]);
    }

    #[test]
    fn secrets_are_hidden_in_logs() {
        let args: Vec<String> = ["--proxy", "http://me:pw@1.2.3.4:8080", "--password", "abc", "--", "https://youtu.be/x"]
            .iter().map(|s| s.to_string()).collect();
        assert_eq!(redact_args(&args), ["--proxy", "http://***@1.2.3.4:8080", "--password", "***", "--", "https://youtu.be/x"]);
    }

    #[test]
    fn extra_args_respect_quotes() {
        assert_eq!(
            split_args(r#"--sub-langs "ja en"  --write-description"#),
            vec!["--sub-langs", "ja en", "--write-description"]
        );
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn progress_and_title_lines_are_parsed() {
        match parse_line("GRB_PROGRESS  42.5%| 3.10MiB/s|00:12") {
            Some(Event::Progress { percent, speed, eta }) => {
                assert_eq!(percent, 42.5);
                assert_eq!(speed, "3.10MiB/s");
                assert_eq!(eta, "00:12");
            }
            _ => panic!("progress not parsed"),
        }
        match parse_line("GRB_TITLE 動画タイトル") {
            Some(Event::Title(t)) => assert_eq!(t, "動画タイトル"),
            _ => panic!("title not parsed"),
        }
        assert!(parse_line("[youtube] Extracting URL").is_none());
    }
}
