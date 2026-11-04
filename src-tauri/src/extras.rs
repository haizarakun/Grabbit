//! 追加機能: 字幕の焼き付け、空き容量、省電力、ライブ配信の確認、「送る」メニュー、起動時の引数。

use crate::download::command;
use crate::settings::Settings;
use crate::tools;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

// ───────────── 字幕の焼き付け ─────────────

/// 動画の隣にある字幕ファイル(.srt / .vtt / .ass)を探す。`langs` の順に優先する。
fn find_subtitle(video: &Path, langs: &str) -> Option<PathBuf> {
    let dir = video.parent()?;
    let stem = video.file_stem()?.to_string_lossy().into_owned();
    let candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            name.starts_with(&stem) && matches!(ext.as_str(), "srt" | "vtt" | "ass") && !name.contains("live_chat")
        })
        .collect();
    for lang in langs.split(',').map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(found) = candidates.iter().find(|p| p.to_string_lossy().contains(&format!(".{lang}."))) {
            return Some(found.clone());
        }
    }
    candidates.into_iter().next()
}

/// 字幕を映像に直接描き込む(焼き付け)。元の動画は焼き付け済みの物に置き換える。
/// 焼き付ける字幕の見た目
pub struct BurnStyle {
    pub font_size: u32,
    pub color: String,
    pub top: bool,
}

pub async fn burn_subtitles(video: &Path, langs: &str, style: &BurnStyle) -> Result<()> {
    let subtitle = find_subtitle(video, langs).context("焼き付ける字幕が見つかりません(この動画に字幕が無い可能性があります)")?;
    let dir = video.parent().context("保存先が分かりません")?;
    // ffmpeg の字幕フィルタはパスの記号に弱いので、決まった名前に写してから使う
    let work_sub = dir.join("grabbit_burn_subtitle").with_extension(subtitle.extension().unwrap_or_default());
    std::fs::copy(&subtitle, &work_sub)?;
    let output = video.with_extension(format!("burned.{}", video.extension().map(|e| e.to_string_lossy()).unwrap_or("mp4".into())));
    // 文字の大きさ・色・位置(ASS の書き方: 色は &H00BBGGRR、位置 8 = 上・中央 / 2 = 下・中央)
    let mut force = Vec::new();
    if style.font_size > 0 {
        force.push(format!("Fontsize={}", style.font_size.clamp(8, 96)));
    }
    match style.color.as_str() {
        "yellow" => force.push("PrimaryColour=&H0000FFFF".into()),
        "cyan" => force.push("PrimaryColour=&H00FFFF00".into()),
        _ => {}
    }
    if style.top {
        force.push("Alignment=8".into());
    }
    let mut filter = format!("subtitles={}", work_sub.file_name().unwrap().to_string_lossy());
    if !force.is_empty() {
        filter += &format!(":force_style='{}'", force.join(","));
    }

    let ffmpeg = tools::bin_dir().join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
    let mut cmd = command(&ffmpeg);
    cmd.current_dir(dir)
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(video)
        .args(["-vf", &filter, "-c:a", "copy"])
        .arg(&output);
    let result = cmd.output().await;
    let _ = std::fs::remove_file(&work_sub);
    let result = result?;
    if !result.status.success() {
        let _ = std::fs::remove_file(&output);
        bail!("字幕の焼き付けに失敗しました: {}", String::from_utf8_lossy(&result.stderr).trim());
    }
    std::fs::rename(&output, video)?;
    Ok(())
}

/// 焼き付けに使った字幕ファイルを片付ける(別ファイルで残す指定が無い時)
pub fn remove_subtitle_files(video: &Path) {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem()) else { return };
    let stem = stem.to_string_lossy().into_owned();
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if name.starts_with(&stem) && matches!(ext.as_str(), "srt" | "vtt" | "ass") {
            let _ = std::fs::remove_file(path);
        }
    }
}

// ───────────── 保存後の仕上げ ─────────────

fn ffmpeg_path() -> PathBuf {
    tools::bin_dir().join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" })
}

/// ffmpeg で作り直したファイルを、元のファイルと入れ替える
async fn run_ffmpeg_replace(source: &Path, build: impl FnOnce(&mut tokio::process::Command, &Path)) -> Result<()> {
    let ext = source.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
    let output = source.with_extension(format!("grabbit-tmp.{ext}"));
    let mut cmd = command(&ffmpeg_path());
    cmd.args(["-y", "-loglevel", "error", "-i"]).arg(source);
    build(&mut cmd, &output);
    let result = cmd.output().await?;
    if !result.status.success() {
        let _ = std::fs::remove_file(&output);
        bail!("{}", String::from_utf8_lossy(&result.stderr).trim());
    }
    std::fs::rename(&output, source)?;
    Ok(())
}

/// 縦横の比を合わせる(足りない方に黒帯を付ける)。aspect は "16:9" か "9:16"。
pub async fn pad_aspect(video: &Path, aspect: &str) -> Result<()> {
    let (w, h) = match aspect {
        "16:9" => (16, 9),
        "9:16" => (9, 16),
        _ => return Ok(()),
    };
    // 横長にするなら高さに合わせて幅を広げる、縦長にするなら幅に合わせて高さを広げる(偶数にそろえる)
    let filter = format!(
        "pad='ceil(max(iw,ih*{w}/{h})/2)*2':'ceil(max(ih,iw*{h}/{w})/2)*2':(ow-iw)/2:(oh-ih)/2:black,setsar=1"
    );
    run_ffmpeg_replace(video, |cmd, out| {
        cmd.args(["-map", "0:v:0", "-map", "0:a?", "-vf", &filter, "-c:v", "libx264", "-crf", "20", "-preset", "veryfast", "-c:a", "copy"]).arg(out);
    })
    .await
}

/// 自動生成字幕の「同じ文が何度も出る」「細切れ」を整える(.srt / .vtt)。
pub fn clean_subtitles(video: &Path, fixes: &str) {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem()) else { return };
    let stem = stem.to_string_lossy().into_owned();
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for path in entries.filter_map(|e| e.ok()).map(|e| e.path()) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !name.starts_with(&stem) || !matches!(ext.as_str(), "srt" | "vtt") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            let cleaned = apply_fixes(&clean_subtitle_text(&text, ext == "vtt"), fixes);
            let _ = std::fs::write(&path, cleaned);
        }
    }
}

/// 「間違い→正しい」の一覧で字幕の文を直す(時間の行は変えない)
pub fn apply_fixes(text: &str, fixes: &str) -> String {
    let pairs: Vec<(&str, &str)> = fixes
        .lines()
        .filter_map(|line| line.split_once('→').or_else(|| line.split_once("->")))
        .map(|(a, b)| (a.trim(), b.trim()))
        .filter(|(a, _)| !a.is_empty())
        .collect();
    if pairs.is_empty() {
        return text.to_string();
    }
    text.lines()
        .map(|line| {
            if line.contains("-->") {
                return line.to_string();
            }
            pairs.iter().fold(line.to_string(), |acc, (wrong, right)| acc.replace(wrong, right))
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// パソコンが操作されていない秒数(Windows)。分からない環境では「ずっと操作されていない」とする
pub fn idle_seconds() -> u64 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::GetTickCount;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
        let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        // SAFETY: info は正しい大きさを書いた LASTINPUTINFO
        if unsafe { GetLastInputInfo(&mut info) } != 0 {
            let now = unsafe { GetTickCount() };
            return u64::from(now.wrapping_sub(info.dwTime)) / 1000;
        }
        u64::MAX
    }
    #[cfg(not(windows))]
    {
        u64::MAX
    }
}

/// 共有する文章から、個人の情報になりそうなものを消す
/// (ユーザー名入りのフォルダ・メールアドレス・IP アドレス・ログイン情報らしき長い値・URL の検索部分)
pub fn scrub_private(text: &str) -> String {
    let mut out = text.to_string();
    if let Some(home) = dirs::home_dir() {
        let home = home.to_string_lossy().into_owned();
        if home.len() > 3 {
            out = out.replace(&home, "~").replace(&home.replace('\\', "\\\\"), "~");
        }
    }
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    let words: Vec<String> = out
        .split_inclusive(|c: char| c.is_whitespace() || "\"'<>()[]{},".contains(c))
        .map(|piece| {
            let (word, tail) = piece.split_at(piece.trim_end_matches(|c: char| c.is_whitespace() || "\"'<>()[]{},".contains(c)).len());
            let replaced = if word.contains('@') && word.contains('.') && !word.starts_with("http") {
                "***@***".to_string()
            } else if word.split('.').count() == 4 && word.split('.').all(|p| p.parse::<u8>().is_ok()) {
                "*.*.*.*".to_string()
            } else if word.len() >= 32 && word.chars().all(|c| c.is_ascii_alphanumeric() || "-_=%".contains(c)) {
                "***".to_string()
            } else if word.starts_with("http") && word.contains('?') {
                // 動画の ID(v= など)は残し、ほかの値は伏せる
                let (base, query) = word.split_once('?').unwrap();
                let kept: Vec<String> = query.split('&').map(|kv| match kv.split_once('=') {
                    Some((k, v)) if ["v", "list", "t"].contains(&k) => format!("{k}={v}"),
                    Some((k, _)) => format!("{k}=***"),
                    None => kv.to_string(),
                }).collect();
                format!("{base}?{}", kept.join("&"))
            } else {
                word.to_string()
            };
            replaced + tail
        })
        .collect();
    let mut out = words.concat();
    if user.len() >= 3 {
        out = out.replace(&user, "<user>");
    }
    out
}

/// 字幕の中身を整える。時間の行はそのまま、文の行は「前と同じ行」を消し、タグ(<c> など)を外す
pub fn clean_subtitle_text(text: &str, vtt: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_lines: Vec<String> = Vec::new();
    let mut block: Vec<String> = Vec::new();
    let strip_tags = |line: &str| {
        let mut result = String::new();
        let mut in_tag = false;
        for c in line.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => result.push(c),
                _ => {}
            }
        }
        result.trim().to_string()
    };
    let flush = |block: &mut Vec<String>, previous: &mut Vec<String>, out: &mut String| {
        if block.is_empty() {
            return;
        }
        let (head, body): (Vec<String>, Vec<String>) = block.drain(..).partition(|l| l.contains("-->") || l.trim().chars().all(|c| c.is_ascii_digit()) || l.starts_with("WEBVTT") || l.starts_with("Kind:") || l.starts_with("Language:"));
        let body: Vec<String> = body.iter().map(|l| strip_tags(l)).filter(|l| !l.is_empty() && !previous.contains(l)).collect();
        if body.is_empty() && head.iter().any(|l| l.contains("-->")) {
            return; // 前と同じ文だけのかたまりは捨てる
        }
        for l in &head {
            // vtt の位置指定(align:start position:0% など)は外す
            let l = if l.contains("-->") { l.split_whitespace().take(3).collect::<Vec<_>>().join(" ") } else { l.clone() };
            out.push_str(&l);
            out.push('\n');
        }
        for l in &body {
            out.push_str(l);
            out.push('\n');
        }
        out.push('\n');
        *previous = body;
    };
    for line in text.lines() {
        if line.trim().is_empty() {
            flush(&mut block, &mut previous_lines, &mut out);
        } else {
            block.push(line.to_string());
        }
    }
    flush(&mut block, &mut previous_lines, &mut out);
    let _ = vtt;
    out
}

/// 音楽ファイルに曲名・アーティスト・アルバム・歌詞を書き込む(分かったものだけ)
pub async fn write_music_tags(file: &Path, tags: &[(&str, String)]) -> Result<()> {
    let tags: Vec<(&str, String)> = tags.iter().filter(|(_, v)| !v.trim().is_empty()).cloned().collect();
    if tags.is_empty() {
        return Ok(());
    }
    run_ffmpeg_replace(file, |cmd, out| {
        cmd.args(["-map", "0", "-c", "copy", "-id3v2_version", "3"]);
        for (key, value) in &tags {
            cmd.arg("-metadata").arg(format!("{key}={value}"));
        }
        cmd.arg(out);
    })
    .await
}

/// 「アーティスト - 曲名」の形のタイトルを分ける。分けられなければ (空, タイトル)
pub fn split_artist_title(title: &str) -> (String, String) {
    // (Official Video) や 【MV】 などの飾りを外す
    let mut clean = title.to_string();
    for (open, close) in [('(', ')'), ('[', ']'), ('【', '】'), ('「', '」')] {
        while let (Some(a), Some(b)) = (clean.find(open), clean.find(close)) {
            if b <= a {
                break;
            }
            let inner = clean[a..b].to_lowercase();
            if ["official", "mv", "music video", "lyric", "audio", "video", "公式"].iter().any(|w| inner.contains(w)) {
                clean.replace_range(a..b + close.len_utf8(), "");
            } else {
                break;
            }
        }
    }
    for sep in [" - ", " – ", " — ", " / ", "／"] {
        if let Some((artist, song)) = clean.split_once(sep) {
            return (artist.trim().to_string(), song.trim().to_string());
        }
    }
    (String::new(), clean.trim().to_string())
}

fn web_client() -> Option<reqwest::Client> {
    crate::net::client(std::time::Duration::from_secs(15), "Grabbit/1.0 ( https://github.com/haizarakun/Grabbit )").ok()
}

/// MusicBrainz で曲を探し、(アーティスト, 曲名, アルバム, 年) を返す
pub async fn lookup_music(artist: &str, song: &str) -> Option<(String, String, String, String)> {
    let query = if artist.is_empty() { format!("recording:\"{song}\"") } else { format!("recording:\"{song}\" AND artist:\"{artist}\"") };
    let url = reqwest::Url::parse_with_params("https://musicbrainz.org/ws/2/recording", &[("query", query.as_str()), ("fmt", "json"), ("limit", "1")]).ok()?;
    crate::stats::connection(url.as_str(), "曲の情報(MusicBrainz)");
    let body = web_client()?.get(url).send().await.ok()?.bytes().await.ok()?;
    let json: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let rec = json["recordings"].get(0)?;
    if rec["score"].as_u64().unwrap_or(0) < 90 {
        return None; // 自信がない時は書き込まない
    }
    let artist = rec["artist-credit"].get(0).and_then(|a| a["name"].as_str()).unwrap_or_default().to_string();
    let release = rec["releases"].get(0);
    let album = release.and_then(|r| r["title"].as_str()).unwrap_or_default().to_string();
    let year: String = release.and_then(|r| r["date"].as_str()).unwrap_or_default().chars().take(4).collect();
    Some((artist, rec["title"].as_str().unwrap_or_default().to_string(), album, year))
}

/// LRCLIB で歌詞を探す(時間付きがあればそれを、無ければ普通の歌詞)
pub async fn lookup_lyrics(artist: &str, song: &str, duration: Option<f64>) -> Option<String> {
    let mut params = vec![("artist_name", artist.to_string()), ("track_name", song.to_string())];
    if let Some(d) = duration {
        params.push(("duration", format!("{}", d.round() as u64)));
    }
    let url = reqwest::Url::parse_with_params("https://lrclib.net/api/get", &params).ok()?;
    crate::stats::connection(url.as_str(), "歌詞(LRCLIB)");
    let body = web_client()?.get(url).send().await.ok()?.bytes().await.ok()?;
    let json: serde_json::Value = serde_json::from_slice(&body).ok()?;
    ["syncedLyrics", "plainLyrics"].iter().find_map(|k| json[k].as_str().filter(|s| !s.trim().is_empty()).map(String::from))
}

/// 動画から音声だけを取り出す(元の音声をそのまま、無理なら m4a に変換)。できたファイルを返す
pub async fn extract_audio(video: &Path) -> Result<PathBuf> {
    let mut output = video.with_extension("m4a");
    let mut n = 2;
    while output.exists() {
        output = video.with_file_name(format!("{} ({n}).m4a", video.file_stem().unwrap_or_default().to_string_lossy()));
        n += 1;
    }
    let mut cmd = command(&ffmpeg_path());
    cmd.args(["-y", "-loglevel", "error", "-i"]).arg(video).args(["-vn", "-c:a", "aac", "-b:a", "192k"]).arg(&output);
    let result = cmd.output().await?;
    if !result.status.success() {
        bail!("音声を取り出せませんでした: {}", String::from_utf8_lossy(&result.stderr).trim());
    }
    Ok(output)
}

/// 音声から、波形が動く動画(SNS に上げられる MP4)を作る。できたファイルを返す
pub async fn waveform_video(audio: &Path) -> Result<PathBuf> {
    let mut output = audio.with_extension("waveform.mp4");
    let mut n = 2;
    while output.exists() {
        output = audio.with_file_name(format!("{} ({n}).waveform.mp4", audio.file_stem().unwrap_or_default().to_string_lossy()));
        n += 1;
    }
    let mut cmd = command(&ffmpeg_path());
    cmd.args(["-y", "-loglevel", "error", "-i"]).arg(audio).args([
        "-filter_complex", "[0:a]showwaves=s=1280x720:mode=cline:colors=0x3b6cf6|0x8ab4ff,format=yuv420p[v]",
        "-map", "[v]", "-map", "0:a", "-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-c:a", "aac", "-b:a", "192k", "-shortest",
    ]).arg(&output);
    let result = cmd.output().await?;
    if !result.status.success() {
        let _ = std::fs::remove_file(&output);
        bail!("波形動画を作れませんでした: {}", String::from_utf8_lossy(&result.stderr).trim());
    }
    Ok(output)
}

// ───────────── スマホとのやりとり(ntfy) ─────────────
// ntfy: 決めた「トピック名」にメッセージを送ると、そのトピックを登録したスマホに届く無料のしくみ。
// 受け取りは「トピック名-send」に送られた URL を拾う(通知と受け取りを分けて、送ったものを自分で拾わないように)。

fn ntfy_url(server: &str, topic: &str) -> Option<String> {
    let server = server.trim().trim_end_matches('/');
    let valid_topic = !topic.is_empty() && topic.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    (server.starts_with("https://") && valid_topic).then(|| format!("{server}/{topic}"))
}

pub async fn ntfy_send(server: &str, topic: &str, title: &str, body: &str) -> Result<()> {
    let url = ntfy_url(server, topic).context("トピック名は英数字と - _ だけ、サーバーは https:// で始めてください")?;
    crate::stats::connection(&url, "スマホへの通知");
    let client = crate::net::client(std::time::Duration::from_secs(15), "Grabbit").map_err(anyhow::Error::msg)?;
    client.post(url).header("Title", title).body(body.to_string()).send().await?.error_for_status()?;
    Ok(())
}

/// since(UNIX秒)より後に「トピック名-send」へ送られた URL を集める。since は進める
pub async fn ntfy_receive(server: &str, topic: &str, since: &mut u64) -> Vec<String> {
    let Some(url) = ntfy_url(server, &format!("{topic}-send")) else { return Vec::new() };
    let Ok(client) = crate::net::client(std::time::Duration::from_secs(20), "Grabbit") else { return Vec::new() };
    crate::stats::connection(&url, "スマホからの受け取り");
    let Ok(response) = client.get(format!("{url}/json?poll=1&since={since}")).send().await else { return Vec::new() };
    let Ok(body) = response.bytes().await else { return Vec::new() };
    let mut urls = Vec::new();
    for line in String::from_utf8_lossy(&body).lines() {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if message["event"] != "message" {
            continue;
        }
        if let Some(time) = message["time"].as_u64() {
            *since = (*since).max(time + 1);
        }
        let text = format!("{} {}", message["message"].as_str().unwrap_or_default(), message["click"].as_str().unwrap_or_default());
        urls.extend(crate::download::extract_urls(&text));
    }
    urls.truncate(50);
    urls
}

// ───────────── 保存後の検査 ─────────────

/// 保存した動画・音声が壊れていないか調べる(途中で切れている・中身が無い など)。
/// 長さを読み取り、最後の数秒を実際に読んでみる。全部を読むと時間がかかるので最後だけ見る。
pub async fn verify_media(path: &Path) -> std::result::Result<(), String> {
    let size = std::fs::metadata(path).map(|m| m.len()).map_err(|_| "ファイルが見つかりません".to_string())?;
    if size == 0 {
        return Err("ファイルが空です".into());
    }
    let exe = |name: &str| tools::bin_dir().join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() });
    let ffprobe = exe("ffprobe");
    if !ffprobe.exists() {
        return Ok(()); // 調べる道具が無い時は、壊れているとは言わない
    }
    let mut probe = command(&ffprobe);
    probe.args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(path);
    let output = tokio::time::timeout(std::time::Duration::from_secs(60), probe.output())
        .await
        .map_err(|_| "検査が時間切れになりました".to_string())?
        .map_err(|e| e.to_string())?;
    let duration: f64 = String::from_utf8_lossy(&output.stdout).trim().parse().unwrap_or(0.0);
    if !output.status.success() || duration <= 0.0 {
        let detail = String::from_utf8_lossy(&output.stderr).lines().next().unwrap_or_default().to_string();
        return Err(format!("再生できる形になっていません {detail}").trim().to_string());
    }
    // 最後の 5 秒を読んでみる(途中で切れたファイルはここでエラーになる)
    let mut tail = command(&exe("ffmpeg"));
    tail.args(["-v", "error", "-sseof", "-5", "-i"]).arg(path).args(["-f", "null", "-"]);
    let output = tokio::time::timeout(std::time::Duration::from_secs(120), tail.output())
        .await
        .map_err(|_| "検査が時間切れになりました".to_string())?
        .map_err(|e| e.to_string())?;
    let errors = String::from_utf8_lossy(&output.stderr);
    let serious = errors.lines().filter(|l| {
        let l = l.to_lowercase();
        l.contains("invalid data") || l.contains("truncat") || l.contains("moov atom not found") || l.contains("end of file") || l.contains("corrupt")
    }).count();
    if !output.status.success() || serious > 0 {
        return Err(format!("最後まで読めません(途中で切れている可能性) {}", errors.lines().next().unwrap_or_default()).trim().to_string());
    }
    Ok(())
}

// ───────────── 切り抜き ─────────────

/// "1:02:03" / "2:03" / "123.5" を秒にする
pub fn parse_time(text: &str) -> Option<f64> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut seconds = 0.0;
    for part in &parts {
        let value: f64 = part.trim().parse().ok()?;
        if !(0.0..1e7).contains(&value) {
            return None;
        }
        seconds = seconds * 60.0 + value;
    }
    Some(seconds)
}

/// 動画・音声の一部(start〜end 秒)を別ファイルに切り出す。
/// exact = false はキーフレーム単位で一瞬(画質そのまま)、true は正確だが再エンコードで時間がかかる。
/// 元のファイルは変えない。できたファイルの場所を返す。
pub async fn trim(source: &Path, start: f64, end: f64, exact: bool) -> Result<PathBuf> {
    if !source.is_file() {
        bail!("ファイルが見つかりません");
    }
    if end <= start {
        bail!("終わりは始まりより後にしてください");
    }
    let stem = source.file_stem().context("ファイル名が分かりません")?.to_string_lossy();
    let ext = source.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
    let label = |t: f64| format!("{:02}-{:02}-{:02}", (t / 3600.0) as u64, (t / 60.0) as u64 % 60, t as u64 % 60);
    let mut output = source.with_file_name(format!("{stem} [{}～{}].{ext}", label(start), label(end)));
    let mut n = 2;
    while output.exists() {
        output = source.with_file_name(format!("{stem} [{}～{}] ({n}).{ext}", label(start), label(end)));
        n += 1;
    }

    let ffmpeg = tools::bin_dir().join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
    let mut cmd = command(&ffmpeg);
    cmd.args(["-y", "-loglevel", "error", "-ss", &format!("{start:.3}"), "-to", &format!("{end:.3}"), "-i"])
        .arg(source);
    if exact {
        cmd.args(["-map", "0", "-c:v", "libx264", "-crf", "18", "-preset", "veryfast", "-c:a", "aac", "-b:a", "192k"]);
    } else {
        cmd.args(["-map", "0", "-c", "copy", "-avoid_negative_ts", "make_zero"]);
    }
    let result = cmd.arg(&output).output().await?;
    if !result.status.success() {
        let _ = std::fs::remove_file(&output);
        bail!("切り抜きに失敗しました: {}", String::from_utf8_lossy(&result.stderr).trim());
    }
    Ok(output)
}

// ───────────── ファイル名の一括変更 ─────────────

/// 名前のひな形から新しいファイル名(拡張子なし)を作る。
/// 使える印: {title} {date}(保存日) {kind} {site} {n}(連番) {name}(今の名前)
pub fn rename_pattern(pattern: &str, entry: &crate::history::Entry, n: usize, digits: usize) -> String {
    let current = Path::new(&entry.file_path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let site = entry.url.split('/').nth(2).unwrap_or_default().trim_start_matches("www.").to_string();
    let date = {
        let days = entry.saved_at / 86_400;
        // 1970-01-01 からの日数を年月日にする(閏年を含む正確な計算)
        let z = days as i64 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        format!("{y:04}-{m:02}-{d:02}")
    };
    let name = pattern
        .replace("{title}", &entry.title)
        .replace("{date}", &date)
        .replace("{kind}", &entry.kind)
        .replace("{site}", &site)
        .replace("{n}", &format!("{n:0digits$}"))
        .replace("{name}", &current);
    // 拡張子は別に付けるので、ここではフォルダ区切りなどを無害にするだけ
    let safe = crate::fallback::safe_file_name(&name);
    safe.trim_end_matches(".download").to_string()
}

/// 名前を変える。同じフォルダに同じ名前があれば (2) などを付ける。新しい場所を返す。
pub fn rename_file(path: &Path, new_stem: &str) -> Result<PathBuf> {
    if !path.is_file() {
        bail!("ファイルが見つかりません");
    }
    let dir = path.parent().context("フォルダが分かりません")?;
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut target = dir.join(format!("{new_stem}{ext}"));
    let mut n = 2;
    while target.exists() && target != path {
        target = dir.join(format!("{new_stem} ({n}){ext}"));
        n += 1;
    }
    if target != path {
        std::fs::rename(path, &target)?;
    }
    Ok(target)
}

// ───────────── 終わったらスリープ・シャットダウン ─────────────

/// パソコンをスリープ / シャットダウンする。kind は "sleep" か "shutdown" だけ受け付ける。
pub fn power_action(kind: &str) -> Result<()> {
    let (program, args): (&str, &[&str]) = match (kind, std::env::consts::OS) {
        ("sleep", "windows") => ("rundll32.exe", &["powrprof.dll,SetSuspendState", "0,1,0"]),
        ("shutdown", "windows") => ("shutdown", &["/s", "/t", "0"]),
        ("sleep", "macos") => ("pmset", &["sleepnow"]),
        ("shutdown", "macos") => ("osascript", &["-e", "tell app \"System Events\" to shut down"]),
        ("sleep", _) => ("systemctl", &["suspend"]),
        ("shutdown", _) => ("systemctl", &["poweroff"]),
        _ => bail!("不明な操作です"),
    };
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.spawn().context("実行できませんでした")?;
    Ok(())
}

// ───────────── 空き容量 ─────────────

/// 保存先の空き容量(バイト)。保存先がまだ無ければ、ある所まで親をたどって調べる。
pub fn free_space(path: &str) -> Option<u64> {
    let mut dir = PathBuf::from(path);
    while !dir.exists() {
        dir = dir.parent()?.to_path_buf();
    }
    fs2::available_space(&dir).ok()
}

/// 空きが設定の下限を割っているか(割っていたら新しいダウンロードを始めない)
pub fn low_on_space(settings: &Settings) -> bool {
    if settings.min_free_gb == 0 {
        return false;
    }
    free_space(&settings.output_dir).is_some_and(|free| free < settings.min_free_gb * 1024 * 1024 * 1024)
}

// ───────────── 省電力 ─────────────

/// バッテリーで動いているか(分からない時は false)
pub fn on_battery() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
        // ACLineStatus: 0 = バッテリー、1 = 電源につながっている、255 = 不明
        if unsafe { GetSystemPowerStatus(&mut status) } != 0 {
            return status.ACLineStatus == 0;
        }
        false
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_dir("/sys/class/power_supply")
            .map(|entries| {
                entries.filter_map(|e| e.ok()).any(|e| {
                    let path = e.path();
                    let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
                    let status = std::fs::read_to_string(path.join("status")).unwrap_or_default();
                    kind.trim() == "Battery" && status.trim() == "Discharging"
                })
            })
            .unwrap_or(false)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}

/// 省電力で動くべきか(設定が auto ならバッテリーの時だけ、on なら常に)
pub fn power_saving(settings: &Settings) -> bool {
    match settings.power_saver.as_str() {
        "on" => true,
        "auto" => on_battery(),
        _ => false,
    }
}

// ───────────── ライブ配信の確認 ─────────────

/// 登録チャンネルの「今の配信」ページの URL。YouTube はチャンネルの末尾に /live を付ける。
pub fn live_url(channel_url: &str) -> String {
    let base = channel_url.trim_end_matches('/');
    let is_youtube_channel = (base.contains("youtube.com/@") || base.contains("youtube.com/channel/") || base.contains("youtube.com/c/"))
        && !base.ends_with("/live");
    if is_youtube_channel {
        // /videos などのタブが付いていたら外してから /live を付ける
        let channel = ["/videos", "/streams", "/shorts", "/featured"].iter().fold(base.to_string(), |acc, tab| acc.trim_end_matches(tab).to_string());
        format!("{channel}/live")
    } else {
        base.to_string()
    }
}

/// 配信中なら、その配信の URL を返す。
pub async fn current_live(settings: &Settings, channel_url: &str) -> Option<String> {
    let mut cmd = command(&tools::ytdlp_path());
    cmd.args(["--simulate", "--no-warnings", "--no-playlist", "--playlist-items", "1", "--encoding", "utf-8"])
        .args(["--print", "%(is_live)s %(webpage_url)s"])
        .args(crate::download::without_browser_cookies(&crate::download::network_args(settings)))
        .args(["--", &live_url(channel_url)]);
    let output = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output()).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?.trim();
    let (is_live, url) = line.split_once(' ')?;
    (is_live == "True" && url.starts_with("http")).then(|| url.to_string())
}

// ───────────── 起動時の引数・「送る」メニュー ─────────────

/// 起動時の引数から URL を集める。URL そのもの、.url ショートカット、URL を書いたテキストに対応。
pub fn urls_from_args(args: &[String]) -> Vec<String> {
    let mut urls = Vec::new();
    for arg in args.iter().skip(1) {
        if arg.starts_with("http://") || arg.starts_with("https://") {
            urls.extend(crate::download::extract_urls(arg));
            continue;
        }
        let path = Path::new(arg);
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !matches!(ext.as_str(), "url" | "txt" | "csv" | "webloc" | "desktop" | "html" | "htm") {
            continue;
        }
        if let Ok(bytes) = std::fs::read(path) {
            urls.extend(crate::download::extract_urls(&String::from_utf8_lossy(&bytes).replace("URL=", " ")));
        }
    }
    urls
}

/// Windows の右クリック「送る」メニューに Grabbit を入れる。
pub fn add_to_send_to() -> Result<()> {
    if !cfg!(windows) {
        bail!("この機能は Windows だけで使えます");
    }
    let exe = std::env::current_exe()?;
    let send_to = dirs::data_dir().context("フォルダが見つかりません")?.join(r"Microsoft\Windows\SendTo\Grabbit.lnk");
    // ショートカットは PowerShell(WScript.Shell)で作る
    let script = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.Save()",
        send_to.display().to_string().replace('\'', "''"),
        exe.display().to_string().replace('\'', "''"),
    );
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-Command", &script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let status = cmd.status()?;
    if !status.success() {
        bail!("「送る」メニューに追加できませんでした");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_pattern_fills_fields() {
        let entry = crate::history::Entry {
            id: 1, title: "A/B".into(), url: "https://www.youtube.com/x".into(), file_path: "/v/old.mp4".into(),
            thumbnail: String::new(), kind: "video".into(), saved_at: 1_700_000_000, size: 0, exists: true, drive_missing: false, uploader: String::new(), trashed: false, duration: 0, height: 0,
        };
        assert_eq!(rename_pattern("{n} {title} [{site}] {date}", &entry, 3, 2), "03 A_B [youtube.com] 2023-11-14");
        assert_eq!(rename_pattern("{name}", &entry, 1, 1), "old");
    }

    #[test]
    fn subtitle_fixes_are_applied() {
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nこんにちわ\n";
        assert!(apply_fixes(srt, "こんにちわ→こんにちは").contains("こんにちは"));
    }

    #[test]
    fn private_info_is_scrubbed() {
        let text = "mail me@example.com from 192.168.1.20 https://x.com/a?v=abc&token=secret";
        let out = scrub_private(text);
        assert!(!out.contains("me@example.com") && !out.contains("192.168.1.20") && !out.contains("secret"));
        assert!(out.contains("v=abc"));
    }

    #[test]
    fn auto_subtitles_are_deduplicated() {
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nhello\n\n2\n00:00:02,000 --> 00:00:03,000\nhello\n\n3\n00:00:03,000 --> 00:00:04,000\n<c>world</c>\n";
        let cleaned = clean_subtitle_text(srt, false);
        assert_eq!(cleaned.matches("hello").count(), 1);
        assert!(cleaned.contains("world") && !cleaned.contains("<c>"));
    }

    #[test]
    fn artist_and_title_are_split() {
        assert_eq!(split_artist_title("YOASOBI - アイドル (Official Music Video)"), ("YOASOBI".into(), "アイドル".into()));
        assert_eq!(split_artist_title("Song only"), (String::new(), "Song only".into()));
    }

    #[test]
    fn times_are_parsed() {
        assert_eq!(parse_time("1:02:03"), Some(3723.0));
        assert_eq!(parse_time("2:03"), Some(123.0));
        assert_eq!(parse_time("12.5"), Some(12.5));
        assert_eq!(parse_time("a:b"), None);
        assert_eq!(parse_time("-5"), None);
    }

    #[test]
    fn live_url_for_youtube_channels() {
        assert_eq!(live_url("https://www.youtube.com/@abc"), "https://www.youtube.com/@abc/live");
        assert_eq!(live_url("https://www.youtube.com/@abc/videos/"), "https://www.youtube.com/@abc/live");
        assert_eq!(live_url("https://www.twitch.tv/abc"), "https://www.twitch.tv/abc");
    }

    #[test]
    fn urls_are_read_from_args_and_files() {
        let dir = std::env::temp_dir().join("grabbit-args-test");
        let _ = std::fs::create_dir_all(&dir);
        let shortcut = dir.join("a.url");
        std::fs::write(&shortcut, "[InternetShortcut]\r\nURL=https://youtu.be/abc\r\n").unwrap();
        let args = vec!["grabbit".into(), "https://x.com/1".into(), shortcut.to_string_lossy().into_owned()];
        assert_eq!(urls_from_args(&args), ["https://x.com/1", "https://youtu.be/abc"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subtitle_is_found_by_language() {
        let dir = std::env::temp_dir().join("grabbit-sub-test");
        let _ = std::fs::create_dir_all(&dir);
        let video = dir.join("v.mp4");
        for name in ["v.mp4", "v.en.srt", "v.ja.srt", "v.live_chat.json"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        assert_eq!(find_subtitle(&video, "ja,en"), Some(dir.join("v.ja.srt")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
