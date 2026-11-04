//! ユーザー設定と登録チャンネルの保存・読み込み。
//! どちらも設定フォルダ内の JSON ファイルとして保存する。

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// アプリのデータ保存先(設定・アーカイブ・ツール類)。
pub fn data_dir() -> PathBuf {
    let dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Grabbit");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// ダウンロード済み動画の記録。重複ダウンロード防止に使う。
pub fn archive_path() -> PathBuf {
    data_dir().join("archive.txt")
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    /// 保存先フォルダ
    pub output_dir: String,
    /// 画質プリセット: best / 2160 / 1080 / 720 / audio_mp3 / audio_m4a
    pub preset: String,
    /// 動画の入れ物: mp4 / mkv
    pub container: String,
    /// ファイル名・フォルダ分けのテンプレート
    pub filename_template: String,
    pub embed_thumbnail: bool,
    pub embed_metadata: bool,
    pub embed_subtitles: bool,
    /// 字幕の言語(カンマ区切り)
    pub subtitle_langs: String,
    /// SponsorBlock でスポンサー区間を除去する
    pub sponsorblock_remove: bool,
    /// ログイン情報を借りるブラウザ名。空なら使わない
    pub cookies_browser: String,
    /// 同時ダウンロード数
    pub max_parallel: usize,
    /// 1本あたりの分割ダウンロード数
    pub concurrent_fragments: u32,
    /// 帯域制限(例: 5M)。空なら無制限
    pub rate_limit: String,
    /// 登録チャンネルの確認間隔(分)
    pub check_interval_minutes: u64,
    /// 上級者向け: yt-dlp にそのまま渡す追加引数
    pub extra_args: String,
    /// yt-dlp の更新チャンネル: stable / nightly / master
    pub update_channel: String,
    /// 部品ごとの取得元(部品の名前 → GitHub の「持ち主/リポジトリ名」)。無ければ標準
    pub tool_repos: std::collections::BTreeMap<String, String>,
    /// 自分で追加した取得元の候補
    pub custom_repos: Vec<String>,
    /// オフにした部品(入れない・使わない)
    pub disabled_tools: Vec<String>,

    // ── 画質・形式の詳細 ──
    /// 優先する映像コーデック: any / h264 / vp9 / av1
    pub video_codec: String,
    /// 音声のみ保存時のビットレート(例: 192K)。空なら最高
    pub audio_bitrate: String,
    /// 変換後も元の動画ファイルを残す
    pub keep_original: bool,

    // ── 絞り込み ──
    pub skip_shorts: bool,
    pub skip_live: bool,
    /// この日付以降の動画だけ(YYYYMMDD)
    pub date_after: String,
    /// ファイルサイズの下限・上限(例: 10M / 2G)
    pub min_filesize: String,
    pub max_filesize: String,
    /// 1回の実行で落とす最大本数。0 なら無制限
    pub max_downloads: u32,
    /// プレイリストの範囲(例: 1-10,15)
    pub playlist_items: String,
    /// yt-dlp の --match-filter をそのまま指定
    pub match_filter: String,

    // ── 追加で保存するもの ──
    pub write_thumbnail_file: bool,
    pub write_subtitle_file: bool,
    /// 自動生成字幕も対象にする
    pub auto_subtitles: bool,
    pub write_description: bool,
    pub write_info_json: bool,
    /// チャプターごとにファイルを分割
    pub split_chapters: bool,
    /// SponsorBlock で除去する区間(カンマ区切り)
    pub sponsorblock_categories: String,

    // ── ファイル名 ──
    /// 英数字だけのファイル名にする
    pub restrict_filenames: bool,
    /// 作業中ファイルを置くフォルダ。空なら保存先と同じ
    pub temp_dir: String,
    /// 重複防止の記録を使う
    pub use_archive: bool,

    // ── 通信 ──
    pub proxy: String,
    pub retries: u32,
    /// ダウンロード間の待ち時間(秒)。BAN対策
    pub sleep_seconds: u32,
    pub user_agent: String,
    /// 地域制限の回避を試みる
    pub geo_bypass: bool,

    // ── ライブ配信 ──
    /// 配信の最初から録画する
    pub live_from_start: bool,
    /// 開始前の配信を待つ
    pub wait_for_live: bool,

    // ── さらに詳細 ──
    /// cookies.txt の場所(ブラウザから取れない時用)
    pub cookies_file: String,
    /// ブラウザ偽装: auto(失敗時だけ) / off / chrome / safari / edge / firefox
    pub impersonate: String,
    /// 字幕の変換先: 空(そのまま) / srt / vtt / ass / lrc
    pub subtitle_format: String,
    /// サムネイルの変換先: 空(そのまま) / jpg / png / webp
    pub convert_thumbnail: String,
    /// 動画の再エンコード先: 空(しない) / mp4 / mkv / webm / mov
    pub recode_video: String,
    /// 一部だけ保存(例: *00:01:00-00:02:30)
    pub download_sections: String,
    pub force_ipv4: bool,
    /// 通信のタイムアウト(秒)。0 なら既定
    pub socket_timeout: u32,
    /// 同じ画質ならフリーな形式(WebM等)を優先
    pub prefer_free_formats: bool,
    /// .part を使わず直接書き込む
    pub no_part: bool,

    // ── 追加で保存するもの(充実版) ──
    /// コメントを保存(動画情報 JSON に入る)
    pub write_comments: bool,
    /// ライブのチャットを保存
    pub write_live_chat: bool,
    /// 元ページへのショートカット(.url 等)を保存
    pub write_url_link: bool,
    /// 動画情報を動画ファイルに埋め込む(MKV)
    pub embed_info_json: bool,
    /// サムネイルを全種類保存
    pub write_all_thumbnails: bool,
    /// プレイリスト自体の情報・サムネイルも保存
    pub write_playlist_files: bool,

    // ── 地域・VR ──
    /// 接続元として扱う国(例: JP / US)。空なら指定しない
    pub region: String,
    /// タイトル・サムネイルなどを取る言語(例: ja / en)。空ならサイト任せ
    pub metadata_lang: String,
    /// VR・360°動画向け(高解像度を優先し MKV で保存)
    pub vr_mode: bool,
    /// サムネイルだけ保存する時に使う画像の URL(1件ごとの指定でだけ使う)
    pub thumbnail_url: String,

    // ── 常駐・通知・予約・更新 ──
    /// コピーした URL を見つけたら知らせる
    pub watch_clipboard: bool,
    /// ウィンドウを閉じてもタスクトレイに残って動き続ける
    pub close_to_tray: bool,
    /// 保存が終わったら通知する
    pub notify_done: bool,
    /// アプリ本体の新しい版を自動で確認する
    pub auto_update_app: bool,
    /// この時間帯だけダウンロードを始める(例: 01:00-06:00)。空ならいつでも
    pub active_hours: String,

    // ── 追加機能 ──
    /// 字幕を映像に焼き付ける
    pub burn_subtitles: bool,
    /// 保存先の空きがこれ(GB)を下回ったら新しいダウンロードを始めない。0 で無効
    pub min_free_gb: u64,
    /// 省電力: auto(バッテリーの時だけ) / on(常に) / off
    pub power_saver: String,
    /// 保存済みの動画をもう一度保存しようとした時: ask(確認) / skip(飛ばす) / allow(保存する)
    pub on_duplicate: String,
    /// 保存先に同じ名前のファイルがあった時: skip(飛ばす) / overwrite(上書き) / rename(番号を付けて別名で保存)
    pub on_same_name: String,
    /// 回線のエラーで失敗した時に、時間をおいて自動でやり直す回数(0 でしない)
    pub auto_retry_count: u32,
    /// サイトごとにうまくいった取り方を覚えて、次から最初に使う
    pub remember_methods: bool,
    /// 保存した後に、ファイルが壊れていないか調べる
    pub verify_files: bool,
    /// 待っているものを始める順番: added(追加した順) / small(小さい順) / large(大きい順)
    pub queue_order: String,
    /// プレイリストを落とす順番: ""(サイトの並び) / reverse(逆順)
    pub playlist_order: String,
    /// 保存日と元のページをファイルの情報に入れる
    pub embed_source_info: bool,
    /// 自動生成字幕の重なり・細切れの行を整える
    pub clean_auto_subs: bool,
    /// 動画の縦横を合わせる(黒帯を足す): "" / "16:9" / "9:16"
    pub pad_aspect: String,
    /// 音楽の曲名・アーティスト名をネットのデータベース(MusicBrainz)で補う
    pub music_tags: bool,
    /// 歌詞が見つかれば音楽ファイルに入れる(LRCLIB)
    pub embed_lyrics: bool,
    /// スマホへの通知・スマホからの受け取りに使う ntfy のトピック(空なら使わない)
    pub ntfy_topic: String,
    /// ntfy のサーバー
    pub ntfy_server: String,
    /// 完了をスマホへ通知する
    pub ntfy_notify: bool,
    /// スマホから送った URL を受け取る
    pub ntfy_receive: bool,
    /// ダウンロード機能を自動で更新しない(今の版のまま使う)
    pub pin_ytdlp: bool,
    /// 速さ優先: 1本を細かく分けて同時にたくさん取る
    pub fast_mode: bool,
    /// 音声の言語(例: ja)。空ならサイトの標準
    pub audio_language: String,
    /// 音声トラックを全部入れて保存する(MKV になる)
    pub all_audio_tracks: bool,
    /// 焼き付ける字幕の文字の大きさ(0 で標準)
    pub burn_font_size: u32,
    /// 焼き付ける字幕の色: "" (白) / yellow / cyan
    pub burn_color: String,
    /// 焼き付ける字幕の位置: bottom / top
    pub burn_position: String,
    /// 字幕を翻訳して取る言語(YouTube の自動翻訳。空ならしない)
    pub subtitle_translate_to: String,
    /// 音楽のテンポ(BPM)とキーを調べて情報に入れる
    pub music_bpm_key: bool,
    /// プレイリストの曲に番号を振る
    pub music_track_numbers: bool,
    /// 記録を残さない(ライブラリ・ログ・覚えた取り方・統計)
    pub private_mode: bool,
    /// ログイン情報をこの日数使わなかったら自動で消す(0 で消さない)
    pub login_expire_days: u32,
    /// 字幕のよくある間違いの直し方(1行に「間違い→正しい」)
    pub subtitle_fixes: String,
    /// プロキシだけを通す(プロキシが無い時はつながない)
    pub proxy_only: bool,
    /// 部品(GitHub の配布物)を取る時の代わりの入口。空なら GitHub から直接
    pub github_mirror: String,
    /// 設定・記録を暗号化して保存する(Windows)
    pub encrypt_records: bool,
    /// ログイン情報の画面を開く合言葉(ハッシュ。空なら合言葉なし)
    pub login_password_hash: String,

    /// 何を保存するか: video / audio / thumbnail / subtitles / info
    pub mode: String,
    /// 音声として保存する時の形式: mp3 / m4a / opus / flac / wav
    pub audio_format: String,
    /// ブラウザのログイン情報を使う場面: needed(ログインが必要な時だけ) / always
    pub cookies_when: String,
    /// 保存先の自動振り分け。上から順に調べて、最初に当てはまったフォルダへ保存する
    pub folder_rules: Vec<FolderRule>,
    /// 地域制限で失敗した時だけ使うプロキシ(空なら使わない)
    pub fallback_proxy: String,

    /// 画面の見た目・言語・表示項目。画面側(ui/app.js の UI_DEFAULTS)だけが使う
    pub ui: serde_json::Map<String, serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        let output_dir = dirs::video_dir()
            .or_else(dirs::download_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Grabbit");
        Self {
            output_dir: output_dir.to_string_lossy().into_owned(),
            preset: "best".into(),
            container: "mp4".into(),
            filename_template: "%(uploader,channel|Unknown)s/%(title).150B.%(ext)s".into(),
            embed_thumbnail: true,
            embed_metadata: true,
            embed_subtitles: false,
            subtitle_langs: "ja,en".into(),
            sponsorblock_remove: false,
            cookies_browser: String::new(),
            max_parallel: 2,
            concurrent_fragments: 4,
            rate_limit: String::new(),
            check_interval_minutes: 60,
            extra_args: String::new(),
            update_channel: "nightly".into(),
            tool_repos: Default::default(),
            custom_repos: Vec::new(),
            disabled_tools: Vec::new(),
            video_codec: "any".into(),
            audio_bitrate: String::new(),
            keep_original: false,
            skip_shorts: false,
            skip_live: false,
            date_after: String::new(),
            min_filesize: String::new(),
            max_filesize: String::new(),
            max_downloads: 0,
            playlist_items: String::new(),
            match_filter: String::new(),
            write_thumbnail_file: false,
            write_subtitle_file: false,
            auto_subtitles: false,
            write_description: false,
            write_info_json: false,
            split_chapters: false,
            sponsorblock_categories: "sponsor,selfpromo,interaction".into(),
            restrict_filenames: false,
            temp_dir: String::new(),
            use_archive: true,
            proxy: String::new(),
            retries: 10,
            sleep_seconds: 0,
            user_agent: String::new(),
            geo_bypass: false,
            live_from_start: false,
            wait_for_live: false,
            cookies_file: String::new(),
            impersonate: "auto".into(),
            subtitle_format: String::new(),
            convert_thumbnail: "jpg".into(),
            recode_video: String::new(),
            download_sections: String::new(),
            force_ipv4: false,
            socket_timeout: 0,
            prefer_free_formats: false,
            no_part: false,
            write_comments: false,
            write_live_chat: false,
            write_url_link: false,
            embed_info_json: false,
            write_all_thumbnails: false,
            write_playlist_files: false,
            region: String::new(),
            metadata_lang: String::new(),
            vr_mode: false,
            thumbnail_url: String::new(),
            watch_clipboard: true,
            close_to_tray: true,
            notify_done: true,
            auto_update_app: true,
            active_hours: String::new(),
            burn_subtitles: false,
            min_free_gb: 5,
            power_saver: "auto".into(),
            on_duplicate: "ask".into(),
            mode: "video".into(),
            audio_format: "mp3".into(),
            cookies_when: "needed".into(),
            folder_rules: Vec::new(),
            on_same_name: "skip".into(),
            auto_retry_count: 3,
            remember_methods: true,
            verify_files: true,
            queue_order: "added".into(),
            playlist_order: String::new(),
            embed_source_info: true,
            clean_auto_subs: true,
            pad_aspect: String::new(),
            music_tags: false,
            embed_lyrics: false,
            ntfy_topic: String::new(),
            ntfy_server: "https://ntfy.sh".into(),
            ntfy_notify: false,
            ntfy_receive: false,
            pin_ytdlp: false,
            fast_mode: false,
            audio_language: String::new(),
            all_audio_tracks: false,
            burn_font_size: 0,
            burn_color: String::new(),
            burn_position: "bottom".into(),
            subtitle_translate_to: String::new(),
            music_bpm_key: false,
            music_track_numbers: true,
            private_mode: false,
            login_expire_days: 0,
            subtitle_fixes: String::new(),
            proxy_only: false,
            github_mirror: String::new(),
            encrypt_records: false,
            login_password_hash: String::new(),
            fallback_proxy: String::new(),
            ui: serde_json::Map::new(),
        }
    }
}

/// 保存先の振り分けルール。kind と site が空なら「何でも」。
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct FolderRule {
    /// 保存するもの: video / audio / thumbnail / subtitles / info(空ならすべて)
    pub kind: String,
    /// サイト名の一部(例: youtube, nicovideo)。空ならすべて
    pub site: String,
    /// 保存先。相対パス(例: 音楽)なら、いつもの保存先の中に作る
    pub folder: String,
}

impl Settings {
    /// 振り分けルールに当てはまれば、その保存先にする。当てはまらなければ今のまま
    pub fn route_output_dir(&mut self, url: &str) {
        let host = url.split('/').nth(2).unwrap_or_default().to_lowercase();
        let rule = self.folder_rules.iter().find(|r| {
            !r.folder.trim().is_empty()
                && (r.kind.is_empty() || r.kind == self.mode)
                && (r.site.trim().is_empty() || host.contains(&r.site.trim().to_lowercase()))
        });
        if let Some(rule) = rule {
            let folder = Path::new(rule.folder.trim());
            self.output_dir = if folder.is_absolute() {
                folder.to_string_lossy().into_owned()
            } else {
                Path::new(&self.output_dir).join(folder).to_string_lossy().into_owned()
            };
        }
    }
}

/// 自動で新着をダウンロードするチャンネル/プレイリスト。
#[derive(Serialize, Deserialize, Clone)]
pub struct Subscription {
    pub id: u64,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    /// 最後に確認した時刻(UNIX秒)
    pub last_checked: u64,
    /// この登録先だけの保存方法(画質・形式など)。設定より優先する
    #[serde(default)]
    pub overrides: serde_json::Map<String, serde_json::Value>,
    /// 配信が始まったら自動で録画する
    #[serde(default)]
    pub record_live: bool,
    /// まとめるためのグループ名(空なら「未分類」)
    #[serde(default)]
    pub group: String,
    /// この登録先だけの確認間隔(分)。0 なら設定の値
    #[serde(default)]
    pub interval_minutes: u64,
    /// 新着を知らせるだけで、保存は手動にする
    #[serde(default)]
    pub notify_only: bool,
    /// 取り方: ""(動画用) / gallery(画像・SNS 用。X・Instagram などの投稿)
    #[serde(default)]
    pub engine: String,
    /// 「知らせるだけ」の時に、もう知らせた動画の ID
    #[serde(default)]
    pub seen: Vec<String>,
    /// 新着が見つかった時間帯(0〜23 時の回数)。よく見つかる時間は確認を増やす
    #[serde(default)]
    pub found_hours: Vec<u32>,
    /// 名前(チャンネル名など)を最後に確かめた時刻
    #[serde(default)]
    pub name_checked: u64,
    /// 決まった時刻に確認する(例: "07:00"。空なら間隔で確認)
    #[serde(default)]
    pub check_times: Vec<String>,
    /// 時刻で確認した日(同じ日に二度確認しないため。"日付 時刻" の形)
    #[serde(default)]
    pub checked_slots: Vec<String>,
}

/// SNS など、動画用ではなく画像・投稿用の方法で自動保存するサイトか
pub fn is_gallery_site(url: &str) -> bool {
    let host = url.split('/').nth(2).unwrap_or_default().to_lowercase();
    ["x.com", "twitter.com", "instagram.com", "pixiv.net", "bsky.app", "tumblr.com", "deviantart.com", "misskey.io", "pawoo.net", "fanbox.cc", "kemono.su"]
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

impl Settings {
    /// 1件ごとの指定(画質・ファイル名・字幕など)で上書きした設定を作る。
    /// 知らない項目や型が合わない値は無視して、元の設定を使う。
    pub fn with_overrides(&self, overrides: &serde_json::Map<String, serde_json::Value>) -> Settings {
        if overrides.is_empty() {
            return self.clone();
        }
        let mut merged = match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => return self.clone(),
        };
        for (key, value) in overrides {
            if key != "ui" && merged.contains_key(key) {
                merged.insert(key.clone(), value.clone());
            }
        }
        serde_json::from_value(serde_json::Value::Object(merged)).unwrap_or_else(|_| self.clone())
    }
}

/// 今の時刻が「ダウンロードしてよい時間帯」に入っているか。
/// `hours` は "01:00-06:00" の形。日をまたぐ指定("22:00-06:00")もできる。空や読めない値ならいつでも可。
pub fn within_active_hours(hours: &str, now_minutes: u32) -> bool {
    let parse = |text: &str| -> Option<u32> {
        let (h, m) = text.trim().split_once(':')?;
        Some(h.parse::<u32>().ok()? * 60 + m.parse::<u32>().ok()?)
    };
    let Some((start, end)) = hours.split_once('-').and_then(|(a, b)| Some((parse(a)?, parse(b)?))) else {
        return true;
    };
    if start <= end {
        (start..end).contains(&now_minutes)
    } else {
        now_minutes >= start || now_minutes < end
    }
}

/// 壊れていた設定ファイルを、控えから戻したか(画面で知らせる用)
pub static RESTORED_FROM_BACKUP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 読み込む。ファイルが壊れていたら、前回の控え(.bak)から戻す
pub fn load_json<T: DeserializeOwned + Default>(name: &str) -> T {
    let path = data_dir().join(name);
    let read = |p: &Path| std::fs::read(p).ok().and_then(|bytes| crate::sealed::open(&bytes)).and_then(|text| serde_json::from_str::<T>(&text).ok());
    if let Some(value) = read(&path) {
        return value;
    }
    let backup = path.with_extension("json.bak");
    if path.exists() {
        if let Some(value) = read(&backup) {
            RESTORED_FROM_BACKUP.store(true, std::sync::atomic::Ordering::SeqCst);
            return value;
        }
    }
    T::default()
}

pub fn save_json<T: Serialize>(name: &str, value: &T) {
    let path: PathBuf = data_dir().join(name);
    // 今のファイルが正しく読める時だけ、控えとして残す(壊れた物で控えを上書きしない)
    if let Ok(bytes) = std::fs::read(&path) {
        if crate::sealed::open(&bytes).is_some_and(|text| serde_json::from_str::<serde_json::Value>(&text).is_ok()) {
            let _ = std::fs::write(path.with_extension("json.bak"), &bytes);
        }
    }
    // 「記録を暗号化する」設定なら暗号化して書く(読む時は印を見て自動で戻す)
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    write_atomic_bytes(&path, &crate::sealed::seal(&text));
}

/// 書き込み途中で落ちても壊れないよう、一時ファイル経由で保存する。
fn write_atomic_bytes(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_dir_follows_rules() {
        let mut s = Settings { output_dir: "/v".into(), mode: "audio".into(), ..Settings::default() };
        s.folder_rules = vec![
            FolderRule { kind: "video".into(), site: "".into(), folder: "動画".into() },
            FolderRule { kind: "audio".into(), site: "soundcloud".into(), folder: "/music/sc".into() },
            FolderRule { kind: "audio".into(), site: "".into(), folder: "音楽".into() },
        ];
        let mut a = s.clone();
        a.route_output_dir("https://www.youtube.com/watch?v=x");
        assert_eq!(a.output_dir, Path::new("/v").join("音楽").to_string_lossy());
        let mut b = s.clone();
        b.route_output_dir("https://soundcloud.com/x");
        assert_eq!(b.output_dir, "/music/sc");
    }

    #[test]
    fn active_hours_work_across_midnight() {
        assert!(within_active_hours("", 0));
        assert!(within_active_hours("01:00-06:00", 3 * 60));
        assert!(!within_active_hours("01:00-06:00", 7 * 60));
        assert!(within_active_hours("22:00-06:00", 23 * 60));
        assert!(within_active_hours("22:00-06:00", 5 * 60));
        assert!(!within_active_hours("22:00-06:00", 12 * 60));
    }

    #[test]
    fn overrides_replace_only_known_fields() {
        let mut overrides = serde_json::Map::new();
        overrides.insert("mode".into(), "thumbnail".into());
        overrides.insert("unknown".into(), 1.into());
        let merged = Settings::default().with_overrides(&overrides);
        assert_eq!(merged.mode, "thumbnail");
        assert_eq!(merged.container, Settings::default().container);
    }
}
