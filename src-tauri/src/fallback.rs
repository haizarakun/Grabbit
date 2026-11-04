//! yt-dlp で取れなかった時の予備手段。
//! 1. gallery-dl(画像・イラスト・ギャラリー系に強い)
//! 2. 直接ダウンロード(URL がファイルそのものを指している場合)

use crate::download::{command, Event, Outcome};
use crate::settings::Settings;
use crate::tools;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::sync::Notify;

// ───────────── gallery-dl ─────────────

fn gallery_args(settings: &Settings) -> Vec<String> {
    let mut args = vec!["--config-ignore".to_string(), "--destination".to_string(), settings.output_dir.clone()];
    if !settings.cookies_file.is_empty() {
        args.extend(["--cookies".into(), settings.cookies_file.clone()]);
    } else if settings.cookies_browser == "grabbit" {
        // アプリ内でログインした情報(ブラウザ名として渡すと gallery-dl が止まる)
        if crate::login::cookie_file().exists() {
            args.extend(["--cookies".into(), crate::login::cookie_file().to_string_lossy().into_owned()]);
        }
    } else if !settings.cookies_browser.is_empty() {
        args.extend(["--cookies-from-browser".into(), settings.cookies_browser.clone()]);
    }
    if !settings.proxy.is_empty() {
        args.extend(["--proxy".into(), settings.proxy.clone()]);
    }
    args
}

/// gallery-dl で取得できるファイルの数を調べる(0 なら非対応)。
pub async fn gallery_count(settings: &Settings, url: &str) -> Result<usize> {
    let program = tools::ensure_gallery_dl().await?;
    let mut cmd = command(&program);
    cmd.args(gallery_args(settings)).args(["--get-urls", "--", url]);
    let output = tokio::time::timeout(std::time::Duration::from_secs(90), cmd.output()).await??;
    let count = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| l.starts_with("http"))
        .count();
    if count == 0 {
        bail!("gallery-dl でも取得できませんでした");
    }
    Ok(count)
}

/// gallery-dl で保存する。保存したファイルの場所を順に知らせる。
pub async fn gallery_download(
    settings: &Settings,
    url: &str,
    cancel: Arc<Notify>,
    on_event: &impl Fn(Event),
) -> Outcome {
    gallery_download_with(settings, url, None, cancel, on_event).await
}

/// gallery-dl の記録ファイル(SNS の自動保存で、保存済みの投稿を飛ばす)
pub fn gallery_archive() -> PathBuf {
    crate::settings::data_dir().join("gallery_archive.sqlite3")
}

/// archive を渡すと、保存済みの投稿を飛ばす(自動保存用)
pub async fn gallery_download_with(
    settings: &Settings,
    url: &str,
    archive: Option<&Path>,
    cancel: Arc<Notify>,
    on_event: &impl Fn(Event),
) -> Outcome {
    let program = match tools::ensure_gallery_dl().await {
        Ok(p) => p,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    let mut cmd = command(&program);
    cmd.args(gallery_args(settings));
    if let Some(archive) = archive {
        cmd.arg("--download-archive").arg(archive);
    }
    cmd.args(["--", url])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Outcome::Failed(e.to_string()),
    };

    let stderr = child.stderr.take().expect("stderr is piped");
    let error_tail = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut last = String::new();
        while let Some(line) = crate::download::read_line_lossy(&mut reader).await {
            if line.contains("[error]") {
                last = line;
            }
        }
        last
    });

    let mut reader = BufReader::new(child.stdout.take().expect("stdout is piped"));
    let mut saved = 0usize;
    let read = async {
        while let Some(line) = crate::download::read_line_lossy(&mut reader).await {
            // 保存したファイルは1行に1つ。"# " で始まる行は保存済みで飛ばしたもの(新しく保存したことにはしない)
            if line.starts_with("# ") {
                continue;
            }
            let path = line.trim();
            if Path::new(path).is_absolute() || path.contains(std::path::MAIN_SEPARATOR) {
                saved += 1;
                on_event(Event::File(path.to_string()));
                on_event(Event::Progress { percent: 100.0, speed: format!("{saved} 件"), eta: String::new() });
            }
        }
    };
    tokio::select! {
        _ = read => {}
        _ = cancel.notified() => {
            let _ = child.kill().await;
            return Outcome::Canceled;
        }
    }
    let ok = child.wait().await.map(|s| s.success()).unwrap_or(false);
    let last_error = error_tail.await.unwrap_or_default();
    match (ok, saved) {
        (true, _) | (false, 1..) => Outcome::Done,
        _ if last_error.is_empty() => Outcome::Failed("gallery-dl でも取得できませんでした".into()),
        _ => Outcome::Failed(last_error),
    }
}

// ───────────── 直接ダウンロード ─────────────

fn http_client(settings: &Settings) -> Result<reqwest::Client> {
    if settings.proxy_only && settings.proxy.trim().is_empty() {
        bail!("「プロキシだけを通す」がオンですが、プロキシが入っていません");
    }
    let mut builder = reqwest::Client::builder().user_agent(if settings.user_agent.is_empty() {
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36"
    } else {
        &settings.user_agent
    });
    if !settings.proxy.is_empty() {
        builder = builder.proxy(reqwest::Proxy::all(&settings.proxy)?);
    }
    Ok(builder.build()?)
}

/// URL がページではなくファイルそのものか調べる。ファイルなら (ファイル名, サイズ) を返す。
pub async fn probe_direct(settings: &Settings, url: &str) -> Result<(String, Option<u64>)> {
    let response = http_client(settings)?.get(url).send().await?.error_for_status()?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_lowercase();
    if content_type.starts_with("text/html") || content_type.contains("xml") {
        bail!("ファイルではなくWebページです");
    }
    Ok((file_name_of(&response, url), response.content_length()))
}

/// ファイル名は Content-Disposition → URL の末尾 の順で決める。
fn file_name_of(response: &reqwest::Response, url: &str) -> String {
    let from_header = response
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split("filename=").nth(1))
        .map(|name| name.trim_matches(['"', ';', ' ']).to_string());
    let from_url = url
        .split(['?', '#'])
        .next()
        .and_then(|path| path.rsplit('/').next())
        .filter(|name| !name.is_empty())
        .map(String::from);
    safe_file_name(&from_header.or(from_url).unwrap_or_default())
}

/// 開くとプログラムとして動いてしまう拡張子。サイトから来た名前にこれが付いていたら無害な名前に変える
const RUNNABLE_EXTENSIONS: &[&str] = &[
    "exe", "com", "bat", "cmd", "scr", "pif", "cpl", "msi", "msp", "ps1", "psm1", "vbs", "vbe", "js", "jse",
    "wsf", "wsh", "hta", "lnk", "url", "reg", "jar", "app", "command", "sh", "desktop", "appimage", "deb", "dmg", "pkg",
];

/// 開いた時にプログラムとして動く種類のファイルか
pub fn is_runnable(path: &Path) -> bool {
    path.extension().is_some_and(|e| RUNNABLE_EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str()))
}

/// サイトから来たファイル名を安全な形にする。
/// フォルダをまたぐ指定(../)・制御文字・Windows の予約名・実行ファイルの拡張子を無害にする。
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || r#"\/:*?"<>|"#.contains(c) { '_' } else { c })
        .collect();
    let mut cleaned = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace()).chars().take(150).collect::<String>();
    if cleaned.is_empty() {
        cleaned = "download".into();
    }
    let stem = cleaned.split('.').next().unwrap_or_default().to_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4);
    if reserved {
        cleaned = format!("_{cleaned}");
    }
    if is_runnable(Path::new(&cleaned)) {
        cleaned.push_str(".download");
    }
    cleaned
}

/// ファイルをそのまま保存する。`base_name` を渡すと、拡張子だけ元のままその名前で保存する。
pub async fn direct_download(
    settings: &Settings,
    url: &str,
    base_name: Option<&str>,
    cancel: Arc<Notify>,
    on_event: &impl Fn(Event),
) -> Outcome {
    let run = async {
        let mut response = http_client(settings)?.get(url).send().await?.error_for_status()?;
        let original = file_name_of(&response, url);
        let name = match base_name {
            Some(base) => {
                let ext = Path::new(&original).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "jpg".into());
                safe_file_name(&format!("{base}.{ext}"))
            }
            None => {
                on_event(Event::Title(original.clone()));
                original
            }
        };
        let dir = PathBuf::from(&settings.output_dir);
        tokio::fs::create_dir_all(&dir).await?;
        let path = unique_path(&dir.join(&name));
        let part = path.with_extension("part");
        let mut file = tokio::fs::File::create(&part).await.context("保存先に書き込めません")?;

        let total = response.content_length();
        let started = std::time::Instant::now();
        let mut received: u64 = 0;
        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk).await?;
            received += chunk.len() as u64;
            let seconds = started.elapsed().as_secs_f64().max(0.1);
            let percent = total.map_or(0.0, |t| received as f32 / t as f32 * 100.0);
            on_event(Event::Progress {
                percent,
                speed: format!("{:.1}MiB/s", received as f64 / seconds / 1_048_576.0),
                eta: String::new(),
            });
        }
        file.flush().await?;
        drop(file);
        tokio::fs::rename(&part, &path).await?;
        on_event(Event::File(path.to_string_lossy().into_owned()));
        anyhow::Ok(())
    };
    tokio::select! {
        result = run => match result {
            Ok(()) => Outcome::Done,
            Err(e) => Outcome::Failed(e.to_string()),
        },
        _ = cancel.notified() => Outcome::Canceled,
    }
}

/// 同名ファイルがあれば「名前 (2).拡張子」のようにずらす。
fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..)
        .map(|n| path.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("空いている名前は必ず見つかる")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_names_are_made_harmless() {
        assert_eq!(safe_file_name(".."), "download");
        assert_eq!(safe_file_name("../../a.mp4"), "_.._a.mp4");
        assert_eq!(safe_file_name("CON.txt"), "_CON.txt");
        assert_eq!(safe_file_name("setup.EXE"), "setup.EXE.download");
        assert_eq!(safe_file_name("photo.jpg"), "photo.jpg");
    }

    #[test]
    fn unique_path_adds_number() {
        let dir = std::env::temp_dir().join("grabbit-test-unique");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("a.mp4");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(unique_path(&file), dir.join("a (2).mp4"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
