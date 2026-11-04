//! 外部ツール(yt-dlp / ffmpeg / Deno)の自動導入と自動更新。
//! すべてアプリ専用フォルダに置くので、ユーザーは何もインストールしなくてよい。

use crate::settings::data_dir;
use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };
/// ffmpeg / Deno を取り直す間隔
const REFRESH_AFTER: Duration = Duration::from_secs(30 * 24 * 60 * 60);

// ───────────── 部品ごとの取得元(GitHub リポジトリ) ─────────────
// 空なら標準の取得元。自分で追加したリポジトリも選べる(ハッシュの照合はそのリポジトリの一覧で行う)。

/// 部品の名前・表示名・標準の取得元
pub const TOOLS: [(&str, &str, &str); 4] = [
    ("yt-dlp", "yt-dlp(ダウンロード機能)", "yt-dlp/yt-dlp-nightly-builds"),
    ("ffmpeg", "FFmpeg(変換機能)", "yt-dlp/FFmpeg-Builds"),
    ("deno", "Deno(YouTube対策用)", "denoland/deno"),
    ("gallery-dl", "gallery-dl(画像の予備エンジン)", "gdl-org/builds"),
];

static REPOS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

/// 設定の「部品ごとの取得元」を反映する
pub fn configure(repos: &BTreeMap<String, String>, disabled: &[String]) {
    *REPOS.lock().unwrap() = repos.iter().filter(|(_, r)| is_valid_repo(r)).map(|(k, v)| (k.clone(), v.clone())).collect();
    // yt-dlp は無いと何も保存できないので、切れないようにする
    *DISABLED.lock().unwrap() = disabled.iter().filter(|n| n.as_str() != "yt-dlp").cloned().collect();
}

// 使わない(オフにした)部品。入れない・更新しない・使わない
static DISABLED: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn is_enabled(tool: &str) -> bool {
    !DISABLED.lock().unwrap().iter().any(|n| n == tool)
}

/// 「持ち主/リポジトリ名」の形か(URL や記号での抜け道を作らない)
pub fn is_valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok = |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p != "." && p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)));
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// 自分で選んだ取得元(無ければ None)
fn chosen_repo(tool: &str) -> Option<String> {
    REPOS.lock().unwrap().get(tool).cloned()
}

/// 部品の状態(設定画面の一覧用)
#[derive(serde::Serialize)]
pub struct ToolInfo {
    pub name: &'static str,
    pub label: &'static str,
    pub enabled: bool,
    pub installed: bool,
    pub size: u64,
    pub repo: String,
    pub default_repo: &'static str,
}

fn tool_files(name: &str) -> Vec<PathBuf> {
    match name {
        "yt-dlp" => vec![ytdlp_dir()],
        "ffmpeg" => vec![bin_dir().join(format!("ffmpeg{EXE}")), bin_dir().join(format!("ffprobe{EXE}"))],
        "deno" => vec![bin_dir().join(format!("deno{EXE}"))],
        "gallery-dl" => vec![bin_dir().join(format!("gallery-dl{EXE}"))],
        _ => Vec::new(),
    }
}

fn size_of(path: &Path) -> u64 {
    if path.is_dir() {
        std::fs::read_dir(path).map(|entries| entries.flatten().map(|e| size_of(&e.path())).sum()).unwrap_or(0)
    } else {
        path.metadata().map(|m| m.len()).unwrap_or(0)
    }
}

pub fn list_tools(channel: &str) -> Vec<ToolInfo> {
    TOOLS
        .iter()
        .map(|&(name, label, default_repo)| {
            let files = tool_files(name);
            let default_repo = if name == "yt-dlp" { release_repo(channel) } else { default_repo };
            ToolInfo {
                name,
                label,
                enabled: is_enabled(name),
                installed: files.first().is_some_and(|f| f.exists()),
                size: files.iter().map(|f| size_of(f)).sum(),
                repo: chosen_repo(name).unwrap_or_default(),
                default_repo,
            }
        })
        .collect()
}

/// 部品を1つ消す(yt-dlp と FFmpeg は次に使う時・次の起動時に入れ直される)
pub fn remove_tool(name: &str) -> Result<()> {
    let files = tool_files(name);
    if files.is_empty() {
        bail!("知らない部品です: {name}");
    }
    for path in files {
        let result = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        if let Err(e) = result {
            if path.exists() {
                bail!("消せませんでした(使用中かもしれません): {e}");
            }
        }
    }
    if name == "yt-dlp" {
        let _ = std::fs::remove_dir_all(bin_dir().join("yt-dlp.prev"));
    }
    record_manifest();
    Ok(())
}

/// 部品を1つ入れ直す(今の取得元から)
pub async fn reinstall_tool(name: &str, channel: &str) -> Result<()> {
    if !is_enabled(name) {
        bail!("この部品はオフになっています。オンにしてから入れ直してください");
    }
    remove_tool(name)?;
    match name {
        "yt-dlp" => install_ytdlp(channel).await?,
        "ffmpeg" => install_ffmpeg().await?,
        "deno" => install_deno().await?,
        "gallery-dl" => {
            ensure_gallery_dl().await?;
        }
        _ => bail!("知らない部品です: {name}"),
    }
    record_manifest();
    Ok(())
}

pub fn bin_dir() -> PathBuf {
    let dir = data_dir().join("bin");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// yt-dlp は「展開済みフォルダ版」を使う。
/// 単一 exe 版は起動のたびに自分を一時フォルダへ展開するので、1回の呼び出しが数倍遅い。
fn ytdlp_dir() -> PathBuf {
    bin_dir().join("yt-dlp")
}

/// OS ごとの配布ファイル名(zip)と、その中の実行ファイル名
fn ytdlp_asset() -> Result<(&'static str, &'static str)> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "aarch64") => ("yt-dlp_win_arm64.zip", "yt-dlp.exe"),
        ("windows", _) => ("yt-dlp_win.zip", "yt-dlp.exe"),
        ("macos", _) => ("yt-dlp_macos.zip", "yt-dlp_macos"),
        ("linux", "aarch64") => ("yt-dlp_linux_aarch64.zip", "yt-dlp_linux_aarch64"),
        ("linux", _) => ("yt-dlp_linux.zip", "yt-dlp_linux"),
        (os, _) => bail!("未対応のOSです: {os}"),
    })
}

pub fn ytdlp_path() -> PathBuf {
    let exe = ytdlp_asset().map(|(_, exe)| exe).unwrap_or("yt-dlp");
    ytdlp_dir().join(exe)
}

/// 子プロセスに渡す PATH。同梱ツールを最優先で見つけさせる。
pub fn child_path_env() -> std::ffi::OsString {
    let mut paths = vec![bin_dir()];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).unwrap_or_default()
}

/// 必要なツールが揃っていて、すぐダウンロードを始められるか。
pub fn is_installed() -> bool {
    ytdlp_path().exists() && (!is_enabled("ffmpeg") || bin_dir().join(format!("ffmpeg{EXE}")).exists())
}

/// 足りないツールだけを入れる(初回のみ時間がかかる)。
pub async fn install_missing(channel: &str, report: impl Fn(&str)) -> Result<()> {
    finish_pending_update();
    if !ytdlp_path().exists() {
        report("初期ダウンロード(yt-dlp)");
        install_ytdlp(channel).await?;
    }

    if is_enabled("ffmpeg") && needs_refresh(&bin_dir().join(format!("ffmpeg{EXE}"))) {
        report("初期ダウンロード(FFmpeg)");
        install_ffmpeg().await?;
    }

    if is_enabled("deno") && needs_refresh(&bin_dir().join(format!("deno{EXE}"))) {
        report("初期ダウンロード(Deno)");
        // Deno が無くても多くのサイトは動くので、失敗しても止めない
        if let Err(e) = install_deno().await {
            report(&format!("Deno の導入に失敗しました: {e}"));
        }
    }
    Ok(())
}

/// 更新チャンネルの名前を正規化する。不明な値は nightly 扱い。
pub fn normalize_channel(channel: &str) -> &'static str {
    match channel {
        "stable" => "stable",
        "master" => "master",
        _ => "nightly",
    }
}

/// 指定チャンネルの最新版へ更新する(チャンネル切替も兼ねる)。失敗しても今の版を使い続ける。
pub async fn update_ytdlp(channel: &str) {
    let _only_one = INSTALL_LOCK.lock().await;
    let _ = install_ytdlp(channel).await;
}

/// 配布元のハッシュ一覧と手元の版を比べ、違っていれば最新版に入れ替える。
async fn install_ytdlp(channel: &str) -> Result<()> {
    let (zip_name, _) = ytdlp_asset()?;
    let repo = chosen_repo("yt-dlp").unwrap_or_else(|| release_repo(channel).to_string());
    let base = format!("https://github.com/{repo}/releases/latest/download");

    let sums = String::from_utf8(fetch(&format!("{base}/SHA2-256SUMS")).await?)?;
    let latest_hash = sums
        .lines()
        .find_map(|line| line.strip_suffix(&format!("  {zip_name}")))
        .context("配布ファイルのハッシュが見つかりません")?
        .to_string();
    let marker = ytdlp_dir().join(".sha256");
    let installed_hash = std::fs::read_to_string(&marker).unwrap_or_default();
    if installed_hash.trim() == latest_hash && ytdlp_path().exists() {
        return Ok(());
    }

    let data = fetch(&format!("{base}/{zip_name}")).await?;
    if sha256_hex(&data) != latest_hash {
        bail!("ダウンロードした yt-dlp が壊れています(ハッシュ不一致)");
    }

    // 新しい版は別フォルダに展開してから入れ替える(使用中でも壊さないため)
    let staging = bin_dir().join("yt-dlp.new");
    let _ = std::fs::remove_dir_all(&staging);
    extract_zip_all(&data, &staging)?;
    std::fs::write(staging.join(".sha256"), &latest_hash)?;
    finish_pending_update();
    Ok(())
}

/// 展開済みの新しい版があれば、今の版と入れ替える。
/// 実行中で入れ替えられなければ、次回起動時にもう一度試す。
fn finish_pending_update() {
    let staging = bin_dir().join("yt-dlp.new");
    if !staging.exists() {
        return;
    }
    let current = ytdlp_dir();
    let old = bin_dir().join("yt-dlp.old");
    let _ = std::fs::remove_dir_all(&old);
    // 以前の単一ファイル版が同じ名前で残っていれば消す
    if current.is_file() {
        let _ = std::fs::remove_file(&current);
    }
    if current.exists() && std::fs::rename(&current, &old).is_err() {
        return;
    }
    if std::fs::rename(&staging, &current).is_err() {
        let _ = std::fs::rename(&old, &current);
        return;
    }
    // 1つ前の版は「前の版に戻す」用に残しておく
    let previous = bin_dir().join("yt-dlp.prev");
    let _ = std::fs::remove_dir_all(&previous);
    let _ = std::fs::rename(&old, &previous);
}

/// 1つ前の版に戻す(今の版と入れ替える)
pub fn rollback_ytdlp() -> Result<()> {
    let previous = bin_dir().join("yt-dlp.prev");
    if !previous.exists() {
        bail!("前の版が残っていません(まだ一度も更新していないか、消えています)");
    }
    let current = ytdlp_dir();
    let swap = bin_dir().join("yt-dlp.swap");
    let _ = std::fs::remove_dir_all(&swap);
    std::fs::rename(&current, &swap).context("使用中のため入れ替えられません。ダウンロードが終わってから試してください")?;
    if let Err(e) = std::fs::rename(&previous, &current) {
        let _ = std::fs::rename(&swap, &current);
        bail!("入れ替えられませんでした: {e}");
    }
    std::fs::rename(&swap, &previous)?;
    Ok(())
}

/// 今の更新チャンネルの最近の変更点(GitHub のリリースの説明)を返す: [(版, 日付, 本文)]
pub async fn ytdlp_changelog(channel: &str) -> Result<Vec<(String, String, String)>> {
    let data = fetch(&format!("https://api.github.com/repos/{}/releases?per_page=5", release_repo(channel))).await?;
    let releases: serde_json::Value = serde_json::from_slice(&data)?;
    Ok(releases
        .as_array()
        .map(|list| {
            list.iter()
                .map(|r| {
                    let text = |k: &str| r[k].as_str().unwrap_or_default().to_string();
                    let body: String = text("body").chars().take(4000).collect();
                    (text("tag_name"), text("published_at").chars().take(10).collect(), body)
                })
                .collect()
        })
        .unwrap_or_default())
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// GitHub の最新リリースから配布ファイルを取り、GitHub が公開しているハッシュ(digest)と照合する。
/// 照合できない時は使わない(途中で差し替えられた物を動かさないため)。
async fn fetch_github_asset_verified(repo: &str, asset: &str) -> Result<Vec<u8>> {
    let api = fetch(&format!("https://api.github.com/repos/{repo}/releases/latest")).await?;
    let release: serde_json::Value = serde_json::from_slice(&api).context("リリース情報を読めません")?;
    let entry = release["assets"]
        .as_array()
        .and_then(|assets| assets.iter().find(|a| a["name"] == asset))
        .context("配布ファイルが見つかりません")?;
    let expected = entry["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).context("配布ファイルのハッシュがありません")?;
    let url = entry["browser_download_url"].as_str().context("配布ファイルの場所が分かりません")?;
    if !url.starts_with("https://github.com/") {
        bail!("配布元が GitHub ではありません");
    }
    let data = fetch(url).await?;
    if sha256_hex(&data) != expected.to_lowercase() {
        bail!("ダウンロードした {asset} が壊れています(ハッシュ不一致)");
    }
    Ok(data)
}

/// gallery-dl(画像・ギャラリー系の予備エンジン)。必要になった時に初めて取得する。
pub async fn ensure_gallery_dl() -> Result<PathBuf> {
    if !is_enabled("gallery-dl") {
        bail!("gallery-dl(画像の予備エンジン)はオフになっています");
    }
    let path = bin_dir().join(format!("gallery-dl{EXE}"));
    if needs_refresh(&path) {
        let asset = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86") => "gallery-dl_windows_x86.exe",
            ("windows", _) => "gallery-dl_windows.exe",
            ("macos", _) => "gallery-dl_macos",
            ("linux", _) => "gallery-dl_linux",
            (os, _) => bail!("未対応のOSです: {os}"),
        };
        let repo = chosen_repo("gallery-dl").unwrap_or_else(|| "gdl-org/builds".into());
        match fetch_github_asset_verified(&repo, asset).await {
            Ok(data) => write_binary(&path, &data)?,
            // 取得に失敗しても、古い版があればそれを使う
            Err(e) if !path.exists() => return Err(e),
            Err(_) => {}
        }
    }
    Ok(path)
}

// ───────────── 部品の改ざんチェック ─────────────
// 入れた時の部品の指紋(SHA-256)を控えておき、起動のたびに同じか確かめる。
// 違っていたら(ウイルスなどに書き換えられた可能性)、入れ直す。

fn manifest_path() -> PathBuf {
    data_dir().join("tools_manifest.json")
}

/// 確かめる部品の一覧(実際にある物だけ)
fn checked_tools() -> Vec<PathBuf> {
    [ytdlp_path(), bin_dir().join(format!("ffmpeg{EXE}")), bin_dir().join(format!("ffprobe{EXE}")), bin_dir().join(format!("deno{EXE}")), bin_dir().join(format!("gallery-dl{EXE}"))]
        .into_iter()
        .filter(|p| p.exists())
        .collect()
}

fn file_hash(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|data| sha256_hex(&data))
}

/// 今の部品の指紋を控える(入れた・更新した後に呼ぶ)
pub fn record_manifest() {
    let map: std::collections::BTreeMap<String, String> = checked_tools()
        .into_iter()
        .filter_map(|p| Some((p.to_string_lossy().into_owned(), file_hash(&p)?)))
        .collect();
    crate::settings::save_json("tools_manifest.json", &map);
}

/// 書き換えられていた部品を消す(このあと入れ直される)
pub fn remove_tampered() -> Vec<String> {
    let changed = verify_manifest();
    for name in &changed {
        if ytdlp_path().file_name().is_some_and(|n| n.to_string_lossy() == *name) {
            let _ = std::fs::remove_dir_all(ytdlp_dir());
        } else {
            let _ = std::fs::remove_file(bin_dir().join(name));
        }
    }
    changed
}

/// 控えと違う部品の名前を返す。控えが無ければ今の状態を控えて、何も返さない
pub fn verify_manifest() -> Vec<String> {
    if !manifest_path().exists() {
        record_manifest();
        return Vec::new();
    }
    let map: std::collections::BTreeMap<String, String> = crate::settings::load_json("tools_manifest.json");
    map.iter()
        .filter(|(path, hash)| Path::new(path).exists() && file_hash(Path::new(path)).as_ref() != Some(hash))
        .map(|(path, _)| Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
        .collect()
}

/// 現在の yt-dlp のバージョン。取れなければ空文字。
pub async fn ytdlp_version() -> String {
    let mut cmd = crate::download::command(&ytdlp_path());
    cmd.arg("--version");
    match cmd.output().await {
        Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Err(_) => String::new(),
    }
}

/// 対応サイト(抽出器)の一覧。yt-dlp に問い合わせ、版ごとにキャッシュする。
pub async fn supported_sites() -> Result<String> {
    let cache = data_dir().join("sites.txt");
    let version_file = ytdlp_dir().join(".sha256");
    let cache_is_fresh = match (std::fs::metadata(&cache), std::fs::metadata(&version_file)) {
        (Ok(c), Ok(v)) => c.modified()? >= v.modified()?,
        (Ok(_), Err(_)) => true,
        _ => false,
    };
    if cache_is_fresh {
        return Ok(std::fs::read_to_string(&cache)?);
    }
    let mut cmd = crate::download::command(&ytdlp_path());
    cmd.arg("--list-extractors");
    let output = cmd.output().await?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !text.is_empty() {
        std::fs::write(&cache, &text)?;
    }
    Ok(text)
}

fn needs_refresh(path: &Path) -> bool {
    let modified = std::fs::metadata(path).and_then(|m| m.modified());
    match modified {
        Ok(time) => SystemTime::now()
            .duration_since(time)
            .map(|age| age > REFRESH_AFTER)
            .unwrap_or(false),
        Err(_) => true,
    }
}

/// チャンネルごとの配布元リポジトリ
fn release_repo(channel: &str) -> &'static str {
    match normalize_channel(channel) {
        "stable" => "yt-dlp/yt-dlp",
        "master" => "yt-dlp/yt-dlp-master-builds",
        _ => "yt-dlp/yt-dlp-nightly-builds",
    }
}

async fn install_ffmpeg() -> Result<()> {
    let chosen = chosen_repo("ffmpeg");
    let base = format!("https://github.com/{}/releases/download/latest", chosen.as_deref().unwrap_or("yt-dlp/FFmpeg-Builds"));
    let base = base.as_str();
    // 標準の取得元には macOS 版が無いので別の配布元を使う(取得元を選んだ時はそちらを使う)
    if std::env::consts::OS == "macos" && chosen.is_some() {
        bail!("macOS では FFmpeg の取得元を変えられません。標準の取得元に戻してください");
    }
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", _) => {
            let data = fetch_verified(base, "ffmpeg-master-latest-win64-gpl.zip").await?;
            extract_zip(&data, &["ffmpeg.exe", "ffprobe.exe"])
        }
        ("linux", arch) => {
            let name = if arch == "aarch64" { "linuxarm64" } else { "linux64" };
            let data = fetch_verified(base, &format!("ffmpeg-master-latest-{name}-gpl.tar.xz")).await?;
            extract_tar_xz(&data, &["ffmpeg", "ffprobe"])
        }
        ("macos", _) => {
            for tool in ["ffmpeg", "ffprobe"] {
                let data = fetch(&format!("https://evermeet.cx/ffmpeg/getrelease/{tool}/zip")).await?;
                extract_zip(&data, &[tool])?;
            }
            Ok(())
        }
        (os, _) => bail!("未対応のOSです: {os}"),
    }
}

async fn install_deno() -> Result<()> {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", _) => "x86_64-pc-windows-msvc",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", _) => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", _) => "x86_64-unknown-linux-gnu",
        (os, _) => bail!("未対応のOSです: {os}"),
    };
    let repo = chosen_repo("deno").unwrap_or_else(|| "denoland/deno".into());
    let url = format!("https://github.com/{repo}/releases/latest/download/deno-{target}.zip");
    let data = fetch(&url).await?;
    // 配布元のハッシュと照らし合わせて、壊れた・すり替えられたファイルを使わない
    let sums = String::from_utf8_lossy(&fetch(&format!("{url}.sha256sum")).await?).to_lowercase();
    if !sums.contains(&sha256_hex(&data)) {
        bail!("ダウンロードした Deno が壊れています(ハッシュ不一致)");
    }
    extract_zip(&data, &[&format!("deno{EXE}")])
}

/// 配布元の checksums.sha256 と照らし合わせてから使う
async fn fetch_verified(base: &str, file: &str) -> Result<Vec<u8>> {
    let sums = String::from_utf8(fetch(&format!("{base}/checksums.sha256")).await?)?;
    let expected = sums
        .lines()
        .find_map(|line| line.strip_suffix(&format!("  {file}")))
        .with_context(|| format!("{file} のハッシュが見つかりません"))?
        .to_string();
    let data = fetch(&format!("{base}/{file}")).await?;
    if sha256_hex(&data) != expected {
        bail!("ダウンロードした {file} が壊れています(ハッシュ不一致)");
    }
    Ok(data)
}

// ───────────── 自動修復 ─────────────

/// ダウンロードに使う部品が正しく動くか確かめ、動かないものは入れ直す。
/// 戻り値は直した部品の名前。
pub async fn repair(channel: &str) -> Result<Vec<&'static str>> {
    let _only_one = INSTALL_LOCK.lock().await;
    let mut fixed = Vec::new();
    if ytdlp_version().await.is_empty() {
        let _ = std::fs::remove_dir_all(ytdlp_dir());
        let _ = std::fs::remove_file(ytdlp_dir().join(".sha256"));
        install_ytdlp(channel).await?;
        fixed.push("yt-dlp");
    }
    if is_enabled("ffmpeg") && !runs(&bin_dir().join(format!("ffmpeg{EXE}")), "-version").await {
        let _ = std::fs::remove_file(bin_dir().join(format!("ffmpeg{EXE}")));
        install_ffmpeg().await?;
        fixed.push("ffmpeg");
    }
    if is_enabled("deno") && !runs(&bin_dir().join(format!("deno{EXE}")), "--version").await {
        let _ = std::fs::remove_file(bin_dir().join(format!("deno{EXE}")));
        if install_deno().await.is_ok() {
            fixed.push("deno");
        }
    }
    Ok(fixed)
}

/// そのプログラムが起動して正常に終わるか
async fn runs(program: &Path, arg: &str) -> bool {
    if !program.exists() {
        return false;
    }
    let mut cmd = crate::download::command(program);
    cmd.arg(arg);
    matches!(tokio::time::timeout(std::time::Duration::from_secs(20), cmd.output()).await, Ok(Ok(out)) if out.status.success())
}

// ダウンロード中の部品の進み具合(初回の準備画面で % を出すため)
static FETCH_DONE: AtomicU64 = AtomicU64::new(0);
static FETCH_TOTAL: AtomicU64 = AtomicU64::new(0);

/// 今ダウンロードしている部品の (受け取った量, 全体の量)。全体が分からない時は 0
pub fn fetch_progress() -> (u64, u64) {
    (FETCH_DONE.load(Ordering::Relaxed), FETCH_TOTAL.load(Ordering::Relaxed))
}

tokio::task_local! {
    /// この中で行うダウンロードだけを「何%」に数える(裏の更新などで数字が混ざらないように)
    static SHOW_PROGRESS: ();
}

// 部品を入れる作業は一度に1つだけ(同じ部品を二重に落としたり、進み具合が混ざったりしないように)
static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 部品を入れる作業を、他と重ならないようにして、進み具合を数えながら行う
pub async fn with_progress<T>(work: impl std::future::Future<Output = T>) -> T {
    let _only_one = INSTALL_LOCK.lock().await;
    FETCH_DONE.store(0, Ordering::Relaxed);
    FETCH_TOTAL.store(0, Ordering::Relaxed);
    SHOW_PROGRESS.scope((), work).await
}

/// 部品の表示名(画面に出す名前)
pub fn display_name(name: &str) -> &'static str {
    match name {
        "yt-dlp" => "yt-dlp",
        "ffmpeg" => "FFmpeg",
        "deno" => "Deno",
        "gallery-dl" => "gallery-dl",
        _ => "?",
    }
}

async fn fetch(url: &str) -> Result<Vec<u8>> {
    let url = &crate::net::via_mirror(url);
    crate::stats::connection(url, "部品の取得・更新");
    let response = crate::net::client(Duration::from_secs(600), "Grabbit")
        .map_err(anyhow::Error::msg)?
        .get(url)
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("ダウンロード失敗: {url}"))?;
    let mut response = response;
    // 照合用の小さなファイルは数えない(大きな本体の%が一瞬 100% に飛ばないように)
    let counted = SHOW_PROGRESS.try_with(|_| ()).is_ok() && !url.ends_with("SUMS") && !url.contains(".sha256") && !url.contains("api.github.com");
    if counted {
        FETCH_DONE.store(0, Ordering::Relaxed);
        FETCH_TOTAL.store(response.content_length().unwrap_or(0), Ordering::Relaxed);
    }
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        data.extend_from_slice(&chunk);
        if counted {
            FETCH_DONE.store(data.len() as u64, Ordering::Relaxed);
        }
    }
    Ok(data)
}

/// アーカイブの中から、名前が一致するファイルだけを bin フォルダへ取り出す。
fn extract_zip(data: &[u8], wanted: &[&str]) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if let Some(name) = matching_name(&entry.name()?, wanted) {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            write_binary(&bin_dir().join(name), &buf)?;
        }
    }
    Ok(())
}

/// zip を丸ごと指定フォルダへ展開する。
fn extract_zip_all(data: &[u8], dest: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        // "../" などで外へ書き出す不正なパスは無視する
        let Some(relative) = entry.enclosed_name() else { continue };
        let path = dest.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        std::fs::write(&path, &buf)?;
        make_executable(&path);
    }
    Ok(())
}

fn extract_tar_xz(data: &[u8], wanted: &[&str]) -> Result<()> {
    let mut archive = tar::Archive::new(xz2::read::XzDecoder::new(data));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().into_owned();
        if let Some(name) = matching_name(&path, wanted) {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            write_binary(&bin_dir().join(name), &buf)?;
        }
    }
    Ok(())
}

fn matching_name<'a>(path: &str, wanted: &[&'a str]) -> Option<&'a str> {
    let file_name = path.rsplit(['/', '\\']).next()?;
    wanted.iter().copied().find(|w| *w == file_name)
}

fn write_binary(dest: &Path, data: &[u8]) -> Result<()> {
    let tmp = dest.with_extension("part");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, dest)?;
    make_executable(dest);
    Ok(())
}

fn make_executable(_path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(_path, std::fs::Permissions::from_mode(0o755));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_falls_back_to_nightly() {
        assert_eq!(normalize_channel("stable"), "stable");
        assert_eq!(normalize_channel("master"), "master");
        assert_eq!(normalize_channel("nightly"), "nightly");
        assert_eq!(normalize_channel("unknown"), "nightly");
    }

    #[test]
    fn repo_follows_channel() {
        assert_eq!(release_repo("stable"), "yt-dlp/yt-dlp");
        assert_eq!(release_repo("nightly"), "yt-dlp/yt-dlp-nightly-builds");
        assert_eq!(release_repo("master"), "yt-dlp/yt-dlp-master-builds");
    }

    /// 実際に配布元から取得して動くか(ネットワークが必要なので通常は実行しない)
    #[tokio::test]
    #[ignore]
    async fn installs_and_runs_real_ytdlp() {
        install_ytdlp("nightly").await.unwrap();
        assert!(!ytdlp_version().await.is_empty());
        assert!(supported_sites().await.unwrap().lines().count() > 1000);
        // 2回目はハッシュが同じなのでダウンロードしない
        install_ytdlp("nightly").await.unwrap();
    }

    #[test]
    fn sha256_matches_known_value() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn archive_entries_match_by_file_name() {
        let wanted = ["ffmpeg.exe", "ffprobe.exe"];
        assert_eq!(matching_name("ffmpeg-master/bin/ffmpeg.exe", &wanted), Some("ffmpeg.exe"));
        assert_eq!(matching_name("dir\\ffprobe.exe", &wanted), Some("ffprobe.exe"));
        assert_eq!(matching_name("bin/ffplay.exe", &wanted), None);
    }
}

#[cfg(test)]
mod repo_tests {
    use super::is_valid_repo;

    #[test]
    fn only_owner_slash_repo_is_accepted() {
        assert!(is_valid_repo("yt-dlp/yt-dlp-nightly-builds"));
        assert!(is_valid_repo("someone/my.builds_2"));
        for bad in ["", "yt-dlp", "a/b/c", "../x", "a/..", "https://github.com/a/b", "a b/c", "a/b?x=1", "/b", "a/"] {
            assert!(!is_valid_repo(bad), "{bad}");
        }
    }
}

#[cfg(test)]
mod progress_tests {
    #[tokio::test]
    #[ignore]
    async fn progress_is_reported() {
        let url = format!("{}/big.bin", std::env::var("GRABBIT_PRACTICAL_URL").unwrap());
        let probe = tokio::spawn(async {
            let mut seen = Vec::new();
            for _ in 0..40 {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                seen.push(super::fetch_progress());
            }
            seen
        });
        let data = super::with_progress(super::fetch(&url)).await.unwrap();
        let seen = probe.await.unwrap();
        assert_eq!(data.len() as u64, seen.last().unwrap().1, "最後は全部受け取っていること");
        assert!(seen.iter().any(|&(done, total)| total > 0 && done < total), "途中の進み具合が見えること: {seen:?}");
    }
}
