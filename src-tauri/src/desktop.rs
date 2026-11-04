//! デスクトップまわり: タスクトレイ常駐、通知、クリップボード監視、アプリ本体の更新。
//! 画面の言語に合わせるため、トレイと通知の文言は画面側から受け取る(set_desktop_texts)。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;

const TRAY_ID: &str = "main";

/// 画面側から受け取った文言(キー → 今の言語の文)
#[derive(Default)]
pub struct Texts(pub Mutex<HashMap<String, String>>);

impl Texts {
    fn get(&self, key: &str, fallback: &str) -> String {
        self.0.lock().unwrap().get(key).cloned().unwrap_or_else(|| fallback.to_string())
    }
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// タスクトレイのアイコンとメニューを作る(言語が変わった時は作り直す)。
pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let texts = app.state::<Texts>();
    let open = MenuItem::with_id(app, "open", texts.get("open", "Open Grabbit"), true, None::<&str>)?;
    let folder = MenuItem::with_id(app, "folder", texts.get("folder", "Open save folder"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", texts.get("quit", "Quit"), true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &folder, &quit])?;

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        return tray.set_menu(Some(menu));
    }
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(app.default_window_icon().cloned().expect("アプリのアイコンがあるはず"))
        .tooltip("Grabbit")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "folder" => {
                let _ = app.emit("tray_open_folder", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// 保存の結果を通知で知らせる。
pub fn notify(app: &AppHandle, done: bool, title: &str) {
    let texts = app.state::<Texts>();
    let heading = if done { texts.get("done", "Saved") } else { texts.get("failed", "Download failed") };
    let _ = app.notification().builder().title(heading).body(title).show();
}

/// 新着などを知らせる(見出しは「新着」)
pub fn notify_text(app: &AppHandle, body: &str) {
    let texts = app.state::<Texts>();
    let _ = app.notification().builder().title(texts.get("new_items", "New uploads")).body(body).show();
}

/// コピーされた URL を見張る。新しい URL がコピーされたら画面に知らせる。
/// `enabled` は設定を読む関数(止めている間は何もしない)。
pub fn watch_clipboard(app: AppHandle, enabled: impl Fn() -> bool + Send + 'static) {
    std::thread::spawn(move || {
        let Ok(mut clipboard) = arboard::Clipboard::new() else { return };
        // 起動前からコピーされていたものは知らせない
        let mut last = clipboard.get_text().unwrap_or_default();
        loop {
            std::thread::sleep(Duration::from_millis(800));
            if !enabled() {
                continue;
            }
            let Ok(text) = clipboard.get_text() else { continue };
            if text == last {
                continue;
            }
            last = text.clone();
            let urls = crate::download::extract_urls(&text);
            if !urls.is_empty() && urls.len() <= 200 {
                let _ = app.emit("clipboard_urls", urls.join("\n"));
            }
        }
    });
}

#[derive(Serialize, Deserialize, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
}

/// アプリ本体の新しい版があるか調べる。
pub async fn check_update(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| e.to_string())?;
    Ok(update.map(|u| UpdateInfo { version: u.version.clone(), notes: u.body.clone().unwrap_or_default() }))
}

/// 新しい版を入れて再起動する。
pub async fn install_update(app: &AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(());
    };
    // 進み具合は (受け取った量, 全体の量) で送る。画面は部品のダウンロードと同じ形で出す
    let _ = app.emit("app_update_start", update.version.clone());
    let (progress_app, finish_app) = (app.clone(), app.clone());
    let mut received = 0u64;
    update
        .download_and_install(
            move |chunk, total| {
                received += chunk as u64;
                let _ = progress_app.emit("app_update_progress", (received, total.unwrap_or(0)));
            },
            move || {
                let _ = finish_app.emit("app_update_installing", ());
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}
