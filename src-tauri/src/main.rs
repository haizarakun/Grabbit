//! Grabbit: yt-dlp を誰でも使える GUI ダウンローダー。
//! 画面(ui/)から呼ばれるコマンドと、ダウンロード待ち行列・定期確認をここで管理する。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analyze;
mod desktop;
mod history;
mod login;
mod methods;
mod net;
mod sealed;
mod stats;
mod audio;
#[cfg(test)]
mod practical;
mod download;
mod extras;
mod fallback;
mod settings;
mod tools;

use download::{Event, Job, Outcome, Status};
use settings::{load_json, save_json, Settings, Subscription};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::sync::Notify;

const SETTINGS_FILE: &str = "settings.json";

/// 1件・1登録先ごとに設定を上書きする値(キーは Settings の項目名)
type Overrides = serde_json::Map<String, serde_json::Value>;

/// ダウンロードごとの記録(失敗した時に開いて原因を見られる)
fn log_path(job_id: u64) -> std::path::PathBuf {
    let dir = settings::data_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{job_id}.log"))
}
const SUBSCRIPTIONS_FILE: &str = "subscriptions.json";

struct AppState {
    settings: Mutex<Settings>,
    subscriptions: Mutex<Vec<Subscription>>,
    jobs: Mutex<Vec<Job>>,
    next_id: AtomicU64,
    running: AtomicUsize,
    /// ツールの準備が終わったか
    ready: AtomicBool,
    /// 待ち行列を見直す合図
    queue_changed: Notify,
    history: history::History,
    /// 一時停止を頼まれて止めている途中のもの
    pausing: Mutex<std::collections::HashSet<u64>>,
    /// 表示用情報の先読みを同時に何件まで行うか
    prefetch_limit: tokio::sync::Semaphore,
}

type Shared = Arc<AppState>;

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ───────────── 画面から呼ばれるコマンド ─────────────

#[tauri::command]
fn get_settings(state: State<Shared>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(state: State<Shared>, mut settings: Settings) {
    // ログイン画面の合言葉は、専用の命令でしか変えられないようにする
    settings.login_password_hash = state.settings.lock().unwrap().login_password_hash.clone();
    apply_global_settings(&settings);
    save_json(SETTINGS_FILE, &settings);
    *state.settings.lock().unwrap() = settings;
    state.queue_changed.notify_one();
}

/// フォルダを選ぶ画面を出し、選んだフォルダの場所を返す(やめたら None)
#[tauri::command]
async fn pick_folder(app: AppHandle, start: Option<String>) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let mut dialog = app.dialog().file();
    if let Some(dir) = start.filter(|d| std::path::Path::new(d).is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    // 画面が止まらないよう、選び終わるのを別の thread で待つ
    let (tx, rx) = tokio::sync::oneshot::channel();
    dialog.pick_folder(move |folder| {
        let _ = tx.send(folder.and_then(|f| f.into_path().ok()).map(|p| p.to_string_lossy().into_owned()));
    });
    rx.await.ok().flatten()
}

/// 通信・暗号化など、アプリ全体に効く設定を反映する
fn apply_global_settings(settings: &Settings) {
    net::configure(settings);
    tools::configure(&settings.tool_repos, &settings.disabled_tools);
    sealed::set_enabled(settings.encrypt_records && cfg!(windows));
}

/// アプリ内の小さなブラウザでログインしてもらう(閉じると自動で保存)
#[tauri::command]
fn open_login(app: AppHandle, state: State<Shared>, url: String, title: Option<String>) -> Result<(), String> {
    if !login_allowed(state.inner()) {
        return Err("ログイン情報には合言葉がかかっています".into());
    }
    login::open(&app, &url, title.as_deref().unwrap_or_default())
}

#[tauri::command]
fn login_sites(state: State<Shared>) -> Vec<login::LoginSite> {
    // 合言葉がかかっている間は、どのサイトにログインしているかも見せない
    if !login_allowed(state.inner()) {
        return Vec::new();
    }
    login::logged_in_sites()
}

/// ログイン情報を消す(site が空なら全部)
#[tauri::command]
fn forget_login(state: State<Shared>, site: String) {
    if !login_allowed(state.inner()) {
        return;
    }
    login::forget(&site);
}

/// ライブラリの動画・音声の一部を切り抜いて別ファイルにする。できたものはライブラリにも載せる。
#[tauri::command]
async fn trim_media(app: AppHandle, state: State<'_, Shared>, id: u64, start: String, end: String, exact: bool) -> Result<String, String> {
    let entry = state.history.list().into_iter().find(|e| e.id == id).ok_or("ライブラリに見つかりません")?;
    let start = extras::parse_time(&start).ok_or("始まりの時間が読めません(例: 1:23)")?;
    let end = extras::parse_time(&end).ok_or("終わりの時間が読めません(例: 2:45)")?;
    let output = extras::trim(std::path::Path::new(&entry.file_path), start, end, exact).await.map_err(|e| e.to_string())?;
    let path = output.to_string_lossy().into_owned();
    state.history.add(history::Entry {
        id: state.next_id.fetch_add(1, Ordering::SeqCst),
        title: format!("{} ✂", entry.title),
        file_path: path.clone(),
        saved_at: now_secs(),
        size: 0,
        exists: true,
        ..entry
    });
    let _ = app.emit("history_changed", ());
    Ok(path)
}

/// ライブラリのファイル名をまとめて付け直す。dry_run なら変えずに、新しい名前の一覧だけ返す。
/// 返す値: [ライブラリの id, 今の名前, 新しい名前]
#[tauri::command]
fn rename_library(app: AppHandle, state: State<Shared>, ids: Vec<u64>, pattern: String, dry_run: bool) -> Result<Vec<(u64, String, String)>, String> {
    if pattern.trim().is_empty() {
        return Err("名前のひな形が空です".into());
    }
    let entries = state.history.list();
    let targets: Vec<_> = ids.iter().filter_map(|id| entries.iter().find(|e| e.id == *id && e.exists)).collect();
    let digits = targets.len().to_string().len().max(2);
    let mut results = Vec::new();
    let mut errors = Vec::new();
    for (i, entry) in targets.iter().enumerate() {
        let path = std::path::Path::new(&entry.file_path);
        let old = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = extras::rename_pattern(&pattern, entry, i + 1, digits);
        if dry_run {
            let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            results.push((entry.id, old, format!("{stem}{ext}")));
            continue;
        }
        match extras::rename_file(path, &stem) {
            Ok(new_path) => {
                state.history.set_path(entry.id, &new_path.to_string_lossy());
                let new = new_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                results.push((entry.id, old, new));
            }
            Err(e) => errors.push(format!("{old}: {e}")),
        }
    }
    if !dry_run {
        let _ = app.emit("history_changed", ());
    }
    if errors.is_empty() { Ok(results) } else { Err(errors.join("\n")) }
}

/// 保存する前に、保存先に同じ名前のファイルがもう無いか調べる。あればその場所を返す。
/// (yt-dlp にファイル名だけを聞くので、ダウンロードはしない)
#[tauri::command]
async fn check_existing_file(state: State<'_, Shared>, url: String, overrides: Option<Overrides>) -> Result<Option<String>, String> {
    let overrides = overrides.unwrap_or_default();
    let mut settings = state.settings.lock().unwrap().with_overrides(&overrides);
    if !overrides.contains_key("output_dir") {
        settings.route_output_dir(&url);
    }
    Ok(find_same_name(&settings, &url).await.map(|p| p.to_string_lossy().into_owned()))
}

/// 保存した時のファイル名を yt-dlp に聞き、保存先に同じ名前があればその場所を返す
async fn find_same_name(settings: &Settings, url: &str) -> Option<std::path::PathBuf> {
    let mut cmd = download::command(&tools::ytdlp_path());
    cmd.args(download::predict_filename_args(settings, url));
    let output = tokio::time::timeout(Duration::from_secs(40), cmd.output()).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let predicted = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    download::existing_same_name(std::path::Path::new(predicted))
}

/// 登録したプロキシがつながるか試す。つながれば、かかった時間(ミリ秒)を返す
#[tauri::command]
async fn test_proxy(proxy: String) -> Result<u64, String> {
    let proxy = proxy.trim();
    if proxy.is_empty() {
        return Err("プロキシが入っていません".into());
    }
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy).map_err(|_| "プロキシの書き方が正しくありません(例: http://127.0.0.1:8080)")?)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let started = std::time::Instant::now();
    client
        .get("https://www.youtube.com/generate_204")
        .send()
        .await
        .map_err(|e| format!("つながりませんでした: {e}"))?;
    Ok(started.elapsed().as_millis() as u64)
}

/// 接続の診断。回線・プロキシ・サイト・部品のどこに問題があるかを順に調べる。
/// 返す値: [項目, 結果(ok / warn / bad), 説明]
#[tauri::command]
async fn diagnose(state: State<'_, Shared>, url: Option<String>) -> Result<Vec<(String, String, String)>, String> {
    let settings = state.settings.lock().unwrap().clone();
    let mut results: Vec<(String, String, String)> = Vec::new();
    let mut add = |name: &str, status: &str, detail: String| results.push((name.into(), status.into(), detail));

    // 1つの URL に、指定のプロキシ(空なら直接)でつないで、かかった時間を測る
    async fn probe(target: &str, proxy: &str) -> Result<(u16, u64), String> {
        let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(15)).user_agent("Mozilla/5.0 Grabbit");
        if !proxy.is_empty() {
            builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| "プロキシの書き方が正しくありません".to_string())?);
        }
        let client = builder.build().map_err(|e| e.to_string())?;
        let started = std::time::Instant::now();
        let response = client.get(target).send().await.map_err(|e| e.to_string())?;
        Ok((response.status().as_u16(), started.elapsed().as_millis() as u64))
    }

    // インターネット(プロキシを通さず。「プロキシだけを通す」の時は直接つながないので飛ばす)
    if settings.proxy_only {
        add("インターネット", "warn", "「プロキシだけを通す」がオンのため、直接の接続は調べませんでした".into());
    } else { match probe("https://www.google.com/generate_204", "").await {
        Ok((_, ms)) => add("インターネット", "ok", format!("つながっています({ms} ミリ秒)")),
        Err(e) => add("インターネット", "bad", format!("つながりません。回線・Wi-Fi・セキュリティソフトを確認してください({e})")),
    } }
    // プロキシ
    for (name, proxy) in [("プロキシ", &settings.proxy), ("地域制限用のプロキシ", &settings.fallback_proxy)] {
        if proxy.trim().is_empty() {
            continue;
        }
        match probe("https://www.youtube.com/generate_204", proxy.trim()).await {
            Ok((_, ms)) => add(name, "ok", format!("つながっています({ms} ミリ秒)")),
            Err(e) => add(name, "bad", format!("つながりません。プロキシのアドレスと、プロキシが動いているかを確認してください({e})")),
        }
    }
    // 調べたいサイト
    if let Some(url) = url.filter(|u| u.starts_with("https://") || u.starts_with("http://")) {
        let host = url.split('/').nth(2).unwrap_or_default().to_string();
        match probe(&url, settings.proxy.trim()).await {
            Ok((code, ms)) if code < 400 => add("サイト", "ok", format!("{host} に届きます({ms} ミリ秒)")),
            Ok((403 | 429, _)) => add("サイト", "warn", format!("{host} に拒否されています(使いすぎ・ロボット判定)。時間をおくか、ログインを使ってください")),
            Ok((code, _)) => add("サイト", "warn", format!("{host} が {code} を返しました(ページが無い・見られない可能性)")),
            Err(e) => add("サイト", "bad", format!("{host} に届きません({e})")),
        }
    }
    // 速度(約 10MB を落としてみる)
    let started = std::time::Instant::now();
    let speed = async {
        let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(30));
        if !settings.proxy.trim().is_empty() {
            builder = builder.proxy(reqwest::Proxy::all(settings.proxy.trim()).ok()?);
        }
        let bytes = builder.build().ok()?.get("https://speed.cloudflare.com/__down?bytes=10000000").send().await.ok()?.bytes().await.ok()?;
        Some(bytes.len())
    }
    .await;
    match speed {
        Some(bytes) => {
            let mbps = bytes as f64 * 8.0 / started.elapsed().as_secs_f64().max(0.001) / 1_000_000.0;
            let status = if mbps < 5.0 { "warn" } else { "ok" };
            add("回線の速さ", status, format!("約 {mbps:.0} Mbps"));
        }
        None => add("回線の速さ", "warn", "測れませんでした".into()),
    }
    // 部品
    let version = tools::ytdlp_version().await;
    if version.is_empty() {
        add("ダウンロード機能", "bad", "動きません。設定の「部品を点検・修復」を押してください".into());
    } else {
        add("ダウンロード機能", "ok", format!("動いています({version})"));
    }
    let ffmpeg = tools::bin_dir().join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
    if ffmpeg.exists() {
        add("変換機能(FFmpeg)", "ok", "入っています".into());
    } else {
        add("変換機能(FFmpeg)", "bad", "見つかりません。設定の「部品を点検・修復」を押してください".into());
    }
    // 保存先
    match extras::free_space(&settings.output_dir) {
        Some(free) if free < 2 * 1024 * 1024 * 1024 => add("保存先の空き", "warn", format!("残り {:.1} GB です", free as f64 / 1e9)),
        Some(free) => add("保存先の空き", "ok", format!("残り {:.0} GB", free as f64 / 1e9)),
        None => add("保存先の空き", "bad", "保存先が見つかりません".into()),
    }
    Ok(results)
}

/// ライブラリの動画・音声をまとめて検査する。進み具合は "verify_progress" で送る。
/// 返す値: [壊れていたものの id, 理由]
#[tauri::command]
async fn verify_library(app: AppHandle, state: State<'_, Shared>, ids: Vec<u64>) -> Result<Vec<(u64, String)>, String> {
    let targets: Vec<_> = state.history.list().into_iter()
        .filter(|e| ids.contains(&e.id) && e.exists && matches!(e.kind.as_str(), "video" | "audio"))
        .collect();
    let mut broken = Vec::new();
    for (i, entry) in targets.iter().enumerate() {
        let _ = app.emit("verify_progress", (i + 1, targets.len(), entry.title.clone()));
        if let Err(problem) = extras::verify_media(std::path::Path::new(&entry.file_path)).await {
            broken.push((entry.id, problem));
        }
    }
    Ok(broken)
}

/// ライブラリの動画から音声だけを取り出す / 音声から波形動画を作る。できたものはライブラリにも載せる
#[tauri::command]
async fn convert_media(app: AppHandle, state: State<'_, Shared>, id: u64, kind: String) -> Result<String, String> {
    let entry = state.history.list().into_iter().find(|e| e.id == id).ok_or("ライブラリに見つかりません")?;
    let source = std::path::Path::new(&entry.file_path);
    let (output, new_kind, mark) = match kind.as_str() {
        "audio" => (extras::extract_audio(source).await, "audio", "♪"),
        "waveform" => (extras::waveform_video(source).await, "video", "〰"),
        "karaoke" => (audio::vocal_version(source, false).await, "audio", "🎤"),
        "vocals" => (audio::vocal_version(source, true).await, "audio", "🗣"),
        _ => return Err("不明な操作です".into()),
    };
    let path = output.map_err(|e| e.to_string())?.to_string_lossy().into_owned();
    state.history.add(history::Entry {
        id: state.next_id.fetch_add(1, Ordering::SeqCst),
        title: format!("{} {mark}", entry.title),
        file_path: path.clone(),
        kind: new_kind.into(),
        saved_at: now_secs(),
        size: 0,
        exists: true,
        ..entry
    });
    let _ = app.emit("history_changed", ());
    Ok(path)
}

/// 保存したものの元ページが、今も見られるか調べる。返す値: [見られなくなった id, 理由]
#[tauri::command]
async fn check_sources(app: AppHandle, state: State<'_, Shared>, ids: Vec<u64>) -> Result<Vec<(u64, String)>, String> {
    let settings = state.settings.lock().unwrap().clone();
    let targets: Vec<_> = state.history.list().into_iter().filter(|e| ids.contains(&e.id) && e.url.starts_with("http")).collect();
    let mut gone = Vec::new();
    for (i, entry) in targets.iter().enumerate() {
        let _ = app.emit("source_progress", (i + 1, targets.len(), entry.title.clone()));
        let mut cmd = download::command(&tools::ytdlp_path());
        cmd.args(["--simulate", "--no-warnings", "--no-playlist", "--flat-playlist", "-O", "id"])
            .args(download::without_browser_cookies(&download::network_args(&settings)))
            .args(["--", &entry.url]);
        match tokio::time::timeout(Duration::from_secs(45), cmd.output()).await {
            Ok(Ok(output)) if output.status.success() => {}
            Ok(Ok(output)) => {
                let reason = String::from_utf8_lossy(&output.stderr).lines().rev().find(|l| l.contains("ERROR")).unwrap_or("ERROR").to_string();
                // 回線のエラーは「消えた」とは言わない
                if !download::is_network_error(&reason) {
                    gone.push((entry.id, reason));
                }
            }
            _ => {}
        }
    }
    Ok(gone)
}

/// ダウンロード機能の最近の変更点
#[tauri::command]
async fn ytdlp_changelog(state: State<'_, Shared>) -> Result<Vec<(String, String, String)>, String> {
    let channel = state.settings.lock().unwrap().update_channel.clone();
    tools::ytdlp_changelog(&channel).await.map_err(|e| e.to_string())
}

/// ダウンロード機能を1つ前の版に戻す(戻した後は自動で更新しないようにする)
#[tauri::command]
async fn rollback_ytdlp(state: State<'_, Shared>) -> Result<String, String> {
    if state.running.load(Ordering::SeqCst) > 0 {
        return Err("ダウンロード中は戻せません。終わってから試してください".into());
    }
    tools::rollback_ytdlp().map_err(|e| e.to_string())?;
    {
        let mut settings = state.settings.lock().unwrap();
        settings.pin_ytdlp = true;
        save_json(SETTINGS_FILE, &*settings);
    }
    Ok(tools::ytdlp_version().await)
}

/// 不具合の報告に必要な情報を1つの文章にまとめる(パスワードやログイン情報は入れない)
#[tauri::command]
async fn bug_report(app: AppHandle, state: State<'_, Shared>) -> Result<String, String> {
    let mut settings = state.settings.lock().unwrap().clone();
    // 人に見せる文章なので、個人の情報になりそうなものは伏せる
    for value in [&mut settings.proxy, &mut settings.fallback_proxy, &mut settings.cookies_file, &mut settings.ntfy_topic, &mut settings.user_agent, &mut settings.login_password_hash, &mut settings.github_mirror] {
        if !value.is_empty() {
            *value = "***".into();
        }
    }
    settings.output_dir = "***".into();
    settings.temp_dir = if settings.temp_dir.is_empty() { String::new() } else { "***".into() };
    settings.ui = serde_json::Map::new();
    let failed: Vec<Job> = state.jobs.lock().unwrap().iter().filter(|j| j.status == Status::Failed).rev().take(5).cloned().collect();
    let mut text = format!(
        "## Grabbit {}\nOS: {} {}\nダウンロード機能: {}\n\n## 設定\n```json\n{}\n```\n",
        app.package_info().version,
        std::env::consts::OS,
        std::env::consts::ARCH,
        tools::ytdlp_version().await,
        serde_json::to_string_pretty(&settings).unwrap_or_default(),
    );
    for job in failed {
        let log = std::fs::read(log_path(job.id)).unwrap_or_default();
        let tail: Vec<String> = String::from_utf8_lossy(&log).lines().rev().take(25).map(String::from).collect();
        let tail: Vec<String> = download::redact_args(&tail.into_iter().rev().collect::<Vec<_>>());
        text += &format!("\n## 失敗: {}\n{}\n```\n{}\n```\n", job.url, job.message, tail.join("\n"));
    }
    // ユーザー名入りのフォルダ・メールアドレスなどを消してから渡す
    Ok(extras::scrub_private(&text))
}

/// 動作の重さ: Grabbit と、動いているダウンロード部品の CPU・メモリ
#[tauri::command]
async fn performance_stats(state: State<'_, Shared>) -> Result<serde_json::Value, String> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_cpu().with_memory());
    tokio::time::sleep(Duration::from_millis(500)).await; // CPU は2回測った差で出る
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_cpu().with_memory());
    let me = sysinfo::get_current_pid().map_err(|e| e.to_string())?;
    // Grabbit 本体と、その子・孫(yt-dlp・ffmpeg・画面の WebView など)を集める
    let mut family = vec![me];
    let mut rows = Vec::new();
    let mut changed = true;
    while changed {
        changed = false;
        for (pid, process) in system.processes() {
            if !family.contains(pid) && process.parent().is_some_and(|p| family.contains(&p)) {
                family.push(*pid);
                changed = true;
            }
        }
    }
    for pid in &family {
        if let Some(p) = system.process(*pid) {
            rows.push(serde_json::json!({
                "name": p.name().to_string_lossy(),
                "cpu": (p.cpu_usage() * 10.0).round() / 10.0,
                "memory_mb": p.memory() / 1_048_576,
            }));
        }
    }
    let running = state.running.load(Ordering::SeqCst);
    let waiting = state.jobs.lock().unwrap().iter().filter(|j| j.status == Status::Waiting).count();
    Ok(serde_json::json!({ "processes": rows, "running": running, "waiting": waiting, "cpus": num_cpus() }))
}

fn num_cpus() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

/// スマホへの通知を試しに送る
#[tauri::command]
async fn ntfy_test(server: String, topic: String) -> Result<(), String> {
    extras::ntfy_send(&server, topic.trim(), "Grabbit", "テスト通知です / Test notification").await.map_err(|e| e.to_string())
}

/// 一覧から消したものを戻す(元に戻す用)
#[tauri::command]
fn restore_jobs(app: AppHandle, state: State<Shared>, jobs: Vec<Job>) {
    let mut list = state.jobs.lock().unwrap();
    for mut job in jobs {
        if list.iter().any(|j| j.id == job.id) {
            continue;
        }
        // 動いていたものは戻した時に「予定」にする(もう動いていないので)
        if job.status == Status::Running {
            job.status = Status::Waiting;
        }
        let _ = app.emit("job", job.clone());
        list.push(job);
    }
    list.sort_by_key(|j| j.id);
    state.queue_changed.notify_one();
}

#[tauri::command]
fn restore_history(app: AppHandle, state: State<Shared>, entries: Vec<history::Entry>) {
    state.history.restore(entries);
    let _ = app.emit("history_changed", ());
}

/// ごみ箱へ移したファイルを元の場所へ戻す(Windows・Linux)
#[tauri::command]
fn restore_from_trash(app: AppHandle, state: State<Shared>, id: u64) -> Result<(), String> {
    let entry = state.history.list().into_iter().find(|e| e.id == id).ok_or("ライブラリに見つかりません")?;
    #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
    {
        let target = std::path::PathBuf::from(&entry.file_path);
        let items: Vec<_> = trash::os_limited::list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|item| item.original_path() == target)
            .collect();
        if items.is_empty() {
            return Err("ごみ箱に見つかりません(もう空にした可能性があります)".into());
        }
        trash::os_limited::restore_all(items).map_err(|e| e.to_string())?;
        state.history.set_trashed(id, false);
        let _ = app.emit("history_changed", ());
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (&app, &entry);
        Err("Mac ではごみ箱から直接戻してください(Finder のごみ箱 → 戻す)".into())
    }
}

/// ライブラリのファイルを別のフォルダへ移し、記録も書き換える
#[tauri::command]
async fn move_library(app: AppHandle, state: State<'_, Shared>, ids: Vec<u64>, dest: String) -> Result<usize, String> {
    let dest = std::path::PathBuf::from(dest.trim());
    if !dest.is_absolute() {
        return Err("移す先は「D:\\動画」のようにフォルダの場所をそのまま入れてください".into());
    }
    std::fs::create_dir_all(&dest).map_err(|e| format!("フォルダを作れません: {e}"))?;
    let targets: Vec<_> = state.history.list().into_iter().filter(|e| ids.contains(&e.id) && e.exists).collect();
    let mut moved = 0;
    let mut errors = Vec::new();
    for (i, entry) in targets.iter().enumerate() {
        let _ = app.emit("move_progress", (i + 1, targets.len(), entry.title.clone()));
        let source = std::path::PathBuf::from(&entry.file_path);
        let Some(name) = source.file_name() else { continue };
        let mut target = dest.join(name);
        let mut n = 2;
        while target.exists() {
            let stem = source.file_stem().unwrap_or_default().to_string_lossy();
            let ext = source.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            target = dest.join(format!("{stem} ({n}){ext}"));
            n += 1;
        }
        // 同じドライブなら名前の付け替えだけ。別のドライブなら写してから元を消す
        let result = match std::fs::rename(&source, &target) {
            Ok(()) => Ok(()),
            Err(_) => tokio::fs::copy(&source, &target).await.map(|_| ()).and_then(|_| std::fs::remove_file(&source)),
        };
        match result {
            Ok(()) => {
                state.history.set_path(entry.id, &target.to_string_lossy());
                moved += 1;
            }
            Err(e) => errors.push(format!("{}: {e}", entry.title)),
        }
    }
    let _ = app.emit("history_changed", ());
    if errors.is_empty() { Ok(moved) } else { Err(errors.join("\n")) }
}

/// タスクバーのアイコンに進み具合を出す(percent が無ければ消す)
#[tauri::command]
fn set_taskbar_progress(app: AppHandle, percent: Option<f64>, paused: bool) {
    use tauri::window::{ProgressBarState, ProgressBarStatus};
    if let Some(window) = app.get_webview_window("main") {
        let state = match percent {
            Some(p) => ProgressBarState {
                status: Some(if paused { ProgressBarStatus::Paused } else { ProgressBarStatus::Normal }),
                progress: Some(p.clamp(0.0, 100.0) as u64),
            },
            None => ProgressBarState { status: Some(ProgressBarStatus::None), progress: None },
        };
        let _ = window.set_progress_bar(state);
    }
}

/// 外への通信の一覧
#[tauri::command]
fn list_connections() -> Vec<stats::Connection> {
    stats::connections()
}

/// 失敗の統計
#[tauri::command]
fn error_stats() -> serde_json::Value {
    stats::summary()
}

#[tauri::command]
fn clear_error_stats() {
    stats::clear_failures();
}

/// 起動時に長く使っていなかったログイン情報を消したか
static LOGIN_EXPIRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 起動時のお知らせ: 設定ファイルを控えから戻した / 使っていないログイン情報を消した
#[tauri::command]
fn startup_notices() -> serde_json::Value {
    serde_json::json!({
        "settings_restored": settings::RESTORED_FROM_BACKUP.swap(false, Ordering::SeqCst),
        "login_expired": LOGIN_EXPIRED.swap(false, Ordering::SeqCst),
    })
}

/// Grabbit のこの版の変更点(GitHub のリリースの説明)
#[tauri::command]
async fn app_release_notes(app: AppHandle) -> Result<String, String> {
    let version = app.package_info().version.to_string();
    let url = format!("https://api.github.com/repos/haizarakun/Grabbit/releases/tags/v{version}");
    stats::connection(&url, "Grabbit の変更点");
    let client = net::client(Duration::from_secs(15), "Grabbit")?;
    let body = client.get(&url).send().await.map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    json["body"].as_str().map(|s| s.chars().take(6000).collect()).ok_or_else(|| "変更点が見つかりません".into())
}

/// 失敗したものの URL をファイルに書き出す(後でまとめて読み込んでやり直す用)
#[tauri::command]
fn export_failed(state: State<Shared>) -> Result<String, String> {
    let urls: Vec<String> = state.jobs.lock().unwrap().iter().filter(|j| matches!(j.status, Status::Failed | Status::Canceled)).map(|j| j.url.clone()).collect();
    if urls.is_empty() {
        return Err("失敗したものはありません".into());
    }
    let dir = dirs::download_dir().or_else(dirs::desktop_dir).ok_or("保存先が見つかりません")?;
    let path = dir.join(format!("Grabbit-failed-{}.txt", now_secs()));
    std::fs::write(&path, urls.join("\n") + "\n").map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// 個人の情報を消した文章にする(ログを人に見せる時用)
#[tauri::command]
fn scrub_text(text: String) -> String {
    extras::scrub_private(&text)
}

// ───────────── ログイン画面の合言葉 ─────────────
// 合言葉はそのまま保存せず、塩(ランダムな値)を混ぜたハッシュだけを保存する

static LOGIN_UNLOCKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn password_hash(password: &str, salt: &str) -> String {
    use sha2::{Digest, Sha256};
    // 何度も重ねてハッシュし、総当たりで当てにくくする
    let mut digest = Sha256::digest(format!("{salt}:{password}").as_bytes()).to_vec();
    for _ in 0..100_000 {
        digest = Sha256::digest(&digest).to_vec();
    }
    format!("{salt}${}", digest.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

fn password_matches(password: &str, stored: &str) -> bool {
    let Some((salt, _)) = stored.split_once('$') else { return false };
    password_hash(password, salt) == stored
}

/// ログイン画面が開けるか(合言葉が無いか、この起動中に一度正しく入れたか)
fn login_allowed(state: &Shared) -> bool {
    state.settings.lock().unwrap().login_password_hash.is_empty() || LOGIN_UNLOCKED.load(Ordering::SeqCst)
}

#[tauri::command]
fn login_locked(state: State<Shared>) -> bool {
    !login_allowed(state.inner())
}

#[tauri::command]
fn unlock_login(state: State<Shared>, password: String) -> bool {
    let stored = state.settings.lock().unwrap().login_password_hash.clone();
    let ok = stored.is_empty() || password_matches(&password, &stored);
    if ok {
        LOGIN_UNLOCKED.store(true, Ordering::SeqCst);
    } else {
        std::thread::sleep(Duration::from_millis(800)); // 続けて試されにくくする
    }
    ok
}

/// 合言葉を決める・変える・外す(new が空なら外す)。今の合言葉が要る
#[tauri::command]
fn set_login_password(state: State<Shared>, current: String, new: String) -> Result<(), String> {
    let mut settings = state.settings.lock().unwrap();
    if !settings.login_password_hash.is_empty() && !password_matches(&current, &settings.login_password_hash) {
        std::thread::sleep(Duration::from_millis(800));
        return Err("今の合言葉が違います".into());
    }
    settings.login_password_hash = if new.is_empty() {
        String::new()
    } else {
        if new.chars().count() < 4 {
            return Err("合言葉は 4 文字以上にしてください".into());
        }
        let mut salt = [0u8; 12];
        getrandom::getrandom(&mut salt).map_err(|e| e.to_string())?;
        password_hash(&new, &salt.iter().map(|b| format!("{b:02x}")).collect::<String>())
    };
    save_json(SETTINGS_FILE, &*settings);
    LOGIN_UNLOCKED.store(true, Ordering::SeqCst);
    Ok(())
}

/// サイトごとに覚えた取り方の一覧
#[tauri::command]
fn list_site_methods() -> std::collections::BTreeMap<String, methods::Method> {
    methods::list()
}

/// 覚えた取り方を消す(空なら全部)
#[tauri::command]
fn forget_site_method(site: String) {
    methods::forget(&site);
}

/// ダウンロードが全部終わった後の操作(画面側で取り消しの猶予を出してから呼ぶ)
#[tauri::command]
fn power_action(app: AppHandle, kind: String) -> Result<(), String> {
    if kind == "quit" {
        app.exit(0);
        return Ok(());
    }
    extras::power_action(&kind).map_err(|e| e.to_string())
}

/// 設定・保存プリセット・自動保存の登録を1つの JSON にまとめる(別の PC へ移す用)。
/// ログイン情報は入れない(ファイルを人に渡した時に漏れないように)。
#[tauri::command]
fn export_settings(state: State<Shared>) -> Result<String, String> {
    let mut settings = state.settings.lock().unwrap().clone();
    settings.cookies_file.clear();
    settings.login_password_hash.clear();
    let data = serde_json::json!({
        "grabbit_export": 1,
        "settings": settings,
        "subscriptions": *state.subscriptions.lock().unwrap(),
    });
    let dir = dirs::download_dir().or_else(dirs::desktop_dir).ok_or("保存先が見つかりません")?;
    let path = dir.join(format!("Grabbit-settings-{}.json", now_secs()));
    std::fs::write(&path, serde_json::to_vec_pretty(&data).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// 書き出した JSON を読み込む。自動保存は同じ URL が無いものだけ足す。
#[tauri::command]
fn import_settings(app: AppHandle, state: State<Shared>, text: String) -> Result<Settings, String> {
    let data: serde_json::Value = serde_json::from_str(&text).map_err(|_| "Grabbit の設定ファイルではありません")?;
    if data["grabbit_export"].as_u64() != Some(1) {
        return Err("Grabbit の設定ファイルではありません".into());
    }
    let mut imported: Settings = serde_json::from_value(data["settings"].clone()).map_err(|e| e.to_string())?;
    // 追加オプションはコマンドを実行できるので、人から受け取ったファイルでは引き継がない
    imported.extra_args.clear();
    // 合言葉は今のものを使う(知らない合言葉で閉め出されないように)
    imported.login_password_hash = state.settings.lock().unwrap().login_password_hash.clone();
    apply_global_settings(&imported);
    if !std::path::Path::new(&imported.output_dir).is_absolute() {
        imported.output_dir = state.settings.lock().unwrap().output_dir.clone();
    }
    let subs: Vec<Subscription> = serde_json::from_value(data["subscriptions"].clone()).unwrap_or_default();
    let mut added = Vec::new();
    update_subscriptions(&app, &state, |current| {
        for mut sub in subs {
            let valid = sub.url.starts_with("https://") || sub.url.starts_with("http://");
            if valid && !current.iter().any(|s| s.url == sub.url) {
                sub.id = state.next_id.fetch_add(1, Ordering::SeqCst);
                sub.overrides.remove("extra_args");
                // この PC にはまだ「保存済み」の記録が無いので、既存の動画を記録し終えるまで止めておく
                // (そのままだと過去の動画を全部落としてしまう)
                sub.enabled = false;
                added.push((sub.id, sub.url.clone()));
                current.push(sub);
            }
        }
    });
    let shared = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        for (id, url) in added {
            if scan_existing(&url, true).await.is_ok() {
                update_subscriptions(&app, &shared, |subs| {
                    if let Some(s) = subs.iter_mut().find(|s| s.id == id) {
                        s.enabled = true;
                        s.last_checked = now_secs();
                    }
                });
            }
        }
    });
    save_json(SETTINGS_FILE, &imported);
    *state.settings.lock().unwrap() = imported.clone();
    state.queue_changed.notify_one();
    Ok(imported)
}

#[tauri::command]
fn reset_settings(state: State<Shared>) -> Settings {
    let defaults = Settings::default();
    save_settings(state, defaults.clone());
    defaults
}

/// 起動した時に渡された URL(「送る」メニューなど)。画面の準備ができてから一度だけ受け取る。
#[tauri::command]
fn startup_urls() -> String {
    extras::urls_from_args(&std::env::args().collect::<Vec<_>>()).join("\n")
}

#[tauri::command]
fn is_ready(state: State<Shared>) -> bool {
    state.ready.load(Ordering::SeqCst)
}

/// URL を解析して、タイトル・サムネイル・選べる形式を返す。
#[tauri::command]
async fn analyze_url(
    state: State<'_, Shared>,
    url: String,
    overrides: Option<Overrides>,
) -> Result<analyze::MediaInfo, String> {
    // 地域や言語を変えて調べ直す時は、その指定で上書きする
    let settings = state.settings.lock().unwrap().with_overrides(&overrides.unwrap_or_default());
    if net::blocked_without_proxy() {
        return Err("「プロキシだけを通す」がオンですが、プロキシが入っていません".into());
    }
    analyze::analyze(&settings, url.trim()).await.map_err(|e| e.to_string())
}

/// ページの見出し情報だけを読んで、タイトルとサムネイルをすぐ返す(本格的な解析より先に出す用)。
#[tauri::command]
async fn quick_preview(url: String) -> Option<analyze::QuickInfo> {
    analyze::quick_preview(url.trim()).await
}

/// キーワードで動画を探す。
#[tauri::command]
async fn search_videos(
    state: State<'_, Shared>,
    query: String,
    service: String,
) -> Result<Vec<analyze::SearchResult>, String> {
    let settings = state.settings.lock().unwrap().clone();
    analyze::search(&settings, query.trim(), &service, 24).await.map_err(|e| e.to_string())
}

/// 言語ごとに違うサムネイルを探す。
#[tauri::command]
async fn localized_thumbnails(
    state: State<'_, Shared>,
    url: String,
    langs: Vec<String>,
) -> Result<Vec<analyze::LocalizedThumbnail>, String> {
    let settings = state.settings.lock().unwrap().clone();
    Ok(analyze::localized_thumbnails(&settings, &url, &langs).await)
}

/// URL を待ち行列に追加する。改行・空白区切りで何件でもまとめて渡せる。
/// 同じ URL の重複や、すでに待機中・ダウンロード中のものは追加しない。
/// `format` / `overrides` はプレビューで選んだ形式と保存方法。`title` / `thumbnail` は表示用。
#[tauri::command]
fn add_downloads(
    app: AppHandle,
    state: State<Shared>,
    text: String,
    format: Option<String>,
    overrides: Option<Overrides>,
    title: Option<String>,
    thumbnail: Option<String>,
    start_at: Option<u64>,
) -> AddResult {
    let overrides = overrides.unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let active: std::collections::HashSet<String> = state
        .jobs
        .lock()
        .unwrap()
        .iter()
        .filter(|j| matches!(j.status, Status::Waiting | Status::Running) && j.overrides == overrides)
        .map(|j| j.url.clone())
        .collect();

    let mut result = AddResult::default();
    for url in download::extract_urls(&text) {
        let url = url.as_str();
        if !seen.insert(url.to_string()) || active.contains(url) {
            result.skipped += 1;
            continue;
        }
        let id = enqueue(&app, &state, url.to_string(), false, format.clone(), overrides.clone());
        if let Some(at) = start_at.filter(|at| *at > now_secs()) {
            update_job(&app, &state, id, |j| j.start_at = at);
        }
        // 1件だけの時はプレビューで分かっているタイトル・サムネイルをすぐ出す
        if let (Some(title), Some(thumbnail)) = (&title, &thumbnail) {
            update_job(&app, &state, id, |j| {
                j.title = title.clone();
                j.thumbnail = thumbnail.clone();
            });
        } else {
            prefetch_info(app.clone(), state.inner().clone(), id, url.to_string());
        }
        result.added += 1;
    }
    result
}

#[derive(serde::Serialize, Default)]
struct AddResult {
    added: usize,
    skipped: usize,
}

fn update_job(app: &AppHandle, state: &Shared, id: u64, change: impl FnOnce(&mut Job)) {
    let mut jobs = state.jobs.lock().unwrap();
    if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
        change(job);
        let _ = app.emit("job", job.clone());
    }
}

/// 待機中の項目のタイトル・サムネイルを先に調べておく(同時に2件まで)。
fn prefetch_info(app: AppHandle, state: Shared, id: u64, url: String) {
    tauri::async_runtime::spawn(async move {
        let _permit = state.prefetch_limit.acquire().await;
        let still_untitled = state.jobs.lock().unwrap().iter().any(|j| j.id == id && j.title == j.url);
        if !still_untitled {
            return;
        }
        let settings = state.settings.lock().unwrap().clone();
        if let Some((title, thumbnail, size)) = analyze::quick_info(&settings, &url).await {
            update_job(&app, &state, id, |j| {
                j.size_hint = size;
                if j.title == j.url {
                    j.title = title;
                }
                if j.thumbnail.is_empty() {
                    j.thumbnail = thumbnail;
                }
            });
        }
    });
}

#[tauri::command]
async fn ytdlp_version() -> String {
    tools::ytdlp_version().await
}

/// 選んだ更新チャンネルで今すぐ更新し、更新後のバージョンを返す。
#[tauri::command]
async fn update_ytdlp_now(state: State<'_, Shared>) -> Result<String, String> {
    let channel = state.settings.lock().unwrap().update_channel.clone();
    tools::update_ytdlp(&channel).await;
    Ok(tools::ytdlp_version().await)
}

/// 対応サイトの一覧(名前と「現在不調」かどうか)。
#[tauri::command]
async fn list_sites() -> Result<Vec<(String, bool)>, String> {
    let text = tools::supported_sites().await.map_err(|e| e.to_string())?;
    Ok(text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| match line.strip_suffix(" (CURRENTLY BROKEN)") {
            Some(name) => (name.to_string(), true),
            None => (line.to_string(), false),
        })
        .collect())
}

#[tauri::command]
fn list_jobs(state: State<Shared>) -> Vec<Job> {
    state.jobs.lock().unwrap().clone()
}

// ───────────── 一時停止・再開・順番 ─────────────

/// 一時停止する。ダウンロード中なら止めて、途中までのファイルは残す(再開で続きから)。
#[tauri::command]
fn pause_job(app: AppHandle, state: State<Shared>, id: u64) {
    let mut jobs = state.jobs.lock().unwrap();
    if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
        match job.status {
            Status::Waiting => job.status = Status::Paused,
            Status::Running => {
                state.pausing.lock().unwrap().insert(id);
                job.cancel.notify_one();
            }
            _ => {}
        }
        let _ = app.emit("job", job.clone());
    }
}

/// 一時停止したものを待ち行列に戻す。
#[tauri::command]
fn resume_job(app: AppHandle, state: State<Shared>, id: u64) {
    update_job(&app, &state, id, |j| {
        if j.status == Status::Paused {
            j.status = Status::Waiting;
            j.phase = "preparing".into();
            j.message.clear();
        }
    });
    state.queue_changed.notify_one();
}

/// すべて一時停止 / すべて再開。
#[tauri::command]
fn pause_all(app: AppHandle, state: State<Shared>, pause: bool) {
    let ids: Vec<u64> = state.jobs.lock().unwrap().iter()
        .filter(|j| if pause { matches!(j.status, Status::Waiting | Status::Running) } else { j.status == Status::Paused })
        .map(|j| j.id)
        .collect();
    for id in ids {
        if pause {
            pause_job(app.clone(), state.clone(), id);
        } else {
            resume_job(app.clone(), state.clone(), id);
        }
    }
}

/// 待ち行列の順番を変える。where_to: "top"(次に始める) / "up" / "down" / "bottom"
#[tauri::command]
fn move_job(app: AppHandle, state: State<Shared>, id: u64, where_to: String) {
    let mut jobs = state.jobs.lock().unwrap();
    let Some(from) = jobs.iter().position(|j| j.id == id) else { return };
    // 動かせるのは「予定」同士の間だけ
    let queued: Vec<usize> = jobs.iter().enumerate()
        .filter(|(_, j)| matches!(j.status, Status::Waiting | Status::Paused))
        .map(|(i, _)| i)
        .collect();
    let Some(rank) = queued.iter().position(|&i| i == from) else { return };
    let target_rank = match where_to.as_str() {
        "top" => 0,
        "up" => rank.saturating_sub(1),
        "down" => (rank + 1).min(queued.len() - 1),
        "bottom" => queued.len() - 1,
        _ => rank,
    };
    if target_rank == rank {
        return;
    }
    let job = jobs.remove(from);
    let to = queued[target_rank];
    jobs.insert(to, job);
    let _ = app.emit("jobs_reordered", jobs.iter().map(|j| j.id).collect::<Vec<_>>());
}

#[tauri::command]
fn cancel_job(app: AppHandle, state: State<Shared>, id: u64) {
    let mut jobs = state.jobs.lock().unwrap();
    if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
        match job.status {
            Status::Waiting | Status::Paused => job.status = Status::Canceled,
            Status::Running => job.cancel.notify_one(),
            _ => {}
        }
        let _ = app.emit("job", job.clone());
    }
}

#[tauri::command]
fn retry_job(app: AppHandle, state: State<Shared>, id: u64) {
    let found = state.jobs.lock().unwrap().iter().find(|j| j.id == id).cloned();
    if let Some(job) = found {
        let new_id = enqueue(&app, &state, job.url.clone(), false, job.format, job.overrides);
        update_job(&app, &state, new_id, |j| {
            j.title = job.title;
            j.thumbnail = job.thumbnail;
        });
    }
}

/// 予約していたものを今すぐ始める。
#[tauri::command]
fn start_now(app: AppHandle, state: State<Shared>, id: u64) {
    update_job(&app, &state, id, |j| j.start_at = 0);
    state.queue_changed.notify_one();
}

// ───────────── ライブラリ(保存したものの一覧) ─────────────

#[tauri::command]
fn list_history(state: State<Shared>) -> Vec<history::Entry> {
    state.history.list()
}

#[tauri::command]
fn remove_history(state: State<Shared>, ids: Vec<u64>, delete_file: bool) -> Result<(), String> {
    state.history.remove(&ids, delete_file)
}

// ───────────── 常駐・通知・アプリの更新 ─────────────

/// トレイと通知の文言を、画面の言語で受け取る。
#[tauri::command]
fn set_desktop_texts(app: AppHandle, texts: std::collections::HashMap<String, String>) {
    *app.state::<desktop::Texts>().0.lock().unwrap() = texts;
    let _ = desktop::build_tray(&app);
}

#[tauri::command]
async fn check_app_update(app: AppHandle) -> Result<Option<desktop::UpdateInfo>, String> {
    desktop::check_update(&app).await
}

#[tauri::command]
async fn install_app_update(app: AppHandle) -> Result<(), String> {
    desktop::install_update(&app).await
}

/// 失敗・キャンセルしたものをまとめてやり直す。
#[tauri::command]
fn retry_failed(app: AppHandle, state: State<Shared>) -> usize {
    let failed: Vec<Job> = state
        .jobs
        .lock()
        .unwrap()
        .iter()
        .filter(|j| matches!(j.status, Status::Failed | Status::Canceled))
        .cloned()
        .collect();
    // やり直す分は一覧から消して、新しく入れ直す
    state.jobs.lock().unwrap().retain(|j| !matches!(j.status, Status::Failed | Status::Canceled));
    for job in &failed {
        let id = enqueue(&app, &state, job.url.clone(), job.from_subscription, job.format.clone(), job.overrides.clone());
        update_job(&app, &state, id, |j| {
            j.title = job.title.clone();
            j.thumbnail = job.thumbnail.clone();
        });
    }
    failed.len()
}

/// 待機中・ダウンロード中をまとめて止める。
#[tauri::command]
fn cancel_all(app: AppHandle, state: State<Shared>) {
    let mut jobs = state.jobs.lock().unwrap();
    for job in jobs.iter_mut() {
        match job.status {
            Status::Waiting => {
                job.status = Status::Canceled;
                let _ = app.emit("job", job.clone());
            }
            Status::Running => job.cancel.notify_one(),
            _ => {}
        }
    }
}

/// 指定した状態のものを一覧から消す(done / failed)。
#[tauri::command]
fn clear_jobs(state: State<Shared>, kind: String) -> Vec<Job> {
    let mut jobs = state.jobs.lock().unwrap();
    jobs.retain(|j| match kind.as_str() {
        "done" => j.status != Status::Done,
        "failed" => !matches!(j.status, Status::Failed | Status::Canceled),
        _ => true,
    });
    jobs.clone()
}

/// 選んだものを一覧から消す(動いているもの・待っているものは消さない)
#[tauri::command]
fn clear_jobs_by_id(state: State<Shared>, ids: Vec<u64>) {
    state.jobs.lock().unwrap().retain(|j| !ids.contains(&j.id) || matches!(j.status, Status::Running | Status::Waiting));
}

#[tauri::command]
fn clear_finished(state: State<Shared>) -> Vec<Job> {
    let mut jobs = state.jobs.lock().unwrap();
    jobs.retain(|j| matches!(j.status, Status::Waiting | Status::Running | Status::Paused));
    jobs.clone()
}

#[tauri::command]
fn list_subscriptions(state: State<Shared>) -> Vec<Subscription> {
    state.subscriptions.lock().unwrap().clone()
}

/// チャンネル/プレイリストを登録する。
/// `new_only` が true なら、今ある動画は「取得済み」として記録し、今後の新着だけを落とす。
#[tauri::command]
async fn add_subscription(
    app: AppHandle,
    state: State<'_, Shared>,
    url: String,
    new_only: bool,
    overrides: Option<Overrides>,
    record_live: Option<bool>,
    group: Option<String>,
    notify_only: Option<bool>,
) -> Result<(), String> {
    let url = url.trim().to_string();
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.chars().any(char::is_whitespace) {
        return Err("URL が正しくありません".into());
    }
    if state.subscriptions.lock().unwrap().iter().any(|s| s.url == url) {
        return Err("このURLはもう登録されています".into());
    }
    // ポッドキャスト(RSS)は音声として保存する
    let mut overrides = overrides.unwrap_or_default();
    let looks_like_feed = ["/feed", ".rss", ".xml", "rss.", "/rss", "podcast"].iter().any(|p| url.to_lowercase().contains(p));
    if looks_like_feed && !overrides.contains_key("mode") {
        overrides.insert("mode".into(), "audio".into());
    }
    let id = state.next_id.fetch_add(1, Ordering::SeqCst);
    let subscription = Subscription {
        id,
        name: url.clone(),
        url: url.clone(),
        enabled: true,
        last_checked: if new_only { now_secs() } else { 0 },
        overrides,
        record_live: record_live.unwrap_or(false),
        group: group.unwrap_or_default().trim().to_string(),
        interval_minutes: 0,
        notify_only: notify_only.unwrap_or(false),
        engine: if settings::is_gallery_site(&url) { "gallery".into() } else { String::new() },
        seen: Vec::new(),
        found_hours: Vec::new(),
        name_checked: now_secs(),
        check_times: Vec::new(),
        checked_slots: Vec::new(),
    };
    let gallery = subscription.engine == "gallery";
    let notify_only = subscription.notify_only;
    update_subscriptions(&app, &state, |subs| subs.push(subscription));

    // SNS(画像・投稿)は gallery-dl で、今ある投稿を「保存済み」として記録する
    if gallery {
        if new_only {
            let settings = state.settings.lock().unwrap().clone();
            if let Err(e) = mark_gallery_existing(&settings, &url).await {
                update_subscriptions(&app, &state, |subs| subs.retain(|s| s.id != id));
                return Err(e);
            }
        }
        return Ok(());
    }
    // 知らせるだけの登録は、今ある動画を「もう知らせた」にしておく
    if notify_only {
        let items = list_items(&url).await.unwrap_or_default();
        update_subscriptions(&app, &state, |subs| {
            if let Some(s) = subs.iter_mut().find(|s| s.id == id) {
                s.seen = items.into_iter().map(|(vid, _, _)| vid).collect();
            }
        });
    }

    // 既存分の確認に失敗したら登録を取り消す(残すと「今後の新着だけ」のはずが過去分まで落ちるため)
    let name = match scan_existing(&url, new_only).await {
        Ok(name) => name,
        Err(e) => {
            update_subscriptions(&app, &state, |subs| subs.retain(|s| s.id != id));
            return Err(e.to_string());
        }
    };
    if let Some(name) = name {
        update_subscriptions(&app, &state, |subs| {
            if let Some(s) = subs.iter_mut().find(|s| s.id == id) {
                s.name = name;
            }
        });
    }
    Ok(())
}

#[tauri::command]
fn remove_subscription(app: AppHandle, state: State<Shared>, id: u64) {
    update_subscriptions(&app, &state, |subs| subs.retain(|s| s.id != id));
}

#[tauri::command]
fn toggle_subscription(app: AppHandle, state: State<Shared>, id: u64) {
    update_subscriptions(&app, &state, |subs| {
        if let Some(s) = subs.iter_mut().find(|s| s.id == id) {
            s.enabled = !s.enabled;
        }
    });
}

#[tauri::command]
fn check_subscriptions_now(app: AppHandle, state: State<Shared>) {
    check_subscriptions(&app, &state, true);
}

const OPENER: &str = if cfg!(windows) {
    "explorer"
} else if cfg!(target_os = "macos") {
    "open"
} else {
    "xdg-open"
};

#[tauri::command]
fn open_folder(path: String) {
    let _ = std::fs::create_dir_all(&path);
    // フォルダだけを開く(ファイルを渡されると、それが実行ファイルなら起動してしまうため)
    let path = std::path::Path::new(&path);
    let folder = if path.is_dir() { Some(path) } else { path.parent().filter(|p| p.is_dir()) };
    if let Some(folder) = folder {
        let _ = std::process::Command::new(OPENER).arg(folder).spawn();
    }
}

/// 保存先の空き容量と、今の動作状態(省電力・容量不足)。
#[tauri::command]
fn system_status(state: State<Shared>) -> serde_json::Value {
    let settings = state.settings.lock().unwrap().clone();
    serde_json::json!({
        "free_bytes": extras::free_space(&settings.output_dir),
        "low_space": extras::low_on_space(&settings),
        "on_battery": extras::on_battery(),
        "power_saving": extras::power_saving(&settings),
    })
}

/// 登録先のグループ・確認間隔・「知らせるだけ」を変える
#[tauri::command]
fn update_subscription(
    app: AppHandle,
    state: State<Shared>,
    id: u64,
    group: Option<String>,
    interval_minutes: Option<u64>,
    notify_only: Option<bool>,
    check_times: Option<Vec<String>>,
) {
    update_subscriptions(&app, &state, |subs| {
        if let Some(s) = subs.iter_mut().find(|s| s.id == id) {
            if let Some(g) = group {
                s.group = g.trim().to_string();
            }
            if let Some(m) = interval_minutes {
                s.interval_minutes = if m == 0 { 0 } else { m.max(15) };
            }
            if let Some(n) = notify_only {
                s.notify_only = n;
            }
            if let Some(times) = check_times {
                // "7:00" や "07:00" を "07:00" にそろえ、読めないものは捨てる
                s.check_times = times.iter().filter_map(|t| normalize_time(t)).collect();
            }
        }
    });
}

/// "7:5" → "07:05"。読めなければ None
fn normalize_time(text: &str) -> Option<String> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    (h < 24 && m < 60).then(|| format!("{h:02}:{m:02}"))
}

/// 登録先の「配信を自動で録画」を切り替える。
#[tauri::command]
fn toggle_live_recording(app: AppHandle, state: State<Shared>, id: u64) {
    update_subscriptions(&app, &state, |subs| {
        if let Some(s) = subs.iter_mut().find(|s| s.id == id) {
            s.record_live = !s.record_live;
        }
    });
}

/// Windows の右クリック「送る」に Grabbit を入れる。
#[tauri::command]
fn add_to_send_to() -> Result<(), String> {
    extras::add_to_send_to().map_err(|e| e.to_string())
}

/// 画面を描き終えたら呼ばれ、隠していたウィンドウを出す。
#[tauri::command]
fn show_window(app: AppHandle) {
    desktop::show_main_window(&app);
}

/// 記録(ログ)がある項目の一覧。新しい順。
#[tauri::command]
fn list_logs(state: State<Shared>) -> Vec<serde_json::Value> {
    let dir = settings::data_dir().join("logs");
    let jobs = state.jobs.lock().unwrap().clone();
    let mut list: Vec<(u64, std::time::SystemTime, u64)> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let id = e.path().file_stem()?.to_str()?.parse::<u64>().ok()?;
                    let meta = e.metadata().ok()?;
                    Some((id, meta.modified().ok()?, meta.len()))
                })
                .collect()
        })
        .unwrap_or_default();
    list.sort_by(|a, b| b.1.cmp(&a.1));
    list.truncate(300);
    list.into_iter()
        .map(|(id, modified, size)| {
            let job = jobs.iter().find(|j| j.id == id);
            // 再起動した後などで一覧に無い時は、記録ファイルの中からタイトルと URL を拾う
            let (log_title, log_url) = if job.is_some() { (String::new(), String::new()) } else { title_from_log(id) };
            serde_json::json!({
                "id": id,
                "title": job.map(|j| j.title.clone()).unwrap_or(log_title),
                "url": job.map(|j| j.url.clone()).unwrap_or(log_url),
                "status": job.map(|j| serde_json::to_value(&j.status).unwrap_or_default()),
                "modified": modified.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                "size": size,
            })
        })
        .collect()
}

/// 記録ファイルの先頭から、タイトルと URL を拾う
fn title_from_log(id: u64) -> (String, String) {
    use std::io::{BufRead, BufReader as StdBufReader};
    let Ok(file) = std::fs::File::open(log_path(id)) else { return Default::default() };
    let mut title = String::new();
    let mut url = String::new();
    for line in StdBufReader::new(file).lines().take(60).map_while(Result::ok) {
        if let Some(command) = line.strip_prefix("> ") {
            url = command.rsplit(' ').next().unwrap_or_default().to_string();
        } else if let Some(t) = line.strip_prefix("GRB_TITLE ") {
            title = t.trim().to_string();
            break;
        }
    }
    (title, url)
}

/// 記録(ログ)の中身。大きい時は最後の 256KB だけ返す。
#[tauri::command]
fn read_log(id: u64) -> String {
    let Ok(bytes) = std::fs::read(log_path(id)) else { return String::new() };
    let start = bytes.len().saturating_sub(256 * 1024);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

/// 記録(ログ)をすべて消す。
#[tauri::command]
fn clear_logs() {
    let _ = std::fs::remove_dir_all(settings::data_dir().join("logs"));
}

#[tauri::command]
fn open_logs_folder() {
    let dir = settings::data_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::process::Command::new(OPENER).arg(dir).spawn();
}

/// ダウンロードの記録(ログ)を開く。
#[tauri::command]
fn open_log(id: u64) {
    let path = log_path(id);
    if path.exists() {
        let _ = std::process::Command::new(OPENER).arg(path).spawn();
    }
}

/// 保存したファイルを既定のアプリで開く。
#[tauri::command]
fn open_file(path: String) {
    // 保存したファイルだけを開く(URL や存在しない物を渡されても何もしない)
    let file = std::path::Path::new(&path);
    if !file.is_file() {
        return;
    }
    // 実行ファイル(サイトが紛れ込ませた .exe など)は起動せず、入っているフォルダを開くだけにする
    let target = if fallback::is_runnable(file) { file.parent().unwrap_or(file) } else { file };
    let _ = std::process::Command::new(OPENER).arg(target).spawn();
}

/// Web ページをブラウザで開く。http / https 以外は開かない(任意のプログラムを起動させないため)。
#[tauri::command]
fn open_url(url: String) {
    let safe = (url.starts_with("https://") || url.starts_with("http://"))
        && !url.chars().any(|c| c.is_control() || c == '"' || c == ' ');
    if !safe {
        return;
    }
    #[cfg(windows)]
    let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", &url]).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new(OPENER).arg(&url).spawn();
}

/// 部品のダウンロードの進み具合を、止めるまで 0.3 秒ごとに画面へ送る。false にすると止まる
fn start_progress_ticker(app: &AppHandle) -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let (flag, app) = (running.clone(), app.clone());
    tauri::async_runtime::spawn(async move {
        while flag.load(Ordering::SeqCst) {
            let _ = app.emit("setup_progress", tools::fetch_progress());
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    });
    running
}

/// 足りない部品を入れる。その間「初期ダウンロード(名前) 何%」を画面に送り続ける。成功したら true
async fn install_with_progress(app: &AppHandle, channel: &str) -> bool {
    let installing = start_progress_ticker(app);
    let report = |message: &str| {
        let _ = app.emit("setup", message.to_string());
    };
    let result = tools::with_progress(tools::install_missing(channel, report)).await;
    installing.store(false, Ordering::SeqCst);
    if let Err(e) = result {
        report(&format!("準備に失敗しました: {e}"));
        return false;
    }
    true
}

/// 部品の一覧(入っているか・大きさ・取得元)
#[tauri::command]
fn list_tools(state: State<Shared>) -> Vec<tools::ToolInfo> {
    tools::list_tools(&state.settings.lock().unwrap().update_channel)
}

/// 部品を1つ消す。ダウンロード中は消さない
#[tauri::command]
async fn remove_tool(app: AppHandle, state: State<'_, Shared>, name: String) -> Result<(), String> {
    if state.jobs.lock().unwrap().iter().any(|j| j.status == Status::Running) {
        return Err("ダウンロード中は消せません。終わってから試してください".into());
    }
    tools::remove_tool(&name).map_err(|e| e.to_string())?;
    // yt-dlp と FFmpeg が無いと保存できないので、すぐ入れ直す(初回と同じ表示を出す)
    if matches!(name.as_str(), "yt-dlp" | "ffmpeg") {
        let shared = state.inner().clone();
        shared.ready.store(false, Ordering::SeqCst);
        let channel = shared.settings.lock().unwrap().update_channel.clone();
        tauri::async_runtime::spawn(async move {
            if install_with_progress(&app, &channel).await {
                tools::record_manifest();
                shared.ready.store(true, Ordering::SeqCst);
                shared.queue_changed.notify_one();
                let _ = app.emit("ready", true);
            }
        });
    }
    Ok(())
}

/// 部品のオン・オフ。オフにすると消して、入れ直さない。オンにすると(必要なら)すぐ入れる
#[tauri::command]
async fn set_tool_enabled(app: AppHandle, state: State<'_, Shared>, name: String, enabled: bool) -> Result<(), String> {
    if name == "yt-dlp" {
        return Err("yt-dlp はダウンロードに必ず使うので、オフにできません".into());
    }
    if state.jobs.lock().unwrap().iter().any(|j| j.status == Status::Running) {
        return Err("ダウンロード中は切り替えられません。終わってから試してください".into());
    }
    let settings = {
        let mut settings = state.settings.lock().unwrap();
        settings.disabled_tools.retain(|n| n != &name);
        if !enabled {
            settings.disabled_tools.push(name.clone());
        }
        settings.clone()
    };
    apply_global_settings(&settings);
    save_json(SETTINGS_FILE, &settings);
    if !enabled {
        return tools::remove_tool(&name).map_err(|e| e.to_string());
    }
    let channel = settings.update_channel.clone();
    tauri::async_runtime::spawn(async move {
        install_with_progress(&app, &channel).await;
        tools::record_manifest();
        let _ = app.emit("ready", true);
    });
    Ok(())
}

/// 部品を1つ、今の取得元から入れ直す
#[tauri::command]
async fn reinstall_tool(app: AppHandle, state: State<'_, Shared>, name: String) -> Result<(), String> {
    if state.jobs.lock().unwrap().iter().any(|j| j.status == Status::Running) {
        return Err("ダウンロード中は入れ直せません。終わってから試してください".into());
    }
    let channel = state.settings.lock().unwrap().update_channel.clone();
    // 入れ直している間も「何%」を出す
    let _ = app.emit("setup", format!("入れ直し({})", tools::display_name(&name)));
    let busy = start_progress_ticker(&app);
    let result = tools::with_progress(tools::reinstall_tool(&name, &channel)).await.map_err(|e| e.to_string());
    busy.store(false, Ordering::SeqCst);
    let _ = app.emit("ready", true);
    result
}

/// ダウンロードの部品を点検し、動かないものを入れ直す。
#[tauri::command]
async fn repair_tools(state: State<'_, Shared>) -> Result<Vec<String>, String> {
    let channel = state.settings.lock().unwrap().update_channel.clone();
    tools::repair(&channel).await.map(|v| v.into_iter().map(String::from).collect()).map_err(|e| e.to_string())
}

/// アプリの版
#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// 保存したファイルをフォルダ内で選択した状態で表示する。
#[tauri::command]
fn show_in_folder(path: String) {
    let file = std::path::Path::new(&path);
    // 実在するファイルだけ(explorer への余計な指定を混ぜさせない)
    if !file.exists() || path.contains('"') {
        return;
    }
    if cfg!(windows) {
        let _ = std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn();
    } else if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("open").arg("-R").arg(file).spawn();
    } else if let Some(dir) = file.parent() {
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
}

// ───────────── 待ち行列 ─────────────

fn enqueue(
    app: &AppHandle,
    state: &Shared,
    url: String,
    from_subscription: bool,
    format: Option<String>,
    overrides: Overrides,
) -> u64 {
    let id = state.next_id.fetch_add(1, Ordering::SeqCst);
    let job = Job::new(id, url, from_subscription, format, overrides);
    let _ = app.emit("job", job.clone());
    state.jobs.lock().unwrap().push(job);
    state.queue_changed.notify_one();
    id
}

/// 空きがあれば待機中のジョブを開始する。ずっと動き続ける。
async fn run_queue(app: AppHandle, state: Shared) {
    loop {
        let hours = state.settings.lock().unwrap().active_hours.clone();
        let now_minutes = {
            use chrono::Timelike;
            let now = chrono::Local::now();
            now.hour() * 60 + now.minute()
        };
        let in_hours = settings::within_active_hours(&hours, now_minutes);
        let current = state.settings.lock().unwrap().clone();
        let low_space = extras::low_on_space(&current);
        let saving = extras::power_saving(&current);
        let _ = app.emit("conditions", serde_json::json!({ "low_space": low_space, "power_saving": saving }));
        while state.ready.load(Ordering::SeqCst) {
            // 省電力中は1件ずつ。空きが足りない時は新しく始めない
            let max = if saving { 1 } else { state.settings.lock().unwrap().max_parallel.max(1) };
            if low_space {
                break;
            }
            if state.running.load(Ordering::SeqCst) >= max {
                break;
            }
            if !in_hours {
                break;
            }
            let Some(job) = take_next_waiting(&state) else { break };
            state.running.fetch_add(1, Ordering::SeqCst);
            let _ = app.emit("job", job.clone());
            tauri::async_runtime::spawn(run_job(app.clone(), state.clone(), job));
        }
        // 予約や時間帯があるので、何も起きなくても時々見直す
        tokio::select! {
            _ = state.queue_changed.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(20)) => {}
        }
    }
}

fn take_next_waiting(state: &Shared) -> Option<Job> {
    let now = now_secs();
    let order = state.settings.lock().unwrap().queue_order.clone();
    let mut jobs = state.jobs.lock().unwrap();
    // 始める条件: 「ほかが終わってから」は動いているものが無い時、「PC を使っていない時」は 5 分操作が無い時
    let running = jobs.iter().any(|j| j.status == Status::Running);
    let idle = extras::idle_seconds() >= 300;
    let condition_met = |j: &Job| match j.overrides.get("__start_when").and_then(|v| v.as_str()) {
        Some("alone") => !running,
        Some("idle") => idle,
        _ => true,
    };
    let ready = jobs.iter().enumerate().filter(|(_, j)| j.status == Status::Waiting && j.start_at <= now && condition_met(j));
    // 小さい順・大きい順。大きさが分からないものは最後に回す
    let index = match order.as_str() {
        "small" => ready.min_by_key(|(i, j)| (if j.size_hint == 0 { u64::MAX } else { j.size_hint }, *i)).map(|(i, _)| i),
        "large" => ready.max_by_key(|(i, j)| (j.size_hint, std::cmp::Reverse(*i))).map(|(i, _)| i),
        _ => ready.map(|(i, _)| i).next(),
    }?;
    let job = &mut jobs[index];
    job.status = Status::Running;
    Some(job.clone())
}

/// 1件をダウンロードする。うまくいかなければ順に別の手段を試す。
/// yt-dlp → (cookie なし) → (ブラウザ偽装) → gallery-dl → 直接ダウンロード
///
/// `message` に入れる "@..." は画面側で各言語に訳す合図。
async fn run_job(app: AppHandle, state: Shared, job: Job) {
    let mut settings = state.settings.lock().unwrap().with_overrides(&job.overrides);
    // この1件で保存先を指定していなければ、振り分けルールで保存先を決める
    if !job.overrides.contains_key("output_dir") {
        settings.route_output_dir(&job.url);
    }
    // 「プロキシだけを通す」なのにプロキシが無い時は、直接つながずに止める
    if settings.proxy_only && settings.proxy.trim().is_empty() {
        finish(&app, &state, job.id, Outcome::Failed("@proxy_required".into()), false, false, false);
        return;
    }
    // 始める前に空きを確かめる(大きさが分かっている時だけ)
    if job.size_hint > 0 {
        if let Some(free) = extras::free_space(&settings.output_dir) {
            if free < job.size_hint + job.size_hint / 10 + 200 * 1024 * 1024 {
                finish(&app, &state, job.id, Outcome::Failed(format!("@not_enough_space {:.1} GB", job.size_hint as f64 / 1e9)), false, false, false);
                return;
            }
        }
    }
    stats::connection(&job.url, "ダウンロード");
    if settings.cookies_browser == "grabbit" {
        login::touch();
    }
    let mut with_cookies = download::build_args(&settings, &job.url, job.from_subscription, job.format.as_deref());
    let mut without_cookies = download::without_browser_cookies(&with_cookies);
    let has_browser_login = !settings.cookies_browser.is_empty();
    let log = log_path(job.id);

    let update = |change: &dyn Fn(&mut Job)| {
        let mut jobs = state.jobs.lock().unwrap();
        if let Some(j) = jobs.iter_mut().find(|j| j.id == job.id) {
            change(j);
            let _ = app.emit("job", j.clone());
        }
    };
    let note = |message: &str| update(&|j| j.message = message.to_string());

    let on_event = |event| match event {
        Event::Title(title) => update(&|j| {
            j.title = title.clone();
            j.percent = 0.0;
        }),
        Event::File(path) => update(&|j| j.file_path = path.clone()),
        Event::Thumbnail(url) => update(&|j| j.thumbnail = url.clone()),
        Event::Uploader(name) => update(&|j| j.uploader = name.clone()),
        Event::Destination(path) => update(&|j| {
            if !j.partials.contains(&path) {
                j.partials.push(path.clone());
            }
        }),
        Event::Meta { duration, height } => update(&|j| {
            j.duration = duration;
            j.height = height;
        }),
        Event::Phase(phase) => update(&|j| j.phase = phase.into()),
        Event::Progress { percent, speed, eta } => update(&|j| {
            j.percent = percent;
            j.speed = speed.clone();
            j.eta = eta.clone();
            j.phase = "downloading".into();
            // 進み始めたら「再挑戦中」などの表示は消す
            if j.message.starts_with("@retry") || j.message == "@auto_retry" {
                j.message.clear();
            }
        }),
    };
    let failed_with = |outcome: &Outcome, check: fn(&str) -> bool| matches!(outcome, Outcome::Failed(m) if check(m));
    let is_failed = |outcome: &Outcome| matches!(outcome, Outcome::Failed(_));
    let run = |args: Vec<String>| download::run(args, job.cancel.clone(), &log, &on_event);

    // サムネイルを画像として選んである時は、その画像をそのまま保存する
    if settings.mode == "thumbnail" && !settings.thumbnail_url.is_empty() {
        update(&|j| j.engine = "direct".into());
        let outcome = fallback::direct_download(&settings, &settings.thumbnail_url, Some(&job.title), job.cancel.clone(), &on_event).await;
        finish(&app, &state, job.id, outcome, false, false, false);
        return;
    }

    // SNS の自動保存は、最初から画像・投稿用の方法で、保存済みを飛ばしながら取る
    if job.overrides.get("__engine").and_then(|v| v.as_str()) == Some("gallery") {
        update(&|j| j.engine = "gallery-dl".into());
        let archive = fallback::gallery_archive();
        let outcome = fallback::gallery_download_with(&settings, &job.url, Some(&archive), job.cancel.clone(), &on_event).await;
        finish(&app, &state, job.id, outcome, false, false, false);
        return;
    }

    update(&|j| j.engine = "yt-dlp".into());
    // ログイン情報は、普段は使わず「ログインが必要な内容」の時だけ使う(設定で常に使うことも可)
    let always_login = settings.cookies_when == "always" && has_browser_login;
    // このサイトで前にうまくいった方法があれば、最初からそれを使う
    let learned = if settings.remember_methods { methods::get(&job.url) } else { methods::Method::default() };
    let use_cookies_first = always_login || (learned.cookies && has_browser_login);
    let mut args = if use_cookies_first { with_cookies.clone() } else { without_cookies.clone() };
    if learned.proxy && !settings.fallback_proxy.trim().is_empty() {
        args = download::with_proxy(args, settings.fallback_proxy.trim());
    }
    if learned.impersonate {
        args = download::with_impersonate(args);
    }
    // 同じ名前のファイルがある時に「番号を付けて別名で保存」なら、保存する名前を変える
    // (1本の動画だけ。プレイリストは名前がそれぞれ違うので調べない)
    if settings.on_same_name == "rename" && !job.from_subscription {
        if let Some(existing) = find_same_name(&settings, &job.url).await {
            with_cookies = download::with_numbered_output(with_cookies, &existing);
            without_cookies = download::with_numbered_output(without_cookies, &existing);
            args = download::with_numbered_output(args, &existing);
        }
    }
    // 前に画像用の方法・直接保存で取れたサイトは、そちらを先に試す(だめなら普通の順番へ)
    let mut outcome = Outcome::Failed(String::new());
    let mut engine_used = String::new();
    if learned.engine == "gallery-dl" || learned.engine == "direct" {
        update(&|j| j.engine = learned.engine.clone());
        outcome = if learned.engine == "gallery-dl" {
            fallback::gallery_download(&settings, &job.url, job.cancel.clone(), &on_event).await
        } else {
            fallback::direct_download(&settings, &job.url, None, job.cancel.clone(), &on_event).await
        };
        if is_failed(&outcome) {
            update(&|j| j.engine = "yt-dlp".into());
        } else {
            engine_used = learned.engine.clone();
        }
    }
    if is_failed(&outcome) {
        outcome = run(args.clone()).await;
    }
    let mut cookie_problem = false;

    if !use_cookies_first && has_browser_login && failed_with(&outcome, download::is_login_required) {
        note("@retry_with_cookies");
        args = with_cookies.clone();
        outcome = run(args.clone()).await;
    }
    // ブラウザのログイン情報を読めなかった(Chrome 起動中など)→ ログインなしに戻す
    if failed_with(&outcome, download::is_cookie_error) {
        cookie_problem = true;
        note("@retry_without_cookies");
        args = without_cookies.clone();
        outcome = run(args.clone()).await;
    }
    // 部品(ダウンロードエンジン・ffmpeg)が動かなかった → 自動で直してもう一度
    if failed_with(&outcome, |m| m.contains("起動できません") || m.contains("ffmpeg not found") || m.contains("ffprobe")) {
        note("@repairing");
        if tools::repair(&settings.update_channel).await.is_ok() {
            outcome = run(args.clone()).await;
        }
    }
    // ファイル名が原因で書き込めなかった → 英数字だけの短い名前で保存し直す
    if failed_with(&outcome, download::is_filename_error) {
        note("@retry_safe_name");
        args = download::with_safe_filenames(args.clone());
        outcome = run(args.clone()).await;
    }
    // サムネイルが取れずに止まった → サムネイルなしで保存し直す(動画が大事なので)
    if settings.mode != "thumbnail" && failed_with(&outcome, download::is_thumbnail_error) {
        note("@retry_without_thumbnail");
        args = download::without_thumbnails(args.clone());
        outcome = run(args.clone()).await;
    }
    // 地域制限 → 予備のプロキシがあれば、それを通してもう一度
    if !settings.fallback_proxy.trim().is_empty() && failed_with(&outcome, download::is_geo_blocked) {
        note("@retry_proxy");
        args = download::with_proxy(args.clone(), settings.fallback_proxy.trim());
        outcome = run(args.clone()).await;
    }
    // 弾かれたサイトはブラウザになりすまして再挑戦
    if is_failed(&outcome) && settings.impersonate == "auto" && !failed_with(&outcome, download::is_login_required) {
        note("@retry_impersonate");
        let retried = run(download::with_impersonate(args.clone())).await;
        if !is_failed(&retried) {
            outcome = retried;
            args = download::with_impersonate(args.clone());
        }
    }
    // 動画として扱えない URL なら、画像用の方法 → ファイルとして直接保存
    let unsupported = failed_with(&outcome, download::is_unsupported);
    if unsupported {
        note("@retry_gallery");
        update(&|j| j.engine = "gallery-dl".into());
        let retried = fallback::gallery_download(&settings, &job.url, job.cancel.clone(), &on_event).await;
        if !is_failed(&retried) {
            outcome = retried;
            engine_used = "gallery-dl".into();
        }
    }
    if unsupported && is_failed(&outcome) {
        note("@retry_direct");
        update(&|j| j.engine = "direct".into());
        let retried = fallback::direct_download(&settings, &job.url, None, job.cancel.clone(), &on_event).await;
        if is_failed(&retried) {
            update(&|j| j.engine = "yt-dlp".into());
        } else {
            outcome = retried;
            engine_used = "direct".into();
        }
    }
    // うまくいったら、その方法を覚える(普通の方法で取れたら、覚えていたものを消す)
    if matches!(outcome, Outcome::Done) && settings.remember_methods && !settings.private_mode {
        let has = |flag: &str| args.iter().any(|a| a == flag);
        methods::remember(&job.url, methods::Method {
            cookies: engine_used.is_empty() && (has("--cookies-from-browser") || has(download::IN_APP_LOGIN_FLAG)) && !always_login,
            impersonate: engine_used.is_empty() && has("--impersonate"),
            proxy: engine_used.is_empty() && !settings.fallback_proxy.trim().is_empty() && args.windows(2).any(|w| w[0] == "--proxy" && w[1] == settings.fallback_proxy.trim()),
            engine: engine_used.clone(),
        });
    }

    let mut burn_note: Option<&str> = None;
    // 字幕の焼き付け(保存できた動画に対して行う)
    if matches!(outcome, Outcome::Done) && settings.burn_subtitles && settings.mode == "video" {
        let path = state.jobs.lock().unwrap().iter().find(|j| j.id == job.id).map(|j| j.file_path.clone()).unwrap_or_default();
        if !path.is_empty() {
            update(&|j| j.phase = "processing".into());
            note("@burning");
            let video = std::path::Path::new(&path);
            let style = extras::BurnStyle { font_size: settings.burn_font_size, color: settings.burn_color.clone(), top: settings.burn_position == "top" };
            match extras::burn_subtitles(video, &settings.subtitle_langs, &style).await {
                Ok(()) => {
                    if !settings.write_subtitle_file {
                        extras::remove_subtitle_files(video);
                    }
                    burn_note = None;
                }
                // 字幕が無いだけなら、動画は保存できているので完了のままにして知らせる
                Err(e) if e.to_string().contains("見つかりません") => burn_note = Some("@no_subtitles_to_burn"),
                Err(e) => outcome = Outcome::Failed(e.to_string()),
            }
        }
    }

    // 仕上げ: 字幕を整える・縦横を合わせる・音楽の情報と歌詞を入れる
    if matches!(outcome, Outcome::Done) {
        let path = state.jobs.lock().unwrap().iter().find(|j| j.id == job.id).map(|j| j.file_path.clone()).unwrap_or_default();
        let file = std::path::Path::new(&path);
        if !path.is_empty() && file.is_file() {
            if settings.clean_auto_subs && settings.auto_subtitles {
                extras::clean_subtitles(file, &settings.subtitle_fixes);
            }
            if settings.mode == "video" && !settings.pad_aspect.is_empty() {
                update(&|j| j.phase = "processing".into());
                note("@padding");
                if let Err(e) = extras::pad_aspect(file, &settings.pad_aspect).await {
                    outcome = Outcome::Failed(format!("@pad_failed {e}"));
                }
            }
            if settings.mode == "audio" && (settings.music_tags || settings.embed_lyrics || settings.music_bpm_key) {
                note("@music_info");
                let title = state.jobs.lock().unwrap().iter().find(|j| j.id == job.id).map(|j| j.title.clone()).unwrap_or_default();
                let (mut artist, mut song) = extras::split_artist_title(&title);
                let mut tags: Vec<(&str, String)> = Vec::new();
                if settings.music_tags {
                    if let Some((a, t, album, year)) = extras::lookup_music(&artist, &song).await {
                        artist = a;
                        song = t;
                        tags.extend([("artist", artist.clone()), ("title", song.clone()), ("album", album), ("date", year)]);
                    } else if !artist.is_empty() {
                        tags.extend([("artist", artist.clone()), ("title", song.clone())]);
                    }
                }
                if settings.embed_lyrics && !artist.is_empty() {
                    if let Some(lyrics) = extras::lookup_lyrics(&artist, &song, None).await {
                        tags.push(("lyrics", lyrics));
                    }
                }
                if settings.music_bpm_key {
                    if let Ok((bpm, key)) = audio::analyze(file).await {
                        if let Some(bpm) = bpm {
                            tags.push(("TBPM", format!("{}", bpm.round() as u32)));
                        }
                        if let Some(key) = key {
                            tags.push(("TKEY", key));
                        }
                    }
                }
                // 情報が書けなくても、音楽そのものは保存できているので失敗にはしない
                let _ = extras::write_music_tags(file, &tags).await;
                note("");
            }
        }
    }

    // 保存した動画・音声が壊れていないか調べる。壊れていたら消して、回線のエラーと同じく自動でやり直す
    if matches!(outcome, Outcome::Done) && settings.verify_files && matches!(settings.mode.as_str(), "video" | "audio") {
        let path = state.jobs.lock().unwrap().iter().find(|j| j.id == job.id).map(|j| j.file_path.clone()).unwrap_or_default();
        if !path.is_empty() {
            update(&|j| j.phase = "processing".into());
            note("@verifying");
            if let Err(problem) = extras::verify_media(std::path::Path::new(&path)).await {
                let _ = std::fs::remove_file(&path);
                update(&|j| j.file_path.clear());
                outcome = Outcome::Failed(format!("@broken_file {problem}"));
            } else if burn_note.is_none() {
                note("");
            }
        }
    }

    let saves_file = matches!(settings.mode.as_str(), "video" | "audio");
    let needs_login_but_none = !has_browser_login && failed_with(&outcome, download::is_login_required);
    finish(&app, &state, job.id, outcome, saves_file, cookie_problem, needs_login_but_none);
    if let Some(message) = burn_note {
        update_job(&app, &state, job.id, |j| j.message = message.into());
    }
}

/// 終わった1件の状態を書き込み、次の待機中を始めさせる。
fn finish(
    app: &AppHandle,
    state: &Shared,
    id: u64,
    outcome: Outcome,
    saves_file: bool,
    cookie_problem: bool,
    needs_login_but_none: bool,
) {
    let paused = state.pausing.lock().unwrap().remove(&id);
    let max_auto_retries = state.settings.lock().unwrap().auto_retry_count;
    update_job(app, state, id, |j| {
        j.speed.clear();
        j.eta.clear();
        j.message.clear();
        j.phase.clear();
        match &outcome {
            Outcome::Done => {
                j.status = Status::Done;
                j.percent = 100.0;
                if saves_file && j.file_path.is_empty() {
                    // 何も保存されなかった(絞り込みの条件に合わなかった等)。完了扱いにしない
                    j.status = Status::Failed;
                    j.message = "@nothing_saved".into();
                } else if cookie_problem {
                    j.message = "@done_without_cookies".into();
                }
            }
            // 回線のエラーなら、時間をおいて自動でやり直す(待っている間は「予定」に戻す)
            Outcome::Failed(reason) if (download::is_network_error(reason) || reason.starts_with("@broken_file")) && j.auto_retries < max_auto_retries => {
                j.status = Status::Waiting;
                j.start_at = now_secs() + download::auto_retry_delay(j.auto_retries);
                j.auto_retries += 1;
                j.percent = 0.0;
                j.message = "@auto_retry".into();
            }
            Outcome::Failed(reason) => {
                j.status = Status::Failed;
                j.message = if needs_login_but_none { "@needs_login".into() } else { reason.clone() };
            }
            Outcome::Canceled if paused => {
                j.status = Status::Paused;
                j.message = "@paused".into();
            }
            Outcome::Canceled => {
                j.status = Status::Canceled;
                // 一時停止ではなく中止なら、途中まで書いたファイル(.part など)を片付ける
                for path in j.partials.drain(..) {
                    for leftover in [format!("{path}.part"), format!("{path}.ytdl"), format!("{path}.part-Frag1")] {
                        let _ = std::fs::remove_file(leftover);
                    }
                }
            }
        }
    });
    record_and_notify(app, state, id);
    state.running.fetch_sub(1, Ordering::SeqCst);
    state.queue_changed.notify_one();
}

/// 保存できたものをライブラリに記録し、設定に合わせて通知する。
fn record_and_notify(app: &AppHandle, state: &Shared, id: u64) {
    let Some(job) = state.jobs.lock().unwrap().iter().find(|j| j.id == id).cloned() else { return };
    let settings = state.settings.lock().unwrap().with_overrides(&job.overrides);
    // 開くとプログラムとして動くファイル(.exe など)が保存されたら、隔離フォルダへ移す
    let job = if job.status == Status::Done && !job.file_path.is_empty() && fallback::is_runnable(std::path::Path::new(&job.file_path)) {
        let quarantine = settings::data_dir().join("quarantine");
        let _ = std::fs::create_dir_all(&quarantine);
        let source = std::path::PathBuf::from(&job.file_path);
        let target = quarantine.join(format!("{}-{}", job.id, source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
        let moved = std::fs::rename(&source, &target).is_ok() || (std::fs::copy(&source, &target).is_ok() && std::fs::remove_file(&source).is_ok());
        let mut updated = job.clone();
        if moved {
            updated.file_path.clear();
            updated.status = Status::Failed;
            updated.message = format!("@quarantined {}", target.display());
            update_job(app, state, id, |j| {
                j.file_path.clear();
                j.status = Status::Failed;
                j.message = updated.message.clone();
            });
        }
        updated
    } else {
        job
    };
    // 失敗の統計(記録を残さないモードでは取らない)
    if job.status == Status::Failed && !settings.private_mode {
        stats::record_failure(&job.url, &job.message);
    }
    // 自動保存で新しく保存できた時間帯を覚える(よく見つかる時間は確認を増やす)
    if job.status == Status::Done && job.from_subscription && !job.file_path.is_empty() {
        use chrono::Timelike;
        let hour = chrono::Local::now().hour() as usize;
        update_subscriptions(app, state, |subs| {
            if let Some(s) = subs.iter_mut().find(|s| s.url == job.url) {
                s.found_hours.resize(24, 0);
                s.found_hours[hour] += 1;
            }
        });
    }
    if job.status == Status::Done && !job.file_path.is_empty() && !settings.private_mode {
        state.history.add(history::Entry {
            id: job.id,
            title: job.title.clone(),
            url: job.url.clone(),
            file_path: job.file_path.clone(),
            thumbnail: job.thumbnail.clone(),
            kind: if job.engine == "yt-dlp" { settings.mode.clone() } else { "file".into() },
            saved_at: now_secs(),
            size: 0,
            exists: true,
            drive_missing: false,
            uploader: job.uploader.clone(),
            trashed: false,
            duration: job.duration,
            height: job.height,
        });
        let _ = app.emit("history_changed", ());
    }
    let finished = matches!(job.status, Status::Done | Status::Failed);
    // 記録を残さないモードでは、終わった分のログも消す
    if settings.private_mode && finished {
        let _ = std::fs::remove_file(log_path(job.id));
    }
    let skipped = job.message == "@skipped";
    if settings.notify_done && finished && !skipped {
        desktop::notify(app, job.status == Status::Done, &job.title);
    }
    // スマホへの通知(ntfy)
    if settings.ntfy_notify && finished && !skipped && !settings.ntfy_topic.trim().is_empty() {
        let (server, topic) = (settings.ntfy_server.clone(), settings.ntfy_topic.trim().to_string());
        let (ok, title) = (job.status == Status::Done, job.title.clone());
        tauri::async_runtime::spawn(async move {
            let _ = extras::ntfy_send(&server, &topic, if ok { "✓ Grabbit" } else { "✕ Grabbit" }, &title).await;
        });
    }
}

// ───────────── 登録チャンネル ─────────────

fn update_subscriptions(app: &AppHandle, state: &Shared, change: impl FnOnce(&mut Vec<Subscription>)) {
    let mut subs = state.subscriptions.lock().unwrap();
    change(&mut subs);
    save_json(SUBSCRIPTIONS_FILE, &*subs);
    let _ = app.emit("subscriptions", subs.clone());
}

/// 確認時刻を過ぎた登録先を待ち行列へ入れる。`force` なら時刻を無視する。
fn check_subscriptions(app: &AppHandle, state: &Shared, force: bool) {
    let default_interval = state.settings.lock().unwrap().check_interval_minutes.max(15);
    let now = now_secs();
    let mut due: Vec<Subscription> = Vec::new();
    update_subscriptions(app, state, |subs| {
        let local = chrono::Local::now();
        let today = local.format("%Y-%m-%d").to_string();
        let now_hm = local.format("%H:%M").to_string();
        for s in subs.iter_mut().filter(|s| s.enabled) {
            // 決まった時刻で確認する登録: その時刻を過ぎていて、今日まだならその回を確認する
            if !s.check_times.is_empty() {
                let due_slot = s.check_times.iter().filter(|t| **t <= now_hm).map(|t| format!("{today} {t}")).find(|slot| !s.checked_slots.contains(slot));
                if force || due_slot.is_some() {
                    if let Some(slot) = due_slot {
                        s.checked_slots.push(slot);
                        let extra = s.checked_slots.len().saturating_sub(20);
                        s.checked_slots.drain(..extra);
                    }
                    s.last_checked = now;
                    due.push(s.clone());
                }
                continue;
            }
            // 登録ごとの間隔があればそれを使う
            let mut interval = if s.interval_minutes > 0 { s.interval_minutes } else { default_interval } * 60;
            // よく新着が見つかる時間帯(多い方の半分以上)は、間隔を半分にする(最短 15 分)
            if s.found_hours.len() == 24 {
                use chrono::Timelike;
                let hour = chrono::Local::now().hour() as usize;
                let peak = *s.found_hours.iter().max().unwrap_or(&0);
                if peak >= 3 && s.found_hours[hour] * 2 >= peak {
                    interval = (interval / 2).max(15 * 60);
                }
            }
            if force || now.saturating_sub(s.last_checked) >= interval {
                s.last_checked = now;
                due.push(s.clone());
            }
        }
    });
    // 名前(チャンネル名など)が変わっていないか、1日1回確かめる
    for sub in due.iter().filter(|s| now.saturating_sub(s.name_checked) >= 86_400 && s.engine.is_empty()) {
        let (app, state, id, url) = (app.clone(), state.clone(), sub.id, sub.url.clone());
        tauri::async_runtime::spawn(async move {
            let name = scan_existing(&url, false).await.ok().flatten();
            update_subscriptions(&app, &state, |subs| {
                if let Some(s) = subs.iter_mut().find(|s| s.id == id) {
                    s.name_checked = now_secs();
                    if let Some(name) = name.filter(|n| !n.is_empty() && n != "NA") {
                        s.name = name;
                    }
                }
            });
        });
    }
    for sub in due {
        if sub.notify_only {
            tauri::async_runtime::spawn(notify_new_items(app.clone(), state.clone(), sub));
            continue;
        }
        let mut overrides = sub.overrides.clone();
        if sub.engine == "gallery" {
            overrides.insert("__engine".into(), "gallery".into());
        }
        enqueue(app, state, sub.url, true, None, overrides);
        // 自動保存は確認するたびに件数が変わるので、表示用の情報は取らない
    }
}

async fn run_scheduler(app: AppHandle, state: Shared) {
    let mut minutes: u64 = 0;
    // 起動前にスマホから送られていたものは受け取らない
    let mut ntfy_since = now_secs();
    loop {
        if state.ready.load(Ordering::SeqCst) {
            let saving = extras::power_saving(&state.settings.lock().unwrap());
            // 省電力中は確認の回数を減らす
            if !saving || minutes % 4 == 0 {
                check_subscriptions(&app, &state, false);
            }
            if minutes % if saving { 10 } else { 3 } == 0 {
                check_live(&app, &state).await;
            }
            // スマホから送られた URL を受け取る
            let settings = state.settings.lock().unwrap().clone();
            if settings.ntfy_receive && !settings.ntfy_topic.trim().is_empty() {
                let urls: Vec<String> = extras::ntfy_receive(&settings.ntfy_server, settings.ntfy_topic.trim(), &mut ntfy_since)
                    .await
                    .into_iter()
                    .filter(|u| !analyze::is_local_address(u))
                    .collect();
                if !urls.is_empty() {
                    desktop::show_main_window(&app);
                    let _ = app.emit("phone_urls", urls.join("\n"));
                }
            }
        }
        minutes += 1;
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

/// 「配信を自動で録画」の登録先が配信中なら、録画を始める。
async fn check_live(app: &AppHandle, state: &Shared) {
    let targets: Vec<Subscription> =
        state.subscriptions.lock().unwrap().iter().filter(|s| s.enabled && s.record_live).cloned().collect();
    let settings = state.settings.lock().unwrap().clone();
    for sub in targets {
        let Some(live) = extras::current_live(&settings, &sub.url).await else { continue };
        let already = state.jobs.lock().unwrap().iter().any(|j| j.url == live && matches!(j.status, Status::Waiting | Status::Running));
        if already {
            continue;
        }
        let mut overrides = sub.overrides.clone();
        overrides.insert("live_from_start".into(), true.into());
        overrides.insert("mode".into(), "video".into());
        let id = enqueue(app, state, live.clone(), true, None, overrides);
        update_job(app, state, id, |j| j.message = "@live_recording".into());
        prefetch_info(app.clone(), state.clone(), id, live);
    }
}

/// 登録先の名前を調べる。`mark_existing` なら今ある動画をアーカイブへ記録する。
/// チャンネル・プレイリストの新しい方から 50 件を (ID, URL, タイトル) で返す
async fn list_items(url: &str) -> anyhow::Result<Vec<(String, String, String)>> {
    let mut cmd = download::command(&tools::ytdlp_path());
    cmd.args(["--ignore-config", "--flat-playlist", "--ignore-errors", "--no-warnings", "--playlist-end", "50", "--encoding", "utf-8"])
        .args(["--print", "%(id)s\t%(webpage_url,url)s\t%(title)s", "--", url]);
    let output = tokio::time::timeout(Duration::from_secs(120), cmd.output()).await??;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let (id, link, title) = (parts.next()?, parts.next()?, parts.next().unwrap_or_default());
            link.starts_with("http").then(|| (id.to_string(), link.to_string(), title.to_string()))
        })
        .collect())
}

/// 「知らせるだけ」の登録先: 新着があれば、保存せずに一時停止の状態で一覧に入れて知らせる
async fn notify_new_items(app: AppHandle, state: Shared, sub: Subscription) {
    let Ok(items) = list_items(&sub.url).await else { return };
    let fresh: Vec<_> = items.iter().filter(|(id, _, _)| !sub.seen.contains(id)).cloned().collect();
    if fresh.is_empty() {
        return;
    }
    for (_, link, title) in &fresh {
        let id = enqueue(&app, &state, link.clone(), true, None, sub.overrides.clone());
        update_job(&app, &state, id, |j| {
            j.status = Status::Paused;
            j.title = title.clone();
            j.message = "@new_item".into();
        });
    }
    update_subscriptions(&app, &state, |subs| {
        if let Some(s) = subs.iter_mut().find(|s| s.id == sub.id) {
            s.seen.extend(fresh.iter().map(|(id, _, _)| id.clone()));
            // 増えすぎないよう新しい方 500 件だけ覚える
            let extra = s.seen.len().saturating_sub(500);
            s.seen.drain(..extra);
        }
    });
    desktop::notify_text(&app, &format!("{}: {} 件", sub.name, fresh.len()));
}

/// SNS の自動保存を登録した時、今ある投稿を落とさずに「保存済み」として記録する
async fn mark_gallery_existing(settings: &Settings, url: &str) -> Result<(), String> {
    let program = tools::ensure_gallery_dl().await.map_err(|e| e.to_string())?;
    let mut cmd = download::command(&program);
    cmd.arg("--config-ignore").arg("--no-download").arg("--download-archive").arg(fallback::gallery_archive());
    if settings.cookies_browser == "grabbit" && login::cookie_file().exists() {
        cmd.arg("--cookies").arg(login::cookie_file());
    }
    cmd.args(["--", url]);
    let output = tokio::time::timeout(Duration::from_secs(300), cmd.output())
        .await
        .map_err(|_| "時間切れになりました".to_string())?
        .map_err(|e| e.to_string())?;
    if !output.status.success() && output.stdout.is_empty() {
        return Err(format!("このアカウントを読み込めませんでした: {}", String::from_utf8_lossy(&output.stderr).lines().last().unwrap_or_default()));
    }
    Ok(())
}

async fn scan_existing(url: &str, mark_existing: bool) -> anyhow::Result<Option<String>> {
    const NAME_TAG: &str = "GRB_NAME ";
    const ITEM_TAG: &str = "GRB_ITEM ";
    let mut cmd = download::command(&tools::ytdlp_path());
    cmd.args(["--ignore-config", "--flat-playlist", "--ignore-errors"])
        .args(["--print", &format!("playlist:{NAME_TAG}%(title)s")])
        .args(["--print", &format!("{ITEM_TAG}%(ie_key)s %(id)s")]);
    if !mark_existing {
        cmd.args(["--playlist-items", "0"]);
    }
    cmd.args(["--", url]).stdout(std::process::Stdio::piped());

    let mut child = cmd.spawn()?;
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut name = None;
    let mut archive_lines = String::new();
    while let Some(line) = download::read_line_lossy(&mut reader).await {
        if let Some(n) = line.strip_prefix(NAME_TAG) {
            name = Some(n.trim().to_string());
        } else if let Some(item) = line.strip_prefix(ITEM_TAG) {
            // アーカイブの形式は「抽出器名(小文字) 動画ID」
            if let Some((key, id)) = item.split_once(' ') {
                archive_lines.push_str(&format!("{} {}\n", key.to_lowercase(), id.trim()));
            }
        }
    }
    child.wait().await?;

    if mark_existing && !archive_lines.is_empty() {
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(settings::archive_path())
            .await?;
        file.write_all(archive_lines.as_bytes()).await?;
    }
    Ok(name)
}

// ───────────── 起動 ─────────────

fn main() {
    // 暗号化の設定は、記録を読む前に知る必要があるので、設定だけ先に読む
    let initial: Settings = load_json(SETTINGS_FILE);
    apply_global_settings(&initial);
    let state: Shared = Arc::new(AppState {
        settings: Mutex::new(initial),
        subscriptions: Mutex::new(load_json(SUBSCRIPTIONS_FILE)),
        jobs: Mutex::new(Vec::new()),
        next_id: AtomicU64::new(now_secs() * 1000),
        running: AtomicUsize::new(0),
        ready: AtomicBool::new(false),
        queue_changed: Notify::new(),
        history: history::History::load(),
        pausing: Mutex::new(std::collections::HashSet::new()),
        prefetch_limit: tokio::sync::Semaphore::new(2),
    });

    // 暗号化して保存していたログイン情報を、ダウンロード機能が読める形に戻す
    login::unlock_at_startup();
    // 長く使っていないログイン情報は消す(設定で日数を決めた時だけ)
    LOGIN_EXPIRED.store(login::expire_unused(state.settings.lock().unwrap().login_expire_days), Ordering::SeqCst);

    tauri::Builder::default()
        // 2つ目を起動した時は、新しく開かずに今の画面を前に出す
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            desktop::show_main_window(app);
            // 「送る」メニューや URL をドロップして起動された時は、その URL を画面に渡す
            let urls = extras::urls_from_args(&args);
            if !urls.is_empty() {
                let _ = app.emit("incoming_urls", urls.join("\n"));
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state.clone())
        .manage(desktop::Texts::default())
        .on_window_event(|window, event| {
            // 閉じるボタンではトレイに隠れて動き続ける(設定で変えられる)
            // トレイに隠すのは本体の画面だけ(ログイン用の小さなブラウザは普通に閉じる)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != "main" {
                    return;
                }
                let state = window.state::<Shared>();
                if state.settings.lock().unwrap().close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            let state = app.state::<Shared>().inner().clone();
            desktop::build_tray(&handle)?;
            let show_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                if let Some(window) = show_handle.get_webview_window("main") {
                    if !window.is_visible().unwrap_or(true) {
                        let _ = window.show();
                    }
                }
            });
            let clipboard_state = state.clone();
            desktop::watch_clipboard(handle.clone(), move || clipboard_state.settings.lock().unwrap().watch_clipboard);
            tauri::async_runtime::spawn(async move {
                let channel = state.settings.lock().unwrap().update_channel.clone();
                // 部品が書き換えられていないか確かめ、違っていたら消して入れ直す
                let tampered = tools::remove_tampered();
                if !tampered.is_empty() {
                    let _ = handle.emit("tools_tampered", tampered);
                }
                let already_installed = tools::is_installed();
                if !install_with_progress(&handle, &channel).await {
                    return;
                }
                tools::record_manifest();
                // 2回目以降はすぐ使えるようにし、yt-dlp の更新は裏で行う
                let pinned = state.settings.lock().unwrap().pin_ytdlp;
                if already_installed && !pinned {
                    tauri::async_runtime::spawn(async move {
                        tools::update_ytdlp(&channel).await;
                        tools::record_manifest();
                    });
                }
                state.ready.store(true, Ordering::SeqCst);
                let _ = handle.emit("ready", true);
                // 部品が壊れていたら自動で直す(画面には直した時だけ知らせる)
                let repair_app = handle.clone();
                let repair_channel = state.settings.lock().unwrap().update_channel.clone();
                tauri::async_runtime::spawn(async move {
                    if let Ok(fixed) = tools::repair(&repair_channel).await {
                        if !fixed.is_empty() {
                            tools::record_manifest();
                            let _ = repair_app.emit("tools_repaired", fixed);
                        }
                    }
                });
                // アプリ本体の新しい版があれば、起動したばかりで何も動いていないうちに入れて再起動する
                if state.settings.lock().unwrap().auto_update_app {
                    let update_app = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Ok(Some(info)) = desktop::check_update(&update_app).await {
                            let _ = update_app.emit("app_update", info);
                            let _ = desktop::install_update(&update_app).await;
                        }
                    });
                }
                state.queue_changed.notify_one();
                tauri::async_runtime::spawn(run_queue(handle.clone(), state.clone()));
                run_scheduler(handle, state).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            reset_settings,
            is_ready,
            startup_urls,
            ytdlp_version,
            update_ytdlp_now,
            add_downloads,
            list_jobs,
            cancel_job,
            pause_job,
            resume_job,
            pause_all,
            move_job,
            retry_job,
            clear_finished,
            retry_failed,
            cancel_all,
            clear_jobs,
            list_subscriptions,
            add_subscription,
            open_login,
            pick_folder,
            trim_media,
            power_action,
            rename_library,
            check_existing_file,
            test_proxy,
            diagnose,
            export_failed,
            scrub_text,
            login_locked,
            unlock_login,
            set_login_password,
            restore_jobs,
            clear_jobs_by_id,
            restore_history,
            restore_from_trash,
            move_library,
            set_taskbar_progress,
            list_connections,
            error_stats,
            clear_error_stats,
            startup_notices,
            app_release_notes,
            convert_media,
            check_sources,
            ytdlp_changelog,
            rollback_ytdlp,
            bug_report,
            performance_stats,
            ntfy_test,
            verify_library,
            list_site_methods,
            forget_site_method,
            export_settings,
            import_settings,
            login_sites,
            forget_login,
            remove_subscription,
            toggle_subscription,
            check_subscriptions_now,
            open_folder,
            open_file,
            open_log,
            open_url,
            repair_tools,
            list_tools,
            remove_tool,
            set_tool_enabled,
            reinstall_tool,
            app_version,
            list_logs,
            read_log,
            clear_logs,
            open_logs_folder,
            show_window,
            system_status,
            toggle_live_recording,
            update_subscription,
            add_to_send_to,
            show_in_folder,
            analyze_url,
            quick_preview,
            search_videos,
            localized_thumbnails,
            start_now,
            list_history,
            remove_history,
            set_desktop_texts,
            check_app_update,
            install_app_update,
            list_sites,
        ])
        .build(tauri::generate_context!())
        .expect("Grabbit の起動に失敗しました")
        .run(|_app, event| {
            // 閉じる時に、ログイン情報を暗号化して保存する
            if let tauri::RunEvent::Exit = event {
                login::lock_at_exit();
            }
        });
}
