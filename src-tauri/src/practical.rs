//! 実際に動かして確かめるテスト(普段の `cargo test` では動かさない)。
//! 準備: GRABBIT_PRACTICAL_URL に試験用の Web サーバー(test.mp4 などを置いたもの)の URL、
//! XDG_DATA_HOME に yt-dlp・ffmpeg を入れた試験用のフォルダを渡して
//! `cargo test -- --ignored practical` で動かす。

use crate::download::{self, Event, Outcome};
use crate::settings::Settings;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

fn base() -> String {
    std::env::var("GRABBIT_PRACTICAL_URL").expect("GRABBIT_PRACTICAL_URL を渡してください")
}

fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("grabbit-practical").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn settings_for(dir: &Path) -> Settings {
    Settings { output_dir: dir.to_string_lossy().into_owned(), ..Settings::default() }
}

/// ダウンロードを1件動かして、結果と保存したファイルを返す
async fn download(settings: &Settings, url: &str) -> (Outcome, Vec<String>) {
    let files = Mutex::new(Vec::new());
    let log = std::env::temp_dir().join("grabbit-practical").join(format!("{}.log", url.len()));
    let args = download::build_args(settings, url, false, None);
    let outcome = download::run(args, Arc::new(Notify::new()), &log, &|event| {
        if let Event::File(path) = event {
            files.lock().unwrap().push(path);
        }
    })
    .await;
    let files = files.into_inner().unwrap();
    (outcome, files)
}

#[tokio::test]
#[ignore]
async fn practical_download_and_analyze() {
    let dir = work_dir("download");
    let settings = settings_for(&dir);
    // 解析(ファイルそのものの URL)
    let info = crate::analyze::analyze(&settings, &format!("{}/test.mp4", base())).await.expect("解析できること");
    assert!(!info.title.is_empty(), "タイトルが取れること");
    // 保存
    let (outcome, files) = download(&settings, &format!("{}/test.mp4", base())).await;
    assert!(matches!(outcome, Outcome::Done), "保存できること: {:?}", matches!(outcome, Outcome::Failed(_)));
    assert_eq!(files.len(), 1, "保存したファイルを知らせること");
    assert!(Path::new(&files[0]).exists(), "ファイルが実際にあること");
    // 保存したファイルの検査
    crate::extras::verify_media(Path::new(&files[0])).await.expect("正しいファイルは合格すること");
}

#[tokio::test]
#[ignore]
async fn practical_broken_file_is_detected() {
    let dir = work_dir("broken");
    let target = dir.join("broken.mp4");
    let bytes = reqwest::get(format!("{}/broken.mp4", base())).await.unwrap().bytes().await.unwrap();
    std::fs::write(&target, bytes).unwrap();
    assert!(crate::extras::verify_media(&target).await.is_err(), "途中で切れたファイルは不合格になること");
}

#[tokio::test]
#[ignore]
async fn practical_post_processing() {
    let dir = work_dir("post");
    let video = dir.join("clip.mp4");
    std::fs::write(&video, reqwest::get(format!("{}/test.mp4", base())).await.unwrap().bytes().await.unwrap()).unwrap();
    let srt = dir.join("clip.ja.srt");
    std::fs::write(&srt, reqwest::get(format!("{}/test.ja.srt", base())).await.unwrap().bytes().await.unwrap()).unwrap();

    // 字幕を整える + 間違いを直す
    crate::extras::clean_subtitles(&video, "こんにちわ→こんにちは");
    let cleaned = std::fs::read_to_string(&srt).unwrap();
    assert_eq!(cleaned.matches("こんにちは").count(), 1, "重なりが消えて直っていること: {cleaned}");
    assert!(!cleaned.contains("<c>"));

    // 焼き付け(色・位置つき)
    let style = crate::extras::BurnStyle { font_size: 28, color: "yellow".into(), top: true };
    crate::extras::burn_subtitles(&video, "ja", &style).await.expect("焼き付けられること");
    crate::extras::verify_media(&video).await.expect("焼き付け後も再生できること");

    // 縦長にする(16:9 → 9:16)
    crate::extras::pad_aspect(&video, "9:16").await.expect("縦横を合わせられること");
    let probe = std::process::Command::new(crate::tools::bin_dir().join("ffprobe"))
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0"])
        .arg(&video)
        .output()
        .unwrap();
    let size = String::from_utf8_lossy(&probe.stdout).trim().to_string();
    let (w, h): (f64, f64) = size.split_once(',').map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap())).unwrap();
    assert!((w / h - 9.0 / 16.0).abs() < 0.02, "縦長になっていること: {size}");

    // 切り抜き(速い方・正確な方)
    let quick = crate::extras::trim(&video, 2.0, 5.0, false).await.expect("切り抜けること");
    let exact = crate::extras::trim(&video, 2.0, 5.0, true).await.expect("正確に切り抜けること");
    for clip in [&quick, &exact] {
        crate::extras::verify_media(clip).await.expect("切り抜いたものが再生できること");
    }

    // 音声を取り出す → 波形動画 → カラオケ版・歌声版
    let audio = crate::extras::extract_audio(&video).await.expect("音声を取り出せること");
    let wave = crate::extras::waveform_video(&audio).await.expect("波形動画を作れること");
    crate::extras::verify_media(&wave).await.expect("波形動画が再生できること");
    let karaoke = crate::audio::vocal_version(&audio, false).await.expect("カラオケ版を作れること");
    let vocals = crate::audio::vocal_version(&audio, true).await.expect("歌声版を作れること");
    assert!(karaoke.exists() && vocals.exists());

    // 名前の付け直し(同じ名前があれば番号)
    let renamed = crate::extras::rename_file(&quick, "clip").expect("名前を変えられること");
    assert!(renamed.file_name().unwrap().to_string_lossy().starts_with("clip"), "{renamed:?}");
}

#[tokio::test]
#[ignore]
async fn practical_music() {
    let dir = work_dir("music");
    let song = dir.join("Artist - Song.mp3");
    std::fs::write(&song, reqwest::get(format!("{}/song.mp3", base())).await.unwrap().bytes().await.unwrap()).unwrap();
    let (bpm, key) = crate::audio::analyze(&song).await.expect("音を解析できること");
    assert!(key.is_some(), "キーが出ること");
    let _ = bpm; // 単音なのでテンポは出なくてよい
    crate::extras::write_music_tags(&song, &[("artist", "Artist".into()), ("title", "Song".into()), ("TBPM", "120".into()), ("lyrics", "la la".into())])
        .await
        .expect("曲の情報を書けること");
    let probe = std::process::Command::new(crate::tools::bin_dir().join("ffprobe"))
        .args(["-v", "error", "-show_entries", "format_tags", "-of", "json"])
        .arg(&song)
        .output()
        .unwrap();
    let tags = String::from_utf8_lossy(&probe.stdout).to_lowercase();
    assert!(tags.contains("\"artist\": \"artist\"") && tags.contains("song"), "{tags}");
}

#[tokio::test]
#[ignore]
async fn practical_dangerous_file_name() {
    let dir = work_dir("evil");
    let settings = settings_for(&dir);
    let names = Mutex::new(Vec::new());
    let outcome = crate::fallback::direct_download(&settings, &format!("{}/evil", base()), None, Arc::new(Notify::new()), &|event| {
        if let Event::File(path) = event {
            names.lock().unwrap().push(path);
        }
    })
    .await;
    assert!(matches!(outcome, Outcome::Done));
    let saved = names.into_inner().unwrap();
    let path = Path::new(&saved[0]);
    assert_eq!(path.parent().unwrap(), dir, "保存先の外へ書き出さないこと: {path:?}");
    assert!(path.to_string_lossy().ends_with(".exe.download"), "実行ファイルのまま保存しないこと: {path:?}");
    assert!(crate::fallback::is_runnable(Path::new("x.EXE")));
}


#[tokio::test]
#[ignore]
async fn practical_local_preview_is_blocked() {
    // 家の中の機器へは簡易プレビューで問い合わせない
    assert!(crate::analyze::quick_preview("http://127.0.0.1:18901/index.html").await.is_none());
}

#[tokio::test]
#[ignore]
async fn practical_every_option_is_accepted() {
    // 設定を全部入れても、ダウンロード機能が引数を受け付けて保存できること
    let dir = work_dir("options");
    let mut settings = settings_for(&dir);
    settings.fast_mode = true;
    settings.audio_language = "ja".into();
    settings.playlist_order = "reverse".into();
    settings.embed_source_info = true;
    settings.embed_metadata = true;
    settings.embed_thumbnail = true;
    settings.on_same_name = "overwrite".into();
    settings.preset = "720".into();
    settings.write_subtitle_file = true;
    settings.auto_subtitles = true;
    settings.subtitle_translate_to = "en".into();
    let (outcome, files) = download(&settings, &format!("{}/test.mp4", base())).await;
    assert!(matches!(outcome, Outcome::Done), "動画: {}", match outcome { Outcome::Failed(m) => m, _ => String::new() });
    assert!(!files.is_empty());
    // 音声トラックを全部
    let mut all = settings.clone();
    all.all_audio_tracks = true;
    all.output_dir = work_dir("options-all").to_string_lossy().into_owned();
    let (outcome, _) = download(&all, &format!("{}/test.mp4", base())).await;
    assert!(matches!(outcome, Outcome::Done), "全部の音声: {}", match outcome { Outcome::Failed(m) => m, _ => String::new() });
    // 音楽として(曲番号つき)
    let mut music = settings_for(&work_dir("options-audio"));
    music.mode = "audio".into();
    music.music_track_numbers = true;
    let (outcome, files) = download(&music, &format!("{}/test.mp4", base())).await;
    assert!(matches!(outcome, Outcome::Done), "音楽: {}", match outcome { Outcome::Failed(m) => m, _ => String::new() });
    assert!(files.iter().any(|f| f.ends_with(".mp3") || f.ends_with(".m4a") || f.ends_with(".opus")), "{files:?}");
}

#[tokio::test]
#[ignore]
async fn practical_web_page_with_video() {
    // ページの中の動画を見つけて保存できること(よくある「動画が埋め込まれたページ」)
    // このページはサムネイルの場所を「/thumb.jpg」と書いているので、サムネイルの埋め込みで止まる。
    // アプリはその時サムネイルなしでやり直すので、同じ手順で確かめる
    let dir = work_dir("page");
    let settings = settings_for(&dir);
    let url = format!("{}/index.html", base());
    let (outcome, _) = download(&settings, &url).await;
    let message = match &outcome { Outcome::Failed(m) => m.clone(), _ => String::new() };
    if !message.is_empty() {
        assert!(download::is_thumbnail_error(&message), "サムネイル以外の理由で失敗: {message}");
        let files = Mutex::new(Vec::new());
        let args = download::without_thumbnails(download::build_args(&settings, &url, false, None));
        let log = std::env::temp_dir().join("grabbit-practical/page.log");
        let outcome = download::run(args, Arc::new(Notify::new()), &log, &|e| if let Event::File(p) = e { files.lock().unwrap().push(p) }).await;
        assert!(matches!(outcome, Outcome::Done), "サムネイルなしでも保存できること");
        assert!(!files.into_inner().unwrap().is_empty());
    }
}

#[tokio::test]
#[ignore]
async fn practical_settings_backup_restores() {
    // 設定ファイルが壊れても、前回の控えから戻せること(XDG_DATA_HOME の試験用フォルダで動かす)
    let name = "practical_settings.json";
    let path = crate::settings::data_dir().join(name);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("json.bak"));
    let first = Settings { output_dir: "/first".into(), ..Settings::default() };
    let second = Settings { output_dir: "/second".into(), ..Settings::default() };
    crate::settings::save_json(name, &first);
    crate::settings::save_json(name, &second); // ここで first が控えになる
    std::fs::write(&path, "{ 壊れた").unwrap();
    let loaded: Settings = crate::settings::load_json(name);
    assert_eq!(loaded.output_dir, "/first", "控えから戻ること");
    assert!(crate::settings::RESTORED_FROM_BACKUP.load(std::sync::atomic::Ordering::SeqCst));
}


#[tokio::test]
#[ignore]
async fn practical_gallery_engine() {
    // 画像用の方法(gallery-dl)で、保存済みを飛ばしながら取れること
    let dir = work_dir("gallery");
    let settings = settings_for(&dir);
    let archive = dir.join("archive.sqlite3");
    let url = format!("{}/thumb.jpg", base());
    let count = Mutex::new(0);
    let first = crate::fallback::gallery_download_with(&settings, &url, Some(&archive), Arc::new(Notify::new()), &|e| {
        if let Event::File(_) = e {
            *count.lock().unwrap() += 1;
        }
    })
    .await;
    assert!(matches!(first, Outcome::Done), "{}", match first { Outcome::Failed(m) => m, _ => String::new() });
    assert_eq!(*count.lock().unwrap(), 1, "1枚保存すること");
    // 2回目は保存済みとして飛ばす
    *count.lock().unwrap() = 0;
    let _ = crate::fallback::gallery_download_with(&settings, &url, Some(&archive), Arc::new(Notify::new()), &|e| {
        if let Event::File(_) = e {
            *count.lock().unwrap() += 1;
        }
    })
    .await;
    assert_eq!(*count.lock().unwrap(), 0, "保存済みは飛ばすこと");
}

#[tokio::test]
#[ignore]
async fn practical_errors_are_understood() {
    let dir = work_dir("errors");
    let settings = settings_for(&dir);
    // 無いページ → 「見られない動画」
    let (outcome, _) = download(&settings, &format!("{}/nothing.mp4", base())).await;
    let message = match outcome { Outcome::Failed(m) => m, _ => panic!("無いページは失敗すること") };
    assert_eq!(crate::stats::classify(&message), "unavailable", "{message}");
    // サイト側の一時的なエラー(500) → 回線のエラーとして自動でやり直す対象
    let (outcome, _) = download(&settings, &format!("{}/slow", base())).await;
    let message = match outcome { Outcome::Failed(m) => m, _ => panic!("500 は失敗すること") };
    assert!(download::is_network_error(&message), "{message}");
}

#[tokio::test]
#[ignore]
async fn practical_cancel_stops() {
    let dir = work_dir("cancel");
    let mut settings = settings_for(&dir);
    settings.rate_limit = "20K".into(); // ゆっくり落として、途中で止める
    let cancel = Arc::new(Notify::new());
    // 書き込みが始まったのを見てから止める(早すぎても遅すぎても .part が残らないため)
    let started = Arc::new(Notify::new());
    let (stopper, waiter) = (cancel.clone(), started.clone());
    tokio::spawn(async move {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(20), waiter.notified()).await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        stopper.notify_waiters();
        stopper.notify_one();
    });
    let log = std::env::temp_dir().join("grabbit-practical/cancel.log");
    let partials = Mutex::new(Vec::new());
    let outcome = download::run(download::build_args(&settings, &format!("{}/test.mp4", base()), false, None), cancel, &log, &|e| {
        if let Event::Destination(p) = e {
            partials.lock().unwrap().push(p);
            started.notify_one();
        }
    })
    .await;
    assert!(matches!(outcome, Outcome::Canceled), "中止できること");
    // 書き込み中のファイルを知らせ、アプリはその .part を片付けられること
    let partials = partials.into_inner().unwrap();
    assert!(!partials.is_empty(), "書き込み中のファイルを知らせること");
    let part = format!("{}.part", partials[0]);
    assert!(Path::new(&part).exists(), "途中のファイルがあること: {part}");
    std::fs::remove_file(&part).unwrap();
}

#[tokio::test]
#[ignore]
async fn practical_same_name_is_numbered() {
    let dir = work_dir("samename");
    let settings = settings_for(&dir);
    let url = format!("{}/test.mp4", base());
    let (_, first) = download(&settings, &url).await;
    // 同じ名前があることを、保存前に見つけられる
    let mut cmd = download::command(&crate::tools::ytdlp_path());
    cmd.args(download::predict_filename_args(&settings, &url));
    let out = cmd.output().await.unwrap();
    let predicted = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let existing = download::existing_same_name(Path::new(&predicted)).expect("同じ名前を見つけること");
    assert_eq!(existing, PathBuf::from(&first[0]));
    // 番号を付けて別名で保存
    let files = Mutex::new(Vec::new());
    let args = download::with_numbered_output(download::build_args(&settings, &url, false, None), &existing);
    let log = std::env::temp_dir().join("grabbit-practical/same.log");
    let outcome = download::run(args, Arc::new(Notify::new()), &log, &|e| if let Event::File(p) = e { files.lock().unwrap().push(p) }).await;
    assert!(matches!(outcome, Outcome::Done));
    let saved = files.into_inner().unwrap();
    assert!(saved[0].contains("(2)"), "番号付きで保存すること: {saved:?}");
    assert!(Path::new(&first[0]).exists(), "元のファイルは残ること");
}
