// Grabbit の画面処理。バックエンド(Rust)のコマンドを呼び、届いたイベントで表示を更新する。
// 文言はすべて t(日本語の原文) を通す(i18n.js が選択中の言語へ置き換える)。

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
const { t, applyTranslations, LANGUAGES, resolveLanguage } = window.I18N;

let settings = {};
const jobs = new Map(); // id → job

// ───────── 見た目・表示項目の初期値(settings.ui に保存される) ─────────
const UI_DEFAULTS = {
  language: "auto",
  theme: "system",
  accent: "#3b6cf6",
  font_size: 14,
  list_style: "list",
  primary_text: "title",
  error_style: "friendly",
  show_thumbnail: true,
  show_favicon: true,
  show_url: false,
  show_path: true,
  show_speed: true,
  show_engine: false,
  show_site_strip: false,
  collapse_done: true,
  show_nav_labels: true,
  auto_relogin: true,
  split_view: false,
  reduce_motion: false,
  sound: "off",
  custom_css: "",
  auto_analyze: true,
  nav_order: [],
  nav_hidden: [],
};
const ui = (key) => settings.ui?.[key] ?? UI_DEFAULTS[key];

const STATUS = {
  waiting: { icon: "…", text: "予定" },
  running: { icon: "⬇", text: "ダウンロード中" },
  done: { icon: "✓", text: "完了" },
  failed: { icon: "✕", text: "失敗" },
  canceled: { icon: "–", text: "キャンセル" },
  paused: { icon: "||", text: "一時停止中" },
};

const PRESETS = [
  ["best", "おまかせ最高画質"],
  ["2160", "4K まで"],
  ["1080", "1080p まで"],
  ["720", "720p まで"],
  ["audio_mp3", "音声のみ MP3"],
  ["audio_m4a", "音声のみ M4A"],
];

// 保存するものの種類(バックエンドの settings.mode)
const MODES = {
  video: "動画",
  audio: "音声だけ",
  thumbnail: "サムネイルだけ",
  subtitles: "字幕だけ",
  info: "概要・情報だけ",
};

// サービスごとに「できること」。選ぶとその内容だけを出す
const SERVICES = [
  { id: "auto", name: "自動判別" },
  { id: "youtube", name: "YouTube", domain: "youtube.com", match: /youtube\.com|youtu\.be/,
    example: "https://www.youtube.com/watch?v=…", modes: ["video", "audio", "thumbnail", "subtitles", "info"],
    subscribe: "https://www.youtube.com/@チャンネル名", live: true, shorts: true },
  { id: "niconico", name: "ニコニコ", domain: "nicovideo.jp", match: /nicovideo\.jp|nico\.ms/,
    example: "https://www.nicovideo.jp/watch/sm…", modes: ["video", "audio", "thumbnail", "subtitles", "info"],
    subscribe: "https://www.nicovideo.jp/user/…", live: true },
  { id: "x", name: "X", domain: "x.com", match: /(^|\.)x\.com|twitter\.com/,
    example: "https://x.com/…/status/…", modes: ["video", "audio", "thumbnail", "info"], images: true },
  { id: "tiktok", name: "TikTok", domain: "tiktok.com", match: /tiktok\.com/,
    example: "https://www.tiktok.com/@…/video/…", modes: ["video", "audio", "thumbnail", "info"],
    subscribe: "https://www.tiktok.com/@ユーザー名" },
  { id: "instagram", name: "Instagram", domain: "instagram.com", match: /instagram\.com/,
    example: "https://www.instagram.com/reel/…", modes: ["video", "audio", "thumbnail", "info"], images: true },
  { id: "twitch", name: "Twitch", domain: "twitch.tv", match: /twitch\.tv/,
    example: "https://www.twitch.tv/videos/…", modes: ["video", "audio", "thumbnail", "info"],
    subscribe: "https://www.twitch.tv/チャンネル名/videos", live: true },
  { id: "bilibili", name: "bilibili", domain: "bilibili.com", match: /bilibili\.com|b23\.tv/,
    example: "https://www.bilibili.com/video/BV…", modes: ["video", "audio", "thumbnail", "subtitles", "info"],
    subscribe: "https://space.bilibili.com/…" },
  { id: "soundcloud", name: "SoundCloud", domain: "soundcloud.com", match: /soundcloud\.com/,
    example: "https://soundcloud.com/…/…", modes: ["audio", "thumbnail", "info"],
    subscribe: "https://soundcloud.com/ユーザー名" },
  { id: "pixiv", name: "pixiv", domain: "pixiv.net", match: /pixiv\.net/,
    example: "https://www.pixiv.net/artworks/…", modes: ["video"], images: true, imagesOnly: true },
  { id: "facebook", name: "Facebook", domain: "facebook.com", match: /facebook\.com|fb\.watch/,
    example: "https://www.facebook.com/watch/?v=…", modes: ["video", "audio", "thumbnail", "info"] },
  { id: "reddit", name: "Reddit", domain: "reddit.com", match: /reddit\.com|redd\.it/,
    example: "https://www.reddit.com/r/…/comments/…", modes: ["video", "audio", "thumbnail", "info"], images: true },
  { id: "bluesky", name: "Bluesky", domain: "bsky.app", match: /bsky\.app/,
    example: "https://bsky.app/profile/…/post/…", modes: ["video", "audio", "thumbnail", "info"], images: true },
  { id: "vimeo", name: "Vimeo", domain: "vimeo.com", match: /vimeo\.com/,
    example: "https://vimeo.com/…", modes: ["video", "audio", "thumbnail", "subtitles", "info"],
    subscribe: "https://vimeo.com/ユーザー名" },
  { id: "dailymotion", name: "Dailymotion", domain: "dailymotion.com", match: /dailymotion\.com|dai\.ly/,
    example: "https://www.dailymotion.com/video/…", modes: ["video", "audio", "thumbnail", "subtitles", "info"] },
  { id: "kick", name: "Kick", domain: "kick.com", match: /kick\.com/,
    example: "https://kick.com/…", modes: ["video", "audio", "thumbnail", "info"], live: true,
    subscribe: "https://kick.com/チャンネル名/videos" },
  { id: "twitcasting", name: "ツイキャス", domain: "twitcasting.tv", match: /twitcasting\.tv/,
    example: "https://twitcasting.tv/…/movie/…", modes: ["video", "audio", "thumbnail", "info"], live: true },
  { id: "abema", name: "ABEMA", domain: "abema.tv", match: /abema\.tv/,
    example: "https://abema.tv/video/episode/…", modes: ["video", "thumbnail", "info"] },
  { id: "tver", name: "TVer", domain: "tver.jp", match: /tver\.jp/,
    example: "https://tver.jp/episodes/…", modes: ["video", "thumbnail", "subtitles", "info"] },
  { id: "bandcamp", name: "Bandcamp", domain: "bandcamp.com", match: /bandcamp\.com/,
    example: "https://….bandcamp.com/track/…", modes: ["audio", "thumbnail", "info"] },
  { id: "mixcloud", name: "Mixcloud", domain: "mixcloud.com", match: /mixcloud\.com/,
    example: "https://www.mixcloud.com/…/…", modes: ["audio", "thumbnail", "info"] },
  { id: "weibo", name: "Weibo", domain: "weibo.com", match: /weibo\.(com|cn)/,
    example: "https://weibo.com/…", modes: ["video", "thumbnail", "info"], images: true },
  { id: "pinterest", name: "Pinterest", domain: "pinterest.com", match: /pinterest\.|pin\.it/,
    example: "https://www.pinterest.com/pin/…", modes: ["video", "thumbnail", "info"], images: true },
  { id: "tumblr", name: "Tumblr", domain: "tumblr.com", match: /tumblr\.com/,
    example: "https://….tumblr.com/post/…", modes: ["video", "audio", "thumbnail", "info"], images: true },
  { id: "imgur", name: "Imgur", domain: "imgur.com", match: /imgur\.com/,
    example: "https://imgur.com/…", modes: ["video", "thumbnail"], images: true },
  { id: "rumble", name: "Rumble", domain: "rumble.com", match: /rumble\.com/,
    example: "https://rumble.com/…", modes: ["video", "audio", "thumbnail", "info"], live: true },
  { id: "odysee", name: "Odysee", domain: "odysee.com", match: /odysee\.com/,
    example: "https://odysee.com/@…/…", modes: ["video", "audio", "thumbnail", "info"],
    subscribe: "https://odysee.com/@チャンネル名" },
  { id: "vk", name: "VK", domain: "vk.com", match: /vk\.com|vkvideo\.ru/,
    example: "https://vk.com/video…", modes: ["video", "audio", "thumbnail", "info"] },
  { id: "douyin", name: "抖音", domain: "douyin.com", match: /douyin\.com/,
    example: "https://www.douyin.com/video/…", modes: ["video", "audio", "thumbnail", "info"] },
  { id: "showroom", name: "SHOWROOM", domain: "showroom-live.com", match: /showroom-live\.com/,
    example: "https://www.showroom-live.com/r/…", modes: ["video", "audio"], live: true },
  { id: "17live", name: "17LIVE", domain: "17.live", match: /17\.live/,
    example: "https://17.live/…", modes: ["video", "audio"], live: true },
  { id: "nhk", name: "NHK", domain: "nhk.or.jp", match: /nhk\.(or\.jp|jp)/,
    example: "https://www3.nhk.or.jp/…", modes: ["video", "audio", "thumbnail", "subtitles", "info"] },
  { id: "streamable", name: "Streamable", domain: "streamable.com", match: /streamable\.com/,
    example: "https://streamable.com/…", modes: ["video", "audio", "thumbnail"] },
  { id: "archive", name: "Internet Archive", domain: "archive.org", match: /archive\.org/,
    example: "https://archive.org/details/…", modes: ["video", "audio", "thumbnail", "info"] },
  { id: "patreon", name: "Patreon", domain: "patreon.com", match: /patreon\.com/,
    example: "https://www.patreon.com/posts/…", modes: ["video", "audio", "thumbnail", "info"] },
  { id: "other", name: "その他のサイト", example: "https://…", modes: ["video", "audio", "thumbnail", "subtitles", "info"], images: true },
];
const serviceById = (id) => SERVICES.find((s) => s.id === id);
const detectService = (url) => SERVICES.find((s) => s.match?.test(hostOf(url))) || serviceById("other");
const selectedService = "auto"; // サービスは常に自動判別

// 利用者の多い順(世界のアクセス数の目安)。アイコン列と対応サイト一覧の並びに使う
const POPULAR_SITES = [
  "youtube.com", "facebook.com", "instagram.com", "x.com", "tiktok.com", "reddit.com", "bilibili.com",
  "twitch.tv", "pinterest.com", "vk.com", "nicovideo.jp", "dailymotion.com", "vimeo.com", "soundcloud.com",
  "pixiv.net", "tumblr.com", "bsky.app", "imgur.com", "rumble.com", "kick.com",
];
// 対応サイト一覧の並び(抽出器の名前)。ここに無いものはこの後に名前順で並ぶ
const POPULAR_EXTRACTORS = [
  "youtube", "facebook", "instagram", "twitter", "tiktok", "reddit", "bilibili", "twitch", "pinterest", "vk",
  "dailymotion", "vimeo", "soundcloud", "linkedin", "niconico", "weibo", "douyin", "imgur", "tumblr", "bluesky",
  "rumble", "kick", "pornhub", "xvideos", "xhamster", "abematv", "tver", "nhk", "bbc", "cnn", "ted", "archive.org",
  "bandcamp", "mixcloud", "spotify", "applepodcasts", "odysee", "streamable", "patreon", "twitcasting", "showroom",
  "17live", "iqiyi", "youku", "naver", "afreecatv", "soop", "zoom", "loom", "googledrive", "dropbox",
];
function popularityRank(name) {
  const base = name.toLowerCase().split(":")[0];
  const index = POPULAR_EXTRACTORS.findIndex((p) => base === p || base.startsWith(p));
  return index === -1 ? Infinity : index;
}

// バックエンドが "@..." で知らせてくる状況(各言語に訳して出す)
const BACKEND_MESSAGES = {
  "@skipped": "保存済みのためスキップ",
  "@retry_without_cookies": "ブラウザのログイン情報を読めないため、ログインなしで再挑戦中…",
  "@retry_impersonate": "ブラウザ偽装で再挑戦中…",
  "@retry_proxy": "地域制限のため、予備のプロキシで再挑戦中…",
  "@auto_retry": "回線のエラーで失敗したため、少し待ってから自動でやり直します",
  "@verifying": "保存したファイルが壊れていないか調べています…",
  "@broken_file": "保存したファイルが壊れていました",
  "@retry_gallery": "画像・ギャラリー用の方法で再挑戦中…",
  "@retry_direct": "ファイルとして直接保存を試しています…",
  "@done_without_cookies": "ログイン情報なしで保存しました",
  "@burning": "字幕を映像に焼き付けています…",
  "@live_recording": "配信を録画中",
  "@nothing_saved": "ファイルが保存されませんでした。「記録」で詳しい理由を確認できます",
  "@repairing": "ダウンロードの部品を直してもう一度試しています…",
  "@no_subtitles_to_burn": "この動画には選んだ言語の字幕が無かったので、字幕なしで保存しました",
  "@paused": "一時停止中(再開すると続きから)",
  "@retry_safe_name": "ファイル名が使えなかったため、短い名前で保存し直しています…",
  "@retry_with_cookies": "ログインが必要な内容のため、ブラウザのログイン情報で再挑戦中…",
  "@retry_without_thumbnail": "サムネイルが取れなかったため、サムネイルなしで保存し直しています…",
  "@quarantined": "開くとプログラムとして動くファイルだったため、安全のため隔離フォルダへ移しました",
  "@proxy_required": "「プロキシだけを通す」がオンですが、プロキシが入っていないため始めませんでした",
  "@not_enough_space": "保存先の空きが足りないため始めませんでした(必要な大きさの目安)",
  "@new_item": "新着です。保存するには「再開」を押してください",
  "@padding": "縦横を合わせています…",
  "@pad_failed": "縦横を合わせられませんでした",
  "@music_info": "曲の情報と歌詞を探しています…",
  "@needs_login": "有料・会員限定などログインが必要な内容です。設定の「ログインが必要な動画用」でブラウザを選んでください。",
};

// 取得方法の表示名(ツール名ではなく何で取ったかを出す)
const ENGINE_NAMES = { "yt-dlp": "動画", "gallery-dl": "画像", direct: "直接" };

// よくあるエラーを、何が起きて何をすればいいかの説明に変える
// 原文の決まった言い回しだけを見て判定する(「age」が「page」に当たる等の食い違いを防ぐため具体的に書く)
// 上から順に調べ、最初に当たったものを使う
const ERROR_EXPLANATIONS = [
  [/could not copy .*cookie|cookie database|failed to decrypt|could not find .*cookies|DPAPI/i,
    "ブラウザのログイン情報を読めませんでした。Chrome / Edge を閉じてから試すか、設定で Firefox か cookies.txt を使ってください。"],
  [/members[- ]only|join this channel|available to this channel's members|requires payment|purchase|premium/i,
    "有料・会員限定などログインが必要な内容です。設定の「ログインが必要な動画用」でブラウザを選んでください。"],
  [/private video|this video is private/i, "非公開の動画です。見られるアカウントのログイン情報が必要です。"],
  [/confirm your age|age[- ]restricted|inappropriate for some users/i, "年齢制限があります。ログイン情報が必要です。"],
  [/sign in to confirm you.re not a bot|not a bot/i, "サイトにロボットと判定されました。しばらく待つか、ログイン情報を使ってください。"],
  [/sign in|login required|requires? (a )?login|--cookies-from-browser or --cookies|use --cookies/i,
    "ログインが必要です。設定の「ログインが必要な動画用」でブラウザを選んでください。"],
  [/live event will begin|premieres in|this live event|is_upcoming|will begin in/i,
    "まだ始まっていない配信です。設定の「開始前の配信は始まるまで待つ」で待てます。"],
  [/video unavailable|has been removed|no longer available|account .*terminated|been deleted/i,
    "動画が削除されたか、見られなくなっています。"],
  [/not available in your country|blocked it in your country|geo[- ]?restrict|not available in your region/i,
    "お住まいの地域では見られない動画です。"],
  [/\bDRM\b/, "DRM(コピー防止)付きのため保存できません。"],
  [/Unsupported URL|gallery-dl でも/, "この URL からは動画や画像が見つかりませんでした。"],
  [/HTTP Error 429|Too Many Requests/i, "アクセスが多すぎて制限されています。しばらく待つか、待ち時間を設定してください。"],
  [/HTTP Error 403|\bForbidden\b/i, "サイトに拒否されました。ブラウザ偽装やログイン情報で通ることがあります。"],
  [/HTTP Error 404|\b404\b.*not found/i, "ページが見つかりません。URL を確認してください。"],
  [/unable to connect|getaddrinfo|name or service not known|network is unreachable|connection refused|No address associated/i,
    "インターネットに接続できません。回線を確認してください。"],
  [/timed out|TimeoutError/i, "通信がタイムアウトしました。回線を確認してもう一度試してください。"],
  [/No space left|disk full|Errno 28/i, "保存先の空き容量が足りません。"],
  [/Permission denied|Access is denied|Errno 13/i, "保存先に書き込めません。保存先を変えてください。"],
  [/Errno 22|Invalid argument|WinError 123|WinError 206|File name too long/i,
    "ファイル名か保存先のパスが使えませんでした。保存先を短い場所にするか、「ファイル名を英数字だけにする」を試してください。"],
  [/Postprocessing|ffmpeg (not found|exited)|Conversion failed/i, "動画の結合・変換に失敗しました。"],
];

function describeMessage(message) {
  if (!message) return "";
  if (BACKEND_MESSAGES[message]) return t(BACKEND_MESSAGES[message]);
  // "@broken_file 詳しい理由" のように、合図の後ろに詳細が付くもの
  const [key, ...rest] = message.split(" ");
  if (key.startsWith("@") && BACKEND_MESSAGES[key]) return `${t(BACKEND_MESSAGES[key])}(${rest.join(" ")})`;
  const friendly = ERROR_EXPLANATIONS.find(([pattern]) => pattern.test(message));
  const style = ui("error_style");
  if (style === "raw" || !friendly) return message;
  if (style === "both") return `${t(friendly[1])}(${message})`;
  return t(friendly[1]);
}

// ───────── 小物 ─────────
function hostOf(url) {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return "";
  }
}

function faviconOf(urlOrHost) {
  const host = urlOrHost.includes("://") ? hostOf(urlOrHost) : urlOrHost;
  return host ? `https://www.google.com/s2/favicons?domain=${host}&sz=64` : "";
}

function fileName(path) {
  return path.split(/[\\/]/).pop();
}

function formatDuration(seconds) {
  if (!seconds) return "";
  const s = Math.round(seconds);
  const pad = (n) => String(n).padStart(2, "0");
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h ? `${h}:${pad(m)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
}

function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  // フォームの中に置いても送信(=画面の読み込み直し)にならないよう、ボタンは普通のボタンにする
  if (tag === "button") node.type = "button";
  for (const [key, value] of Object.entries(props)) {
    // aria-label などの「-」が入る名前と role は属性として付ける(読み上げソフト用)
    if (key.includes("-") || key === "role") node.setAttribute(key, value);
    else node[key] = value;
  }
  node.append(...children.filter(Boolean));
  return node;
}

/** 記号だけのボタン(意味はマウスを載せると出る) */
function iconButton(symbol, label, onClick) {
  return el("button", { className: "ghost small icon-button", textContent: symbol, title: t(label), "aria-label": t(label), onclick: onClick, type: "button" });
}

function button(label, onClick, className = "ghost small") {
  return el("button", { className, textContent: t(label), onclick: onClick, type: "button" });
}

// 読み込めなかった画像(オフライン・アイコン無しのサイト等)は壊れたアイコンを出さずに隠す
document.addEventListener("error", (e) => {
  if (e.target.tagName === "IMG") e.target.classList.add("img-missing");
}, true);

// ───────── 見た目の反映 ─────────
function applyAppearance() {
  const root = document.documentElement;
  try {
    localStorage.setItem("grabbit-ui", JSON.stringify(settings.ui || {}));
  } catch {
    // 保存できなくても次回の起動が少し遅く見えるだけ
  }
  const prefersDark = matchMedia("(prefers-color-scheme: dark)").matches;
  const theme = ui("theme") === "system" ? (prefersDark ? "dark" : "light") : ui("theme");
  root.dataset.theme = theme;
  root.style.setProperty("--accent", ui("accent"));
  // 自分で選んだ色(選んでいなければテーマの色のまま)
  for (const [key, variable] of [["custom_bg", "--bg"], ["custom_panel", "--panel"], ["custom_text", "--text"]]) {
    if (ui(key)) root.style.setProperty(variable, ui(key));
    else root.style.removeProperty(variable);
  }
  root.style.setProperty("--font-size", `${ui("font_size")}px`);
  $("sites").classList.toggle("hidden", !ui("show_site_strip"));
  document.body.classList.toggle("no-nav-labels", !ui("show_nav_labels"));
  if (document.querySelector(".main") && $("tab-jobs")) applySplitView();
  document.body.classList.toggle("reduce-motion", Boolean(ui("reduce_motion")));
  applyNavLayout();
  // 自分で書いた CSS をそのまま当てる
  let userStyle = $("user-css");
  if (!userStyle) {
    userStyle = el("style", { id: "user-css" });
    document.head.append(userStyle);
  }
  userStyle.textContent = ui("custom_css");

  const lang = resolveLanguage(ui("language"));
  root.lang = lang;
  root.dir = LANGUAGES[lang]?.rtl ? "rtl" : "ltr";
  applyTranslations(lang);
}
matchMedia("(prefers-color-scheme: dark)").addEventListener("change", applyAppearance);

// ───────── タブ ─────────
function openTab(name) {
  document.querySelectorAll(".nav [data-tab]").forEach((b) => {
    b.classList.toggle("active", b.dataset.tab === name);
    if (b.dataset.tab === name) b.setAttribute("aria-current", "page");
    else b.removeAttribute("aria-current");
  });
  document.querySelectorAll(".tab").forEach((tab) => tab.classList.toggle("hidden", tab.id !== `tab-${name}`));
  if (name === "sites") loadSites();
  if (name === "library") loadLibrary();
  if (name === "logs") loadLogs();
  if (name === "presets") renderPresetEditor();
  if (name === "settings") renderTools();
  applySplitView();
}

/** 広い画面で、ダウンロードの横にライブラリを並べる(ダウンロード画面の時だけ) */
function applySplitView() {
  const main = document.querySelector(".main");
  const onJobs = !$("tab-jobs").classList.contains("hidden");
  const split = Boolean(ui("split_view")) && onJobs && innerWidth >= 1300;
  main.classList.toggle("split-active", split);
  if (split) {
    $("tab-library").classList.remove("hidden");
    setTimeout(() => loadLibrary(), 0); // 起動直後に呼ばれても、準備が終わってから読む
  } else if (onJobs) {
    $("tab-library").classList.add("hidden");
  }
}
addEventListener("resize", () => applySplitView());
document.querySelectorAll(".nav [data-tab]").forEach((b) => b.addEventListener("click", () => openTab(b.dataset.tab)));

// ───────── 左メニューの並び・表示 ─────────
// ダウンロードと設定は消せない(戻れなくなるため)
const NAV_LOCKED = ["jobs", "settings"];
function navButtons() {
  return [...document.querySelectorAll(".nav [data-tab]")];
}
/** 保存されている順番(新しく増えたタブは最後に足す) */
function navOrder() {
  const all = navButtons().map((b) => b.dataset.tab);
  const saved = (ui("nav_order") || []).filter((id) => all.includes(id));
  return [...saved, ...all.filter((id) => !saved.includes(id))];
}
function applyNavLayout() {
  const nav = document.querySelector(".nav");
  const hidden = ui("nav_hidden") || [];
  for (const id of navOrder()) {
    const b = nav.querySelector(`[data-tab="${id}"]`);
    nav.append(b);
    b.classList.toggle("hidden", hidden.includes(id) && !NAV_LOCKED.includes(id));
    if (b.classList.contains("hidden") && b.classList.contains("active")) openTab("jobs");
  }
}
async function changeNav(change) {
  settings.ui = { ...(settings.ui || {}), nav_order: navOrder(), nav_hidden: [...(ui("nav_hidden") || [])] };
  change(settings.ui);
  await persistSettings();
  applyNavLayout();
  renderNavEditor();
}
function renderNavEditor() {
  const order = navOrder();
  const hidden = ui("nav_hidden") || [];
  $("nav-editor").replaceChildren(...order.map((id, i) => {
    const b = document.querySelector(`.nav [data-tab="${id}"]`);
    const locked = NAV_LOCKED.includes(id);
    const show = el("input", { type: "checkbox", checked: locked || !hidden.includes(id), disabled: locked });
    show.addEventListener("change", () => changeNav((u) => {
      u.nav_hidden = show.checked ? u.nav_hidden.filter((h) => h !== id) : [...u.nav_hidden, id];
    }));
    const move = (delta) => changeNav((u) => {
      const j = i + delta;
      if (j < 0 || j >= u.nav_order.length) return;
      [u.nav_order[i], u.nav_order[j]] = [u.nav_order[j], u.nav_order[i]];
    });
    return el("li", {},
      el("label", { className: "check" }, show,
        el("span", { className: "ico", textContent: b.querySelector(".ico").textContent }),
        b.querySelector(".label").textContent),
      el("span", { className: "spacer" }),
      iconButton("↑", "上へ", () => move(-1)),
      iconButton("↓", "下へ", () => move(1)));
  }));
}

// ───────── 対応サイトのアイコン列 ─────────
function renderSiteStrip() {
  $("sites").replaceChildren(
    ...POPULAR_SITES.map((site) => el("img", { src: faviconOf(site), title: site, loading: "lazy" })),
    el("a", { textContent: t("ほか 1700 以上のサイトに対応 →"), onclick: () => openTab("sites") }),
  );
}

// ───────── サービス選択 ─────────
// サービスは選ばせず、URL から自動で判別する
function renderServices() {}

// ───────── URL の解析 → プレビュー → ダウンロード ─────────
/**
 * 入力から URL を取り出す。空白が無く URL がくっついていても分ける("…si=abchttps://…")。
 * "?url=https://…" のような URL の中の URL は分けない。追跡用の si= は外す。
 * (バックエンドの extract_urls と同じ決まり)
 */
function extractUrls(text) {
  text = String(text ?? "");
  const spaced = text.replace(/https?:\/\//g, (match, offset) => {
    const previous = text[offset - 1];
    return previous && !/[\s=&?/:#%("']/.test(previous) ? ` ${match}` : match;
  });
  return spaced.split(/\s+/).filter((s) => /^https?:\/\//.test(s)).map((url) => {
    const [base, query] = url.split("?");
    if (!query) return url;
    const kept = query.split("&").filter((p) => !p.startsWith("si="));
    return kept.length ? `${base}?${kept.join("&")}` : base;
  });
}

function urlsInInput() {
  return [...new Set(extractUrls($("url").value))];
}

function updateGoButton() {
  const urls = urlsInInput();
  const hasWords = !urls.length && $("url").value.trim();
  $("go").textContent = t(urls.length > 1 ? "まとめてダウンロード" : hasWords ? "検索" : "調べる");
}

function showStatus(text, kind = "") {
  const box = $("analyze-status");
  box.textContent = text;
  box.className = `analyze-status ${kind}`;
  box.classList.toggle("hidden", !text);
}

let analyzeTicket = 0; // 古い解析結果で上書きしないための番号
let previewUrl = "";

async function go() {
  const urls = urlsInInput();
  const words = $("url").value.trim();
  if (urls.length === 0 && words) return searchVideos(words);
  if (urls.length === 0) {
    showStatus(t("URL を貼り付けてください"), "error");
    return $("url").focus();
  }
  $("search-results").classList.add("hidden");
  $("url-suggest").classList.add("hidden");
  if (urls.length === 1) rememberUrl(urls[0]);
  if (urls.length > 1) {
    const { added, skipped } = await invoke("add_downloads", { text: urls.join("\n"), overrides: presetValues });
    $("url").value = "";
    updateGoButton();
    const message = t("{n} 件を追加しました", { n: added });
    return showStatus(skipped ? `${message}(${t("重複 {n} 件は除外", { n: skipped })})` : message);
  }

  const ticket = ++analyzeTicket;
  const url = urls[0];
  $("preview").classList.add("hidden");
  $("go").disabled = true;
  showStatus(t("何がダウンロードできるか調べています…"), "busy");

  // 本格的な解析は数秒かかるので、ページの見出し情報だけ先に読んで出しておく。
  // その時点で「保存」も押せる(細かい形式の一覧は解析が終わり次第出る)
  let fullDone = false;
  // 何も返ってこないうちから、サイト名とアイコンだけの枠を出して保存も押せるようにする
  showPreview(quickInfo(url, { title: url, thumbnail: "", site: detectService(url).name, uploader: "" }));
  invoke("quick_preview", { url }).then((quick) => {
    if (!quick || fullDone || ticket !== analyzeTicket) return;
    showPreview(quickInfo(url, quick));
    showStatus(t("詳しい形式を調べています…(このまま保存もできます)"), "busy");
  }).catch(() => {});

  try {
    const info = await invoke("analyze_url", { url, overrides: null });
    fullDone = true;
    if (ticket !== analyzeTicket) return;
    showStatus("");
    showPreview(info);
  } catch (e) {
    fullDone = true;
    if (ticket !== analyzeTicket) return;
    const reason = describeMessage(String(e));
    if (previewInfo?.quick && previewInfo.url === url) {
      // 見出しは読めたので、保存は試せるようにしておく(保存時に別の方法でも取りにいく)
      showStatus(`${t("詳しい形式は分かりませんでした")}: ${reason}`, "error");
    } else {
      showStatus(`${t("ダウンロードできませんでした")}: ${reason}`, "error");
    }
  } finally {
    $("go").disabled = false;
  }
}

let previewInfo = null;
let previewMode = "video";
// サムネだけ保存する時に選んだ画像(複数可)。"all" は「全サイズまとめて」
let selectedThumbnails = new Set();
let extraThumbnails = []; // 他の言語で見つかったサムネイル { url, lang }
const THUMBNAIL_LANGS = ["ja", "en", "ko", "zh-Hans", "zh-Hant", "es", "fr", "de", "pt", "ru", "it", "id", "th", "vi", "tr", "ar", "hi"];

// 地域の選択肢(国名は表示中の言語で出す)
const REGIONS = ["JP", "US", "GB", "KR", "TW", "HK", "CN", "DE", "FR", "IT", "ES", "BR", "MX", "CA", "AU", "IN", "RU", "ID", "TH", "VN", "PH", "TR", "SA"];

function fillRegionSelects() {
  let names;
  try {
    names = new Intl.DisplayNames([document.documentElement.lang], { type: "region" });
  } catch {
    names = { of: (code) => code };
  }
  for (const select of document.querySelectorAll(".region-select")) {
    const current = select.value;
    select.replaceChildren(new Option(t("指定しない"), ""), ...REGIONS.map((code) => new Option(`${names.of(code)} (${code})`, code)));
    select.value = current;
  }
}

/** サムネイルを選べるように並べる(サムネだけ保存の時)。押すたびに選ぶ/外すが切り替わる */
function renderThumbnailPicker() {
  const picker = $("pv-thumbs");
  const thumbs = [
    ...(previewInfo?.thumbnails || []).map((th) => ({ ...th, label: th.width ? `${th.width}×${th.height}` : "" })),
    ...extraThumbnails.map((th) => ({ url: th.url, label: languageName(th.lang) })),
  ];
  const show = previewMode === "thumbnail" && thumbs.length > 0;
  picker.classList.toggle("hidden", !show);
  if (!show) return;
  if (selectedThumbnails.size === 0) selectedThumbnails.add(thumbs[0].url);

  const toggle = (url) => {
    if (url === "all") {
      selectedThumbnails = new Set(selectedThumbnails.has("all") ? [thumbs[0].url] : ["all"]);
    } else {
      selectedThumbnails.delete("all");
      if (selectedThumbnails.has(url) && selectedThumbnails.size > 1) selectedThumbnails.delete(url);
      else selectedThumbnails.add(url);
    }
    renderThumbnailPicker();
    updatePreviewControls();
  };
  const choice = (url, child, label) => el("button", {
    type: "button",
    className: `thumb-choice ${selectedThumbnails.has(url) ? "active" : ""}`,
    title: label,
    onclick: () => toggle(url),
  }, child, el("span", { textContent: label }));

  const everyThumbnail = () => {
    selectedThumbnails = new Set(thumbs.map((th) => th.url));
    renderThumbnailPicker();
    updatePreviewControls();
  };
  picker.replaceChildren(
    ...thumbs.map((th) => choice(th.url, el("img", { src: th.url, loading: "lazy" }), th.label)),
    choice("all", el("div", { className: "thumb-all", textContent: "⧉" }), t("全サイズ")),
    el("div", { className: "thumb-tools" },
      button("表示中を全部選ぶ", everyThumbnail),
      button("他の言語のサムネイルも探す", findLocalizedThumbnails),
      el("span", { id: "thumb-lang-status", className: "muted" }),
    ),
  );
}

function languageName(code) {
  try {
    return new Intl.DisplayNames([document.documentElement.lang], { type: "language" }).of(code);
  } catch {
    return code;
  }
}

/** 言語を変えて調べ直し、言語ごとに違うサムネイルを足す */
async function findLocalizedThumbnails() {
  const status = () => $("thumb-lang-status");
  status().textContent = t("言語ごとに調べています…");
  const url = previewUrl;
  const found = await invoke("localized_thumbnails", { url, langs: THUMBNAIL_LANGS }).catch(() => []);
  if (url !== previewUrl) return;
  const known = new Set((previewInfo.thumbnails || []).map((th) => th.url.split("?")[0]));
  extraThumbnails = found.filter((th) => !known.has(th.url.split("?")[0]));
  renderThumbnailPicker();
  status().textContent = extraThumbnails.length
    ? t("{n} 言語で違うサムネイルが見つかりました", { n: extraThumbnails.length })
    : t("言語による違いはありませんでした");
}

// ───────── キーワード検索 ─────────
async function searchVideos(query) {
  const service = SERVICES.find((s) => s.id === selectedService);
  const target = ["niconico", "soundcloud", "bilibili"].includes(service?.id) ? service.id : "youtube";
  $("preview").classList.add("hidden");
  showStatus(t("「{q}」を探しています…", { q: query }), "busy");
  $("go").disabled = true;
  try {
    const results = await invoke("search_videos", { query, service: target });
    showStatus(results.length ? "" : t("見つかりませんでした"), results.length ? "" : "error");
    renderSearchResults(results);
  } catch (e) {
    showStatus(`${t("検索できませんでした")}: ${describeMessage(String(e))}`, "error");
  } finally {
    $("go").disabled = false;
  }
}

function renderSearchResults(results) {
  const box = $("search-results");
  box.classList.toggle("hidden", results.length === 0);
  const picked = new Set();
  const saveButton = el("button", { type: "button", className: "primary small", disabled: true });
  const updateSave = () => {
    saveButton.disabled = picked.size === 0;
    saveButton.textContent = t("選んだ {n} 件を保存", { n: picked.size });
  };
  saveButton.onclick = async () => {
    const { added } = await invoke("add_downloads", { text: [...picked].join("\n"), overrides: presetValues });
    showStatus(t("{n} 件を追加しました", { n: added }));
    box.classList.add("hidden");
  };
  updateSave();
  box.replaceChildren(
    el("div", { className: "search-head" },
      el("span", { textContent: t("{n} 件見つかりました", { n: results.length }) }),
      saveButton,
      button("閉じる", () => box.classList.add("hidden"))),
    el("div", { className: "search-grid" }, ...results.map((r) => {
      const card = el("div", { className: "search-card" });
      const check = el("input", { type: "checkbox", onchange: () => {
        check.checked ? picked.add(r.url) : picked.delete(r.url);
        card.classList.toggle("picked", check.checked);
        updateSave();
      } });
      card.append(
        el("img", { src: r.thumbnail || faviconOf(r.url), loading: "lazy", onclick: () => check.click() }),
        el("label", { className: "search-title" }, check, el("span", { textContent: r.title })),
        el("div", { className: "muted small-text", textContent: [r.uploader, formatDuration(r.duration)].filter(Boolean).join(" · ") }),
        savedLabel(r.url) ? el("span", { className: "badge saved", textContent: savedLabel(r.url) }) : "",
        button("詳しく見る", () => {
          $("url").value = r.url;
          box.classList.add("hidden");
          go();
        }),
      );
      return card;
    })),
  );
}

/** 見出し情報だけの簡易プレビューを、解析結果と同じ形にする */
function quickInfo(url, quick) {
  return {
    url, quick: true, title: quick.title, uploader: quick.uploader || "", thumbnail: quick.thumbnail, duration: null,
    site: quick.site || hostOf(url), is_playlist: false, entry_count: 0, formats: [], engine: "yt-dlp",
    notice: "", subtitle_langs: [], auto_caption_langs: [], can_subscribe: false,
    thumbnails: quick.thumbnail ? [{ url: quick.thumbnail, width: 0, height: 0 }] : [],
  };
}

// ───────── 保存先に同じ名前のファイルがないか ─────────
let existsCheckTimer;
let existsCheckId = 0;
/** 調べた動画を保存した時の名前で、保存先にもうファイルがあれば知らせる(少し待ってから1回だけ調べる) */
function checkExistingFile() {
  clearTimeout(existsCheckTimer);
  $("pv-exists").classList.add("hidden");
  if (!previewInfo || previewInfo.is_playlist || previewInfo.engine !== "yt-dlp" || !["video", "audio"].includes(previewMode)) return;
  const id = ++existsCheckId;
  const url = previewUrl;
  existsCheckTimer = setTimeout(async () => {
    const path = await invoke("check_existing_file", { url, overrides: collectOverrides() }).catch(() => null);
    if (id !== existsCheckId || !path) return; // 別の動画に移っていたら出さない
    const what = {
      skip: "保存先に同じ名前のファイルがあります。このまま保存すると、すでにあるものとして飛ばされます",
      overwrite: "保存先に同じ名前のファイルがあります。このまま保存すると上書きされます",
      rename: "保存先に同じ名前のファイルがあります。番号を付けた別の名前で保存します",
    }[settings.on_same_name] || "保存先に同じ名前のファイルがあります。このまま保存すると、すでにあるものとして飛ばされます";
    $("pv-exists").replaceChildren(
      el("span", { textContent: `⚠ ${t(what)}` }),
      el("small", { className: "muted", textContent: path }),
      button("フォルダで表示", () => invoke("show_in_folder", { path })),
    );
    $("pv-exists").classList.remove("hidden");
  }, 600);
}

function showPreview(info, keepRegion = false) {
  const previous = previewInfo; // 簡易プレビューから本格的な結果に替わる時、選んだものを保つため
  previewInfo = info;
  if (previous?.url !== info.url) {
    selectedThumbnails = new Set();
    extraThumbnails = [];
  }
  previewUrl = info.url;
  const service = detectService(info.url);
  const siteName = { "gallery-dl": t("画像・ギャラリー"), direct: t("ファイル直リンク") }[info.site] || info.site || hostOf(info.url);
  $("pv-thumb").src = info.thumbnail || faviconOf(info.url);
  $("pv-icon").src = faviconOf(info.url);
  $("pv-sitename").textContent = siteName;
  $("pv-title").textContent = info.title || info.url;
  $("pv-meta").textContent = [
    info.uploader,
    formatDuration(info.duration),
    info.is_playlist && t("{n} 件", { n: info.entry_count }),
    estimateText(info.filesize),
  ].filter(Boolean).join(" · ");
  warnIfNoSpace(info.filesize);
  const saved = savedLabel(info.url);
  $("pv-saved").textContent = saved;
  $("pv-saved").classList.toggle("hidden", !saved);
  $("pv-notice").textContent = info.notice ? describeMessage(info.notice) : "";
  $("pv-notice").classList.toggle("hidden", !info.notice);

  // 保存するもの: サービスでできることのうち、この URL で実際にできるものだけ
  const hasSubs = info.subtitle_langs.length + info.auto_caption_langs.length > 0;
  const available = info.engine === "yt-dlp" ? service.modes.filter((m) => {
    if (m === "subtitles") return hasSubs || info.is_playlist || info.quick;
    if (m === "thumbnail") return Boolean(info.thumbnail) || info.is_playlist;
    return true;
  }) : ["video"];
  const keepMode = previous?.quick && previous.url === info.url && available.includes(previewMode);
  previewMode = keepMode ? previewMode : available.includes(settings.mode) ? settings.mode : available[0];
  $("pv-modes").replaceChildren(...available.map((mode) => el("button", {
    type: "button",

    className: mode === previewMode ? "active" : "",
    textContent: info.engine === "yt-dlp" ? t(MODES[mode]) : t("保存"),
    onclick: () => {
      previewMode = mode;
      $("pv-modes").querySelectorAll("button").forEach((b, i) => b.classList.toggle("active", available[i] === mode));
      updatePreviewControls();
      renderThumbnailPicker();
    },
  })));
  $("pv-modes").querySelectorAll("button").forEach((b, i) => (b.dataset.mode = available[i]));
  $("pv-modes").classList.toggle("hidden", available.length < 2);

  // 画質の選択肢(動画の時)
  const select = $("pv-format");
  select.replaceChildren();
  if (info.engine === "yt-dlp") {
    const presets = el("optgroup", { label: t("かんたん") });
    for (const [value, label] of PRESETS.filter(([v]) => !v.startsWith("audio"))) {
      const size = presetSize(info.formats, value);
      presets.append(new Option(size ? `${t(label)} — ${t("約 {size}", { size: formatSize(size) })}` : t(label), `preset:${value}`));
    }
    select.append(presets);
    const videoFormats = info.formats.filter((f) => f.kind !== "audio");
    if (videoFormats.length) {
      const group = el("optgroup", { label: t("個別の形式") });
      for (const f of videoFormats) group.append(new Option(f.label, f.selector));
      select.append(group);
    }
    select.value = settings.preset.startsWith("audio") ? "preset:best" : `preset:${settings.preset}`;
  }
  $("pv-audio-format").value = settings.audio_format;

  // 保存方法の細かい指定(初期値は今の設定)。簡易プレビュー中に変えた内容は保つ
  if (!keepMode) {
    $("opt-filename").value = settings.filename_template;
    previewFilename.refresh();
    $("opt-output").value = settings.output_dir;
    $("opt-container").value = settings.container;
    $("opt-subs").value = settings.burn_subtitles ? "burn" : settings.embed_subtitles && settings.write_subtitle_file ? "both"
      : settings.embed_subtitles ? "embed" : settings.write_subtitle_file ? "file" : "none";
    $("opt-embed-thumb").checked = settings.embed_thumbnail;
    $("opt-metadata").checked = settings.embed_metadata;
    $("opt-sponsor").checked = settings.sponsorblock_remove;
    $("opt-thumb-file").checked = settings.write_thumbnail_file;
    $("opt-sections").value = settings.download_sections;
    $("opt-vr").checked = settings.vr_mode || /360°|360度|\bVR\b|VR180/i.test(info.title);
    $("opt-comments").checked = settings.write_comments;
    $("opt-live-chat").checked = settings.write_live_chat;
    $("opt-live-chat").closest("label").classList.toggle("hidden", !service.live);
    if (!keepRegion) $("opt-region").value = settings.region;
    $("opt-items").value = settings.playlist_items;
  }
  // 字幕の言語は、解析で分かった分を毎回入れ直す
  fillSubtitleLangs(info, false);
  $("opt-items-label").classList.toggle("hidden", !info.is_playlist);
  $("opt-sponsor").closest("label").classList.toggle("hidden", service.id !== "youtube");

  $("pv-subscribe").classList.toggle("hidden", !(info.can_subscribe && service.subscribe));
  // 配信中なら、最初から録るか今から録るかを選べる
  liveFromStart = Boolean(settings.live_from_start);
  $("pv-live").classList.toggle("hidden", !info.live_now);
  $("pv-live").querySelectorAll("button").forEach((b) => b.classList.toggle("active", (b.dataset.live === "start") === liveFromStart));
  // YouTube のチャンネルなら、そのチャンネルの再生リスト一覧を見られる
  const channelBase = youtubeChannelBase(info.url);
  $("pv-channel-playlists").classList.toggle("hidden", !channelBase || /\/playlists$/.test(info.url));
  // 配信予定・プレミア公開: 始まる時刻に合わせて自動で予約する
  if (info.starts_at && info.starts_at * 1000 > Date.now()) {
    const when = new Date(info.starts_at * 1000).toLocaleString(document.documentElement.lang);
    $("pv-notice").textContent = `${t("{time} に始まる配信です。保存を押すと、始まる時刻に合わせて自動で録画します", { time: when })}`;
    $("pv-notice").classList.remove("hidden");
  }
  $("pv-toggle-options").classList.toggle("hidden", info.engine !== "yt-dlp");
  if (!keepRegion && !keepMode) $("pv-options").classList.add("hidden");
  if (previous?.url !== info.url || !pickedEntries.size) pickedEntries = new Set((info.entries || []).map((e) => e.url));
  if (!keepMode) {
    // 前回選んだ保存のしかたを使う(このサイトで使えない時は自分の設定に戻す)
    const remembered = allPresets().find((p) => p.id === activePreset);
    if (remembered && service.modes.includes(remembered.values.mode || "video")) applyPreset(remembered);
    else presetValues = {};
  }
  renderPresets();
  renderPlaylistPicker();
  updatePreviewControls();
  renderThumbnailPicker();
  $("preview").classList.remove("hidden");
  $("pv-download").focus();
}

// ───────── プレイリストの中身を選ぶ ─────────
let pickedEntries = new Set(); // 選んだ動画の URL

function renderPlaylistPicker() {
  const box = $("pv-playlist");
  const entries = previewInfo?.entries || [];
  box.classList.toggle("hidden", entries.length === 0);
  if (entries.length === 0) return;

  const counter = el("span", { className: "muted" });
  const update = () => {
    counter.textContent = t("{n} / {total} 件を選択中", { n: pickedEntries.size, total: entries.length });
    updatePreviewControls();
  };
  const setAll = (on) => {
    pickedEntries = new Set(on ? entries.map((e) => e.url) : []);
    renderPlaylistPicker();
  };
  // 「1-10,15」のような範囲指定で選ぶ
  const range = el("input", { placeholder: t("例: 1-10,15"), className: "range-input" });
  range.addEventListener("change", () => {
    const wanted = new Set();
    for (const part of range.value.split(",")) {
      const [a, b] = part.split("-").map((n) => parseInt(n, 10));
      if (Number.isNaN(a)) continue;
      for (let i = a; i <= (Number.isNaN(b) ? a : b); i++) wanted.add(i);
    }
    pickedEntries = new Set(entries.filter((e) => wanted.has(e.index)).map((e) => e.url));
    renderPlaylistPicker();
  });
  const filter = el("input", { type: "search", placeholder: t("タイトルで絞り込み"), className: "range-input" });
  const list = el("ul", { className: "playlist-list" });
  const drawList = () => {
    const query = filter.value.trim().toLowerCase();
    list.replaceChildren(...entries.filter((e) => !query || e.title.toLowerCase().includes(query)).map((entry) => {
      const check = el("input", { type: "checkbox", checked: pickedEntries.has(entry.url) });
      check.addEventListener("change", () => {
        check.checked ? pickedEntries.add(entry.url) : pickedEntries.delete(entry.url);
        update();
      });
      return el("li", {},
        el("label", { className: "playlist-row" },
          check,
          el("span", { className: "playlist-index", textContent: entry.index }),
          entry.thumbnail ? el("img", { src: entry.thumbnail, loading: "lazy" }) : "",
          el("span", { className: "playlist-title", textContent: entry.title || entry.url }),
          savedLabel(entry.url) ? el("span", { className: "badge saved", textContent: "✓" , title: savedLabel(entry.url) }) : "",
          el("span", { className: "muted", textContent: formatDuration(entry.duration) })));
    }));
  };
  filter.addEventListener("input", drawList);
  drawList();
  box.replaceChildren(
    el("div", { className: "playlist-tools" },
      button("全部選ぶ", () => setAll(true)),
      button("全部外す", () => setAll(false)),
      range, filter, counter),
    list,
  );
  update();
}

/**
 * 字幕の言語の選択肢。自動生成の字幕は100以上の言語があって選びにくいので、
 * 最初は「この動画に付いている字幕」と「よく使う言語」だけを出す。
 */
function fillSubtitleLangs(info, showAll) {
  const langs = $("opt-sub-langs");
  const wanted = settings.subtitle_langs.split(",").map((l) => l.trim());
  const common = new Set([...wanted, "ja", "en", "ko", "zh-Hans", "zh-Hant", "es", "fr", "de", "pt", "ru"]);
  const manual = info.subtitle_langs.filter((l) => l !== "live_chat");
  const auto = info.auto_caption_langs.filter((l) => !manual.includes(l) && (showAll || common.has(l)));
  const label = (code) => `${languageName(code)} (${code})`;
  langs.replaceChildren(
    ...manual.map((l) => new Option(label(l), l, false, wanted.includes(l))),
    ...auto.map((l) => new Option(`${label(l)} · ${t("自動生成")}`, `auto:${l}`, false, wanted.includes(l))),
  );
  const hidden = info.auto_caption_langs.length - manual.length - auto.length;
  $("opt-sub-more").classList.toggle("hidden", showAll || hidden <= 0);
  $("opt-sub-more").textContent = t("ほかの言語も出す(+{n})", { n: hidden });
  $("opt-sub-more").onclick = () => fillSubtitleLangs(info, true);
}

/** 保存するものに合わせて、関係ある入力だけを出す */
function updatePreviewControls() {
  if (!previewInfo) return; // まだ何も調べていない時(保存プリセットを選んだだけの時など)
  checkExistingFile(); // 保存のしかたが変わると名前も変わるので、そのたびに調べ直す
  const info = previewInfo;
  const isYtdlp = info.engine === "yt-dlp";
  $("pv-format").classList.toggle("hidden", !isYtdlp || previewMode !== "video");
  $("pv-audio-format").classList.toggle("hidden", previewMode !== "audio");
  const hasLangs = $("opt-sub-langs").options.length > 0;
  $("opt-sub-langs-label").classList.toggle("hidden", !hasLangs || (previewMode !== "subtitles" && $("opt-subs").value === "none"));
  $("opt-subs").closest("label").classList.toggle("hidden", previewMode !== "video" || !hasLangs);
  $("opt-container").closest("label").classList.toggle("hidden", previewMode !== "video");
  $("opt-embed-thumb").closest("label").classList.toggle("hidden", previewMode === "thumbnail" || previewMode === "info");
  const pictures = [...selectedThumbnails].filter((u) => u !== "all").length;
  const count = info.is_playlist ? t("{n} 件", { n: info.entries?.length ? pickedEntries.size : info.entry_count })
    : previewMode === "thumbnail" && pictures > 1 ? t("{n} 枚", { n: pictures }) : "";
  $("pv-download").textContent = [t(MODES[previewMode] && isYtdlp ? `${MODES[previewMode]}を保存` : "ダウンロード"), count].filter(Boolean).join(" · ");
  updateFilenamePreview();
}

// ファイル名は選ぶだけで組み立てられるようにする(例はこの動画の値で出す)
const previewFilename = FilenameBuilder.mount($("fn-preview"), $("opt-filename"), () => ({
  title: previewInfo?.title,
  uploader: previewInfo?.uploader,
  channel: previewInfo?.uploader,
  extractor_key: previewInfo?.site,
  ext: previewMode === "audio" ? $("pv-audio-format").value : $("opt-container").value,
}));
function updateFilenamePreview() {
  previewFilename.refresh();
}
const settingsFilename = FilenameBuilder.mount($("fn-settings"), filenameTemplateInput());
function filenameTemplateInput() {
  return document.querySelector('#settings [name="filename_template"]');
}

// ───────── プリセット(初心者向けのおまかせ設定) ─────────
// 値は「この1件だけ設定を上書きする内容」。選ぶだけで保存方法がまとめて決まる
// 最初は「自分の設定」(設定画面の値)だけ。ほかは自分で作る
let activePreset = ""; // 初めは何も選ばない(設定どおりに保存する)

/** 自分で保存したプリセット */
function allPresets() {
  return settings.ui?.presets || [];
}

/** 前の版で既定の名前「自分の設定」になっていた保存プリセットは、上の「自分の設定」と区別して出す */
function presetLabel(preset) {
  return preset.name === "自分の設定" ? t("保存した設定") : preset.name;
}

function renderPresets() {
  // 調べる前でも全部出す。調べた後はそのサイトで使えるものだけ
  const box = $("pv-presets");
  const service = previewInfo?.engine === "yt-dlp" ? detectService(previewInfo.url) : null;
  // 最初は「自分の設定」と自分で保存したものだけ。標準のプリセットは設定で出せる
  const shownPresets = allPresets();
  const usable = shownPresets.filter((p) => !service || service.modes.includes(p.values.mode || "video"));
  const mine = el("button", {
    type: "button",
    className: `preset ${activePreset ? "" : "active"}`,
    title: t("設定画面の値どおりに保存します"),
    onclick: () => clearPreset(),
  }, el("span", { className: "preset-icon", textContent: mySettingsAsPreset().icon }),
  el("span", { className: "preset-text" }, el("b", { textContent: mySettingsAsPreset().name }), el("small", { textContent: mySettingsAsPreset().note })));
  box.replaceChildren(
    mineHidden() ? "" : mine,
    ...usable.map((preset) => el("button", {
      type: "button",
      className: `preset ${preset.id === activePreset ? "active" : ""}`,
      title: `${preset.note || ""} (${t("右クリックで削除")})`,
      // 選んでいるものをもう一度押すと選択を外す(設定どおりに戻る)
      onclick: () => (preset.id === activePreset ? clearPreset() : applyPreset(preset)),
      oncontextmenu: (e) => {
        e.preventDefault();
        if (confirm(t("プリセット「{name}」を削除しますか?", { name: preset.name }))) deletePreset(preset.id);
      },
    },
      el("span", { className: "preset-icon", textContent: preset.icon || "◆" }),
      el("span", { className: "preset-text" },
        el("b", { textContent: presetLabel(preset) }),
        el("small", { textContent: preset.note || t("保存したプリセット") })))),
    el("button", { type: "button", className: "preset add", textContent: t("＋ 今の設定を保存"), onclick: saveCurrentAsPreset }),
  );
}

/** プリセットを選ぶと、保存するもの・画質・細かい指定がまとめて切り替わる */
function applyPreset(preset) {
  activePreset = preset.id;
  if (settings.ui?.active_preset !== preset.id) {
    settings.ui = { ...(settings.ui || {}), active_preset: preset.id };
    persistSettings();
  }
  presetValues = { ...preset.values };
  const v = presetValues;
  previewMode = v.mode || "video";
  if (v.audio_format) $("pv-audio-format").value = v.audio_format;
  if (v.preset && $("pv-format").querySelector(`option[value="preset:${v.preset}"]`)) $("pv-format").value = `preset:${v.preset}`;
  if (v.container) $("opt-container").value = v.container;
  $("opt-subs").value = v.burn_subtitles ? "burn" : v.embed_subtitles && v.write_subtitle_file ? "both"
    : v.embed_subtitles ? "embed" : v.write_subtitle_file ? "file" : "none";
  if ("embed_thumbnail" in v) $("opt-embed-thumb").checked = v.embed_thumbnail;
  if ("embed_metadata" in v) $("opt-metadata").checked = v.embed_metadata;
  if ("write_thumbnail_file" in v) $("opt-thumb-file").checked = v.write_thumbnail_file;
  if ("write_comments" in v) $("opt-comments").checked = v.write_comments;
  $("pv-modes").querySelectorAll("button").forEach((b) => b.classList.toggle("active", b.dataset.mode === previewMode));
  renderPresets();
  // URL を調べる前は、プレビューの表示を更新しない
  if (!previewInfo) return;
  updatePreviewControls();
  renderThumbnailPicker();
}
let presetValues = {};

/** プリセットを選んでいない状態(設定画面の値どおりに保存する)に戻す */
/** 「自分の設定」を消しているか(保存プリセットが1つも無い時は消せないので出す) */
function mineHidden() {
  return Boolean(ui("hide_mine")) && (settings.ui?.presets || []).length > 0;
}

function clearPreset() {
  // 「自分の設定」を消している時は、先頭の保存プリセットを使う
  if (mineHidden()) return applyPreset(allPresets()[0]);
  activePreset = "";
  presetValues = {};
  if (settings.ui?.active_preset) {
    settings.ui = { ...settings.ui, active_preset: "" };
    persistSettings();
  }
  renderPresets();
}

async function saveCurrentAsPreset() {
  const name = prompt(t("プリセットの名前"), t("保存した設定"));
  if (!name) return;
  const overrides = collectOverrides(true);
  const preset = { id: `custom-${Date.now()}`, icon: "◆", name, values: overrides };
  settings.ui = settings.ui || {};
  settings.ui.presets = [...(settings.ui.presets || []), preset];
  await persistSettings();
  activePreset = preset.id;
  renderPresets();
}

async function deletePreset(id) {
  settings.ui.presets = (settings.ui.presets || []).filter((p) => p.id !== id);
  if (activePreset === id) {
    activePreset = "";
    presetValues = {};
    if (mineHidden()) {
      activePreset = allPresets()[0].id;
      presetValues = { ...allPresets()[0].values };
    }
    settings.ui.active_preset = activePreset;
  }
  await persistSettings();
  renderPresets();
}

/** 画面で選んだ保存方法を、この1件だけの上書き設定にまとめる */
function collectOverrides(forceDetails = false) {
  // 選んでいるプリセットの内容を土台にして、画面で変えた所を上に重ねる
  const overrides = { ...presetValues, mode: previewMode };
  if (previewMode === "audio") overrides.audio_format = $("pv-audio-format").value;
  if (previewMode === "thumbnail") {
    if (selectedThumbnails.has("all")) overrides.write_all_thumbnails = true;
  }
  // 配信中: 最初から録るか、今から録るか
  if (previewInfo?.live_now) overrides.live_from_start = liveFromStart;
  if (!forceDetails && $("pv-options").classList.contains("hidden")) return overrides;
  if ($("opt-start-when").value) overrides.__start_when = $("opt-start-when").value;

  const subs = $("opt-subs").value;
  const chosen = [...$("opt-sub-langs").selectedOptions].map((o) => o.value);
  Object.assign(overrides, {
    filename_template: $("opt-filename").value || settings.filename_template,
    output_dir: $("opt-output").value || settings.output_dir,
    container: $("opt-container").value,
    embed_subtitles: subs === "embed" || subs === "both",
    burn_subtitles: subs === "burn",
    write_subtitle_file: subs === "file" || subs === "both",
    embed_thumbnail: $("opt-embed-thumb").checked,
    embed_metadata: $("opt-metadata").checked,
    sponsorblock_remove: $("opt-sponsor").checked,
    write_thumbnail_file: $("opt-thumb-file").checked,
    download_sections: $("opt-sections").value.trim(),
    vr_mode: $("opt-vr").checked,
    write_comments: $("opt-comments").checked,
    write_live_chat: $("opt-live-chat").checked,
    region: $("opt-region").value,
    playlist_items: $("opt-items").value.trim(),
  });
  if (chosen.length) {
    overrides.subtitle_langs = chosen.map((v) => v.replace(/^auto:/, "")).join(",");
    overrides.auto_subtitles = chosen.some((v) => v.startsWith("auto:"));
  }
  return overrides;
}

/** 予約の日時(UNIX秒)。指定がない・過去なら null(すぐ始める) */
function scheduledTime() {
  const value = $("opt-start-at").value;
  if (!value || $("pv-options").classList.contains("hidden")) {
    // 配信予定は、始まる 1 分前に予約する(待っている間に回線を使わない)
    const starts = previewInfo?.starts_at;
    return starts && starts * 1000 > Date.now() + 60_000 ? starts - 60 : null;
  }
  const at = Math.floor(new Date(value).getTime() / 1000);
  return at > Date.now() / 1000 ? at : null;
}

/** YouTube のチャンネルの URL なら、チャンネルの入口(…/@名前)を返す */
function youtubeChannelBase(url = "") {
  const m = url.match(/^(https:\/\/(?:www\.|m\.)?youtube\.com\/(?:@[^/?#]+|channel\/[^/?#]+|c\/[^/?#]+))/);
  return m ? m[1] : "";
}

async function downloadPreview() {
  if (!previewInfo.is_playlist && !allowDuplicate(previewUrl)) return;
  const choice = $("pv-format").value;
  let format = null;
  if (previewMode === "video" && choice.startsWith("preset:")) {
    const preset = choice.slice("preset:".length);
    if (preset !== settings.preset) {
      settings.preset = preset; // 選んだプリセットを次回の既定にする
      await persistSettings();
    }
  } else if (previewMode === "video" && choice) {
    format = choice;
  }
  const overrides = collectOverrides();
  const common = { text: previewUrl, format, startAt: scheduledTime() };
  const chosen = [...selectedThumbnails].filter((u) => u !== "all");
  if (previewMode === "thumbnail" && chosen.length > 0 && !selectedThumbnails.has("all")) {
    // 選んだサムネイルを1枚ずつ保存する(言語ごとのものは名前に言語を付ける)
    for (const url of chosen) {
      const lang = extraThumbnails.find((th) => th.url === url)?.lang;
      const size = (previewInfo.thumbnails || []).find((th) => th.url === url);
      const suffix = lang ? ` [${lang}]` : chosen.length > 1 && size?.width ? ` [${size.width}x${size.height}]` : "";
      await invoke("add_downloads", {
        ...common,
        overrides: { ...overrides, thumbnail_url: url },
        title: `${previewInfo.title || "thumbnail"}${suffix}`,
        thumbnail: url,
      });
    }
  } else if (previewInfo.entries?.length && pickedEntries.size < previewInfo.entries.length) {
    // プレイリストの一部だけを選んだ時は、選んだ動画を1本ずつ入れる
    if (pickedEntries.size === 0) return showStatus(t("保存する動画を選んでください"), "error");
    await invoke("add_downloads", { ...common, text: [...pickedEntries].join("\n"), overrides });
  } else {
    await invoke("add_downloads", {
      ...common,
      overrides,
      title: previewInfo.title || null,
      thumbnail: previewInfo.thumbnail || null,
    });
  }
  $("preview").classList.add("hidden");
  $("url").value = "";
  updateGoButton();
  showStatus(t("ダウンロードを開始しました"));
  setTimeout(() => showStatus(""), 3000);
}

$("pv-toggle-options").addEventListener("click", () => {
  $("pv-options").classList.toggle("hidden");
  updatePreviewControls();
});
$("opt-subs").addEventListener("change", updatePreviewControls);
// 地域を変えたら、その地域のタイトル・サムネイルで調べ直す
$("opt-region").addEventListener("change", async () => {
  const region = $("opt-region").value;
  showStatus(t("何がダウンロードできるか調べています…"), "busy");
  try {
    const info = await invoke("analyze_url", { url: previewUrl, overrides: { region } });
    showStatus("");
    showPreview(info, true);
    $("opt-region").value = region;
  } catch (e) {
    showStatus(`${t("ダウンロードできませんでした")}: ${describeMessage(String(e))}`, "error");
  }
});
$("opt-container").addEventListener("change", updateFilenamePreview);
$("pv-subscribe").addEventListener("click", () => {
  const service = detectService(previewUrl);
  openTab("settings");
  selectSubService(service.id);
  $("sub-url").value = previewUrl;
  $("sec-subs").scrollIntoView({ behavior: "smooth" });
  $("preview").classList.add("hidden");
});

$("go").addEventListener("click", go);
$("pv-download").addEventListener("click", downloadPreview);

// ───────── ログ ─────────
let logs = [];
let currentLog = null;
let logTimer = null;

async function loadLogs() {
  logs = await invoke("list_logs").catch(() => []);
  renderLogList();
}

function renderLogList() {
  const query = $("log-search").value.trim().toLowerCase();
  const errorsOnly = $("log-errors-only").checked;
  const shown = logs.filter((log) => {
    const job = jobs.get(log.id);
    const title = job?.title || log.title || String(log.id);
    if (errorsOnly && (job?.status || log.status) !== "failed") return false;
    return !query || title.toLowerCase().includes(query) || (job?.url || log.url).toLowerCase().includes(query);
  });
  $("log-list").replaceChildren(...shown.map((log) => {
    const job = jobs.get(log.id);
    const status = job?.status || log.status;
    return el("li", {
      className: `log-item ${status || ""} ${currentLog === log.id ? "active" : ""}`,
      onclick: () => openLog(log.id),
    },
      el("span", { className: "log-state", textContent: STATUS[status]?.icon || "·" }),
      el("div", { className: "log-item-body" },
        el("div", { className: "log-item-title", textContent: job?.title || log.title || job?.url || log.url || t("(タイトル不明)") }),
        el("div", { className: "muted small-text", textContent: [
          new Date(log.modified * 1000).toLocaleString(document.documentElement.lang),
          formatSize(log.size),
        ].filter(Boolean).join(" · ") })));
  }));
}

async function openLog(id) {
  currentLog = id;
  renderLogList();
  const job = jobs.get(id);
  $("log-title").textContent = job?.title || logs.find((l) => l.id === id)?.title || String(id);
  await drawLog();
  // ダウンロード中の記録は、開いている間 2 秒ごとに読み直す
  clearInterval(logTimer);
  logTimer = setInterval(() => {
    if ($("tab-logs").classList.contains("hidden")) return clearInterval(logTimer);
    if (jobs.get(currentLog)?.status === "running") drawLog();
  }, 2000);
}

async function drawLog() {
  const text = await invoke("read_log", { id: currentLog });
  const box = $("log-text");
  const atBottom = box.scrollTop + box.clientHeight >= box.scrollHeight - 20;
  if ($("log-raw").checked) {
    box.textContent = text;
  } else {
    box.replaceChildren(...readableLog(text).map(([kind, line]) => el("div", { className: `log-line ${kind}`, textContent: line })));
  }
  if (atBottom) box.scrollTop = box.scrollHeight;
}

$("log-search").addEventListener("input", renderLogList);
$("log-errors-only").addEventListener("change", renderLogList);
$("log-raw").addEventListener("change", () => currentLog && drawLog());

/**
 * 記録を読みやすく整える: 進み具合の行は最後の1行にまとめ、目印を日本語の見出しに直し、
 * エラー・警告に説明を添える。
 */
function readableLog(text) {
  const out = [];
  let lastProgress = null;
  const flushProgress = () => {
    if (lastProgress) out.push(["progress", lastProgress]);
    lastProgress = null;
  };
  for (const raw of text.split("\n")) {
    const line = raw.trimEnd();
    if (!line) continue;
    if (line.startsWith("GRB_PROGRESS")) {
      const [percent, speed, eta] = line.replace("GRB_PROGRESS", "").split("|").map((x) => x.trim());
      lastProgress = `⬇ ${percent}  ${speed}${eta && eta !== "NA" && eta !== "Unknown" ? `  ${t("残り {time}", { time: eta })}` : ""}`;
      continue;
    }
    if (/^\[download\]\s+\d/.test(line)) continue; // 進み具合の原文は上でまとめる
    flushProgress();
    if (line.startsWith("> ")) {
      const url = line.split(" ").pop();
      out.push(["heading", `▶ ${t("開始")}  ${url}`]);
    } else if (line.startsWith("GRB_TITLE ")) {
      out.push(["info", `${t("タイトル")}: ${line.slice(10)}`]);
    } else if (line.startsWith("GRB_THUMB ")) {
      continue;
    } else if (line.startsWith("GRB_FILE ")) {
      out.push(["ok", `✓ ${t("保存しました")}: ${line.slice(9)}`]);
    } else if (line.startsWith("ERROR")) {
      out.push(["error", `✕ ${line.replace(/^ERROR:\s*/, "")}`]);
      const hint = describeMessage(line);
      if (hint && hint !== line) out.push(["hint", `→ ${hint}`]);
    } else if (line.startsWith("WARNING")) {
      out.push(["warn", `⚠ ${line.replace(/^WARNING:\s*/, "")}`]);
    } else {
      out.push(["", line]);
    }
  }
  flushProgress();
  return out;
}
$("log-refresh").addEventListener("click", loadLogs);
$("log-folder").addEventListener("click", () => invoke("open_logs_folder"));
$("log-clear").addEventListener("click", async () => {
  if (!confirm(t("記録をすべて消しますか?"))) return;
  await invoke("clear_logs");
  currentLog = null;
  $("log-text").textContent = "";
  loadLogs();
});
$("log-copy").addEventListener("click", () => navigator.clipboard.writeText($("log-text").textContent).catch(() => {}));
$("pv-close").addEventListener("click", () => $("preview").classList.add("hidden"));
let queueTimer = 0;
$("url").addEventListener("input", () => {
  updateGoButton();
  // URL だけが入ったら、少し待ってから待機中に入れる(打ち途中で入れないため)
  clearTimeout(queueTimer);
  const text = $("url").value.trim();
  if (text && text.split(/\s+/).every((word) => /^https?:\/\/\S+\.\S+/.test(word))) {
    queueTimer = setTimeout(queueInputUrls, 400);
  }
});
$("url").addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    go();
  }
});
// URL を貼ったらすぐ待機中に入れる(調べたいときは「調べる」を押す)
$("url").addEventListener("paste", () => setTimeout(() => {
  updateGoButton();
  clearTimeout(queueTimer);
  if (urlsInInput().length) queueInputUrls();
}));

async function queueInputUrls() {
  const urls = urlsInInput();
  if (!urls.length) return;
  $("url").value = "";
  urls.forEach(rememberUrl);
  // 自動で入れたものは「後で保存」へ。保存はそこから押す
  const known = new Set(laterList().map((x) => x.url));
  const fresh = urls.filter((url) => !known.has(url));
  await setLater([...laterList(), ...fresh.map((url) => ({ url, title: "", thumbnail: "", overrides: { ...presetValues } }))]);
  updateGoButton();
  $("later-box").open = true;
  const message = t("{n} 件を「後で保存」に入れました", { n: fresh.length });
  const skipped = urls.length - fresh.length;
  showStatus(skipped ? `${message}(${t("重複 {n} 件は除外", { n: skipped })})` : message);
}

// URL やリンクをドラッグ&ドロップで受け付ける
const dropZone = $("drop-zone");
dropZone.addEventListener("dragover", (e) => {
  e.preventDefault();
  dropZone.classList.add("dragging");
});
dropZone.addEventListener("dragleave", () => dropZone.classList.remove("dragging"));
dropZone.addEventListener("drop", (e) => {
  e.preventDefault();
  dropZone.classList.remove("dragging");
  if (e.dataTransfer.files.length) return importFiles(e.dataTransfer.files);
  const text = e.dataTransfer.getData("text/uri-list") || e.dataTransfer.getData("text/plain");
  if (!text) return;
  $("url").value = text.trim();
  updateGoButton();
  if (urlsInInput().length) queueInputUrls();
  else go();
});

// ───────── ダウンロード一覧 ─────────
$("clear").addEventListener("click", () => {
  const removed = [...jobs.values()].filter((j) => j.status === "done");
  replaceJobsLocally([...jobs.values()].filter((j) => j.status !== "done"), removed);
});
$("open-folder").addEventListener("click", () => invoke("open_folder", { path: settings.output_dir }));
$("open-dir").addEventListener("click", () => invoke("open_folder", { path: settings.output_dir }));

// 一覧は状態ごとのまとまりで見せる
const JOB_GROUPS = [
  { key: "waiting", title: "予定", test: (j) => j.status === "waiting" || j.status === "paused" },
  { key: "active", title: "DL中", test: (j) => j.status === "running" },
  { key: "done", title: "DL完了", test: (j) => j.status === "done" },
  { key: "failed", title: "エラー", test: (j) => j.status === "failed" || j.status === "canceled" },
];
const collapsed = {}; // まとまりごとの開閉(画面を開いている間だけ覚える)
const renderedJobs = new Map(); // id → { signature, element } 変わった項目だけ描き直す

function jobGroupElement(group) {
  let section = $(`group-${group.key}`);
  if (section) return section;
  section = el("section", { id: `group-${group.key}`, className: `job-group ${group.key}` },
    el("button", { type: "button", className: "group-head", onclick: () => {
      collapsed[group.key] = !isCollapsed(group.key);
      renderJobs();
    } }),
    el("ul", { className: "jobs" }),
  );
  $("job-groups").append(section);
  return section;
}

let jobFilter = "all"; // 集計の数字を押した時の絞り込み

function isCollapsed(key) {
  if (jobFilter !== "all") return false; // 絞り込み中は畳まない
  return collapsed[key] ?? (key === "done" && ui("collapse_done"));
}

function renderJobs() {
  const all = [...jobs.values()].reverse(); // 新しいものを上に
  const listClass = `jobs ${ui("list_style") === "list" ? "" : ui("list_style")}`;
  const groupFilter = { all: null, active: ["active"], waiting: ["waiting"], done: ["done"], failed: ["failed"] }[jobFilter];
  for (const group of JOB_GROUPS) {
    // 「予定」は始まる順(上から順に始まる)、ほかは新しいものを上に
    const source = group.key === "waiting" ? [...all].reverse() : all;
    const members = groupFilter && !groupFilter.includes(group.key) ? [] : source.filter(group.test);
    const section = jobGroupElement(group);
    section.classList.toggle("hidden", members.length === 0);
    section.classList.toggle("collapsed", isCollapsed(group.key));
    section.querySelector(".group-head").textContent = `${isCollapsed(group.key) ? "▸" : "▾"} ${t(group.title)}  ${members.length}`;
    const list = section.querySelector("ul");
    list.className = listClass;
    if (isCollapsed(group.key)) {
      list.replaceChildren();
      continue;
    }
    list.replaceChildren(...members.map(jobElement));
  }
  // 一覧から消えた項目の描画は捨てる
  for (const id of renderedJobs.keys()) if (!jobs.has(id)) renderedJobs.delete(id);
  $("jobs-empty").classList.toggle("hidden", all.length > 0);

  const count = (test) => all.filter(test).length;
  const active = count((j) => j.status === "running" || j.status === "waiting");
  const summary = $("job-summary");
  summary.replaceChildren();
  if (all.length) {
    // 押すとそのまとまりだけを表示する(もう一度押すと全部に戻る)
    const chip = (filter, className, text) => el("button", {
      type: "button",
      className: `summary-chip ${className} ${jobFilter === filter ? "active" : ""}`,
      textContent: text,
      onclick: () => {
        jobFilter = jobFilter === filter ? "all" : filter;
        renderJobs();
      },
    });
    const failed = count((j) => j.status === "failed" || j.status === "canceled");
    summary.append(
      chip("all", "", t("全 {n} 件", { n: all.length })),
      chip("active", "", t("進行中 {n}", { n: count((j) => j.status === "running") })),
      chip("waiting", "", `${t("待機中 {n}", { n: count((j) => j.status === "waiting" || j.status === "paused") })}`),
      chip("done", "ok", `✓ ${t("完了 {n}", { n: count((j) => j.status === "done") })}`),
      chip("failed", "bad", `✕ ${t("失敗 {n}", { n: failed })}`),
    );
    // まとめての操作
    $("bulk-actions").replaceChildren(
      failed ? button("失敗をまとめて再試行", async () => {
        await invoke("retry_failed");
        jobFilter = "all";
      }) : "",
      failed ? button("失敗を一覧から消す", () => {
        const removed = [...jobs.values()].filter((j) => j.status === "failed" || j.status === "canceled");
        replaceJobsLocally([...jobs.values()].filter((j) => !removed.includes(j)), removed);
      }) : "",
      active ? button("すべて一時停止", () => invoke("pause_all", { pause: true })) : "",
      count((j) => j.status === "paused") ? button("すべて再開", () => invoke("pause_all", { pause: false })) : "",
      active ? button("すべて中止", () => {
        if (confirm(t("待機中・ダウンロード中をすべて止めますか?"))) invoke("cancel_all");
      }) : "",
      failed ? button("失敗を書き出す", exportFailed) : "",
      afterDoneSelect(),
    );
  }
  watchAllDone(active);
  $("nav-badge").textContent = active;
  updateTaskbar();
  $("nav-badge").classList.toggle("hidden", active === 0);
}

// ───────── 全部終わったら(スリープ・シャットダウンなど) ─────────
// 1回かぎりの指定。終わったら元の「何もしない」に戻る
let afterDone = "";
let wasActive = 0;
let afterDoneTimer = null;
const AFTER_DONE = [["", "何もしない"], ["quit", "アプリを閉じる"], ["sleep", "スリープ"], ["shutdown", "シャットダウン"]];

function afterDoneSelect() {
  const select = el("select", { title: t("全部終わったら") },
    ...AFTER_DONE.map(([v, label]) => new Option(`${t("全部終わったら")}: ${t(label)}`, v, false, v === afterDone)));
  select.addEventListener("change", () => (afterDone = select.value));
  return el("label", { className: "after-done" }, select);
}

/** ダウンロード中が 0 件になった瞬間に、選んだ操作を 60 秒後に行う(その間は取り消せる) */
function watchAllDone(active) {
  const finishedNow = wasActive > 0 && active === 0;
  wasActive = active;
  if (!finishedNow || !afterDone || afterDoneTimer) return;
  const kind = afterDone;
  afterDone = "";
  let left = 60;
  const label = t(AFTER_DONE.find(([v]) => v === kind)[1]);
  const text = el("span");
  const banner = el("div", { className: "after-done-banner" }, text,
    button("今すぐ", () => run(), "small"),
    button("取り消す", () => stop(), "primary small"));
  const stop = () => {
    clearInterval(afterDoneTimer);
    afterDoneTimer = null;
    banner.remove();
  };
  const run = () => {
    stop();
    invoke("power_action", { kind }).catch((e) => alert(describeMessage(String(e))));
  };
  const tick = () => {
    text.textContent = t("ダウンロードが全部終わりました。{sec} 秒後に「{action}」します", { sec: left, action: label });
    if (left-- <= 0) run();
  };
  document.body.append(banner);
  tick();
  afterDoneTimer = setInterval(tick, 1000);
  invoke("show_window").catch(() => {});
}

/** 画質の上限ごとの大きさの目安(いちばん良い映像 + いちばん良い音声) */
function presetSize(formats = [], preset) {
  const limit = Number(preset) || Infinity;
  const video = formats.filter((f) => f.kind !== "audio" && f.filesize && (f.height || 0) <= limit).sort((a, b) => b.height - a.height || b.filesize - a.filesize)[0];
  if (!video) return 0;
  const audio = video.kind === "both" ? null : formats.filter((f) => f.kind === "audio" && f.filesize).sort((a, b) => b.filesize - a.filesize)[0];
  return video.filesize + (audio?.filesize || 0);
}

// ───────── 大きさと時間の見込み ─────────
// 最近のダウンロードの速さ(バイト/秒)をならして覚え、かかる時間の目安に使う
let averageSpeed = Number((() => { try { return localStorage.getItem("grabbit-speed"); } catch { return 0; } })()) || 0;
function noteSpeed(text = "") {
  const m = text.match(/([\d.]+)\s*([KMG]i?B)\/s/i);
  if (!m) return;
  const unit = { k: 1024, m: 1024 ** 2, g: 1024 ** 3 }[m[2][0].toLowerCase()];
  const bytes = Number(m[1]) * unit;
  averageSpeed = averageSpeed ? averageSpeed * 0.9 + bytes * 0.1 : bytes;
  try { localStorage.setItem("grabbit-speed", String(Math.round(averageSpeed))); } catch { /* 覚えられなくても困らない */ }
}
function estimateText(size) {
  if (!size) return "";
  const parts = [t("約 {size}", { size: formatSize(size) })];
  if (averageSpeed > 0) {
    const seconds = size / averageSpeed;
    parts.push(seconds < 60 ? t("約 {n} 秒", { n: Math.max(1, Math.round(seconds)) }) : t("約 {n} 分", { n: Math.round(seconds / 60) }));
  }
  return parts.join(" · ");
}
async function warnIfNoSpace(size) {
  if (!size) return;
  const status = await invoke("system_status").catch(() => null);
  if (status?.free_bytes != null && status.free_bytes < size * 1.1) {
    $("pv-notice").textContent = `⚠ ${t("保存先の空きが足りません(残り {free}、必要 {size})", { free: formatSize(status.free_bytes), size: formatSize(size) })}`;
    $("pv-notice").classList.remove("hidden");
  }
}

// ───────── 終わった時の音 ─────────
function playSound(ok) {
  const kind = ui("sound");
  if (!kind || kind === "off") return;
  try {
    const ctx = new AudioContext();
    const notes = { chime: ok ? [660, 880] : [440, 330], beep: ok ? [1000] : [400, 400], pop: ok ? [520] : [260] }[kind] || [];
    notes.forEach((freq, i) => {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = kind === "beep" ? "square" : "sine";
      osc.frequency.value = freq;
      const start = ctx.currentTime + i * 0.18;
      gain.gain.setValueAtTime(0.0001, start);
      gain.gain.exponentialRampToValueAtTime(0.25, start + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.0001, start + (kind === "pop" ? 0.12 : 0.3));
      osc.connect(gain).connect(ctx.destination);
      osc.start(start);
      osc.stop(start + 0.35);
    });
    setTimeout(() => ctx.close(), 1200);
  } catch {
    // 音が出せない環境でも何もしない
  }
}

// ───────── タスクバーの進み具合 ─────────
let lastTaskbar = "";
function updateTaskbar() {
  const running = [...jobs.values()].filter((j) => j.status === "running");
  const paused = [...jobs.values()].some((j) => j.status === "paused") && running.length === 0;
  const percent = running.length ? running.reduce((sum, j) => sum + (j.percent || 0), 0) / running.length : null;
  const key = `${percent == null ? "-" : Math.round(percent)}|${paused}`;
  if (key === lastTaskbar) return;
  lastTaskbar = key;
  invoke("set_taskbar_progress", { percent: percent == null ? null : Math.round(percent), paused }).catch(() => {});
}

// ───────── 選んでまとめて操作(Ctrl / Shift を押しながらクリック) ─────────
const selectedJobs = new Set();
let lastClickedJob = null;
function toggleSelection(job, event) {
  if (event.shiftKey && lastClickedJob != null) {
    const order = [...document.querySelectorAll("#job-groups .job")].map((e) => Number(e.dataset.id));
    const [a, b] = [order.indexOf(lastClickedJob), order.indexOf(job.id)].sort((x, y) => x - y);
    order.slice(a, b + 1).forEach((id) => selectedJobs.add(id));
  } else if (selectedJobs.has(job.id)) {
    selectedJobs.delete(job.id);
  } else {
    selectedJobs.add(job.id);
  }
  lastClickedJob = job.id;
  renderSelection();
}
function renderSelection() {
  document.querySelectorAll("#job-groups .job").forEach((e) => e.classList.toggle("selected", selectedJobs.has(Number(e.dataset.id))));
  const bar = $("select-bar");
  bar.classList.toggle("hidden", selectedJobs.size === 0);
  if (!selectedJobs.size) return;
  const picked = () => [...selectedJobs].map((id) => jobs.get(id)).filter(Boolean);
  const each = (fn) => async () => {
    for (const job of picked()) await fn(job);
    selectedJobs.clear();
    renderSelection();
  };
  bar.replaceChildren(
    el("b", { textContent: t("{n} 件を選択中", { n: selectedJobs.size }) }),
    button("一時停止", each((j) => ["running", "waiting"].includes(j.status) && invoke("pause_job", { id: j.id }))),
    button("再開", each((j) => j.status === "paused" && invoke("resume_job", { id: j.id }))),
    button("再試行", each((j) => ["failed", "canceled"].includes(j.status) && invoke("retry_job", { id: j.id }))),
    button("中止", each((j) => ["running", "waiting", "paused"].includes(j.status) && invoke("cancel_job", { id: j.id }))),
    button("一覧から消す", async () => {
      const removed = picked().filter((j) => !["running", "waiting"].includes(j.status));
      const remaining = [...jobs.values()].filter((j) => !removed.includes(j));
      selectedJobs.clear();
      renderSelection();
      replaceJobsLocally(remaining, removed);
    }),
    el("span", { className: "spacer" }),
    button("選ぶのをやめる", () => { selectedJobs.clear(); renderSelection(); }),
  );
}
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && selectedJobs.size) { selectedJobs.clear(); renderSelection(); }
});

// ───────── 元に戻す ─────────
let undoTimer;
function offerUndo(text, undo) {
  document.querySelector(".undo-bar")?.remove();
  clearTimeout(undoTimer);
  const bar = el("div", { className: "undo-bar", role: "status" },
    el("span", { textContent: text }),
    button("元に戻す", async () => { bar.remove(); await undo(); }, "ghost small"));
  document.body.append(bar);
  undoTimer = setTimeout(() => bar.remove(), 8000);
}
/** 一覧から消した分を、画面と記録の両方で消し、元に戻せるようにする */
async function replaceJobsLocally(remaining, removed) {
  if (!removed.length) return;
  // 画面の一覧だけでなく、アプリ側の一覧からも消す(完了・失敗・中止のものだけ)
  for (const job of removed) jobs.delete(job.id);
  await invoke("clear_jobs_by_id", { ids: removed.map((j) => j.id) }).catch(() => {});
  renderJobs();
  offerUndo(t("{n} 件を一覧から消しました", { n: removed.length }), async () => {
    await invoke("restore_jobs", { jobs: removed });
    removed.forEach((j) => jobs.set(j.id, j));
    renderJobs();
  });
}

/** 内容が変わった時だけ作り直す(大量に追加しても重くならないように) */
function jobElement(job) {
  const signature = JSON.stringify(job) + document.documentElement.lang + JSON.stringify(settings.ui);
  const cached = renderedJobs.get(job.id);
  if (cached && cached.signature === signature) return cached.element;
  const element = renderJob(job);
  element.tabIndex = 0;
  element.dataset.id = job.id;
  element.addEventListener("click", (e) => {
    if ((e.ctrlKey || e.metaKey || e.shiftKey) && !e.target.closest("button")) {
      e.preventDefault();
      toggleSelection(jobs.get(job.id) || job, e);
    }
  });
  element.oncontextmenu = (e) => openContextMenu(e, jobMenu(jobs.get(job.id) || job));
  // キーボードでもメニューを出せるように(Shift+F10 / メニューキー)
  element.onkeydown = (e) => {
    if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
      const r = element.getBoundingClientRect();
      openContextMenu({ preventDefault() {}, clientX: r.left + 40, clientY: r.top + 30 }, jobMenu(jobs.get(job.id) || job));
    }
  };
  renderedJobs.set(job.id, { signature, element });
  return element;
}

/** 状態の文字(ダウンロード中でも、準備中・変換中を区別して出す) */
function statusText(job) {
  if (job.status !== "running") return t(STATUS[job.status].text);
  if (job.phase === "processing") return t("変換・仕上げ中");
  if (job.phase === "downloading") return t("ダウンロード中");
  return t("準備中");
}

function primaryText(job) {
  switch (ui("primary_text")) {
    case "filename": return job.file_path ? fileName(job.file_path) : job.title;
    case "url": return job.url;
    default: return job.title;
  }
}

function renderJob(job) {
  const item = $("job-template").content.firstElementChild.cloneNode(true);
  const status = STATUS[job.status];
  item.classList.add(job.status);
  if (job.status === "running") item.classList.add(`phase-${job.phase || "preparing"}`);

  const thumb = item.querySelector(".thumb");
  if (ui("show_thumbnail")) {
    thumb.src = job.thumbnail || faviconOf(job.url);
  } else {
    item.querySelector(".thumb-wrap").classList.add("hidden");
  }
  item.querySelector(".state-icon").textContent = status.icon;
  item.querySelector(".state-icon").title = t(status.text);

  const favicon = item.querySelector(".favicon");
  if (ui("show_favicon")) favicon.src = faviconOf(job.url);
  else favicon.remove();
  item.querySelector(".title").textContent = primaryText(job);
  item.querySelector(".title").title = job.title;
  const engine = item.querySelector(".engine");
  if (ui("show_engine") && job.engine) engine.textContent = t(ENGINE_NAMES[job.engine] || job.engine);
  else engine.remove();
  item.querySelector(".bar span").style.width = `${job.percent}%`;

  const scheduled = job.status === "waiting" && job.start_at * 1000 > Date.now();
  const parts = [scheduled
    ? t("予約 {time}", { time: new Date(job.start_at * 1000).toLocaleString(document.documentElement.lang) })
    : statusText(job)];
  const mode = job.overrides?.mode;
  if (mode && mode !== "video") parts.push(t(MODES[mode]));
  if (job.from_subscription) parts.push(t("自動保存"));
  if (job.status === "running" && job.phase === "downloading") {
    parts.push(`${job.percent.toFixed(1)}%`);
    if (ui("show_speed")) parts.push(job.speed, job.eta && t("残り {time}", { time: job.eta }));
  }
  if (job.message) parts.push(describeMessage(job.message));
  item.querySelector(".meta").textContent = parts.filter(Boolean).join(" · ");

  const sub = [];
  if (ui("show_url")) sub.push(job.url);
  if (ui("show_path") && job.file_path) sub.push(job.file_path);
  const subLine = item.querySelector(".sub");
  subLine.textContent = sub.join("  ·  ");
  if (job.file_path) {
    subLine.classList.add("path-link");
    subLine.title = t("フォルダで表示");
    subLine.onclick = () => invoke("show_in_folder", { path: job.file_path });
  }

  const actions = item.querySelector(".actions");
  const removeJob = async () => {
    if (["waiting", "paused"].includes(job.status)) await invoke("cancel_job", { id: job.id }).catch(() => {});
    await invoke("clear_jobs_by_id", { ids: [job.id] }).catch(() => {});
    jobs.delete(job.id);
    renderJobs();
  };
  const move = (whereTo) => invoke("move_job", { id: job.id, whereTo });
  if (scheduled) {
    actions.append(
      button("今すぐ開始", () => invoke("start_now", { id: job.id })),
      button("中止", () => invoke("cancel_job", { id: job.id })),
    );
  } else if (job.status === "waiting") {
    actions.append(
      iconButton("⤒", "次に始める", () => move("top")),
      iconButton("↑", "上へ", () => move("up")),
      iconButton("↓", "下へ", () => move("down")),
      button("一時停止", () => invoke("pause_job", { id: job.id })),
      button("中止", () => invoke("cancel_job", { id: job.id })),
    );
  } else if (job.status === "paused") {
    actions.append(
      iconButton("↑", "上へ", () => move("up")),
      iconButton("↓", "下へ", () => move("down")),
      button("再開", () => invoke("resume_job", { id: job.id })),
      button("中止", () => invoke("cancel_job", { id: job.id })),
    );
  } else if (job.status === "running") {
    actions.append(
      button("一時停止", () => invoke("pause_job", { id: job.id })),
      button("中止", () => invoke("cancel_job", { id: job.id })),
    );
  } else if (job.status === "failed" || job.status === "canceled") {
    if (needsLogin(job.message)) actions.append(button("ログインする", () => startLogin(job.url, job.id), "primary small"));
    if (job.status === "failed") {
      actions.append(button("診断", () => runDiagnosis(job.url)));
      actions.append(button("似た報告を探す", () => searchSimilarReports(job.message)));
    }
    actions.append(
      button("再試行", () => invoke("retry_job", { id: job.id })),
      button("記録", () => {
        openTab("logs");
        openLog(job.id);
      }),
    );
  } else if (job.status === "done" && !job.file_path) {
    // サムネだけ・字幕だけなどは保存先フォルダを開く
    actions.append(button("フォルダ", () => invoke("open_folder", { path: job.overrides?.output_dir || settings.output_dir })));
  } else if (job.status === "done" && job.file_path) {
    actions.append(button("開く", () => invoke("open_file", { path: job.file_path })));
  }
  if (job.status !== "running") actions.append(button("削除", removeJob));
  return item;
}

// 進捗は頻繁に届くので、描画は1フレームに1回へまとめる
let renderPending = false;
// 順番を変えた時は、バックエンドの並びに合わせて入れ直す
listen("jobs_reordered", ({ payload }) => {
  const ordered = payload.map((id) => jobs.get(id)).filter(Boolean);
  jobs.clear();
  ordered.forEach((j) => jobs.set(j.id, j));
  renderJobs();
});

listen("job", ({ payload }) => {
  const before = jobs.get(payload.id);
  jobs.set(payload.id, payload);
  if (payload.status === "failed" && before?.status !== "failed") maybePromptRelogin(payload);
  if (payload.status === "running" && payload.speed) noteSpeed(payload.speed);
  if (["done", "failed"].includes(payload.status) && before && before.status !== payload.status && payload.message !== "@skipped") {
    playSound(payload.status === "done");
  }
  if (renderPending) return;
  renderPending = true;
  requestAnimationFrame(() => {
    renderPending = false;
    renderJobs();
  });
});

// ───────── 対応サイト一覧 ─────────
let sites = null; // [[名前, 不調か], ...]

/** 抽出器名からドメインを推測してアイコンを出す(分からなければ地球儀になる) */
function siteDomain(name) {
  const base = name.split(":")[0].toLowerCase();
  return base.includes(".") ? base : `${base}.com`;
}

async function loadSites() {
  if (sites) return renderSites();
  $("site-count").textContent = t("読み込み中…");
  try {
    sites = await invoke("list_sites");
  } catch (e) {
    $("site-count").textContent = `${t("読み込めませんでした")}: ${e}`;
    return;
  }
  renderSites();
}

function renderSites() {
  if (!sites) return;
  const query = $("site-search").value.trim().toLowerCase();
  const hideBroken = $("site-hide-broken").checked;
  const matched = sites
    .filter(([name, broken]) => (!hideBroken || !broken) && name.toLowerCase().includes(query))
    .sort(([a], [b]) => popularityRank(a) - popularityRank(b) || a.localeCompare(b));
  const working = sites.filter(([, broken]) => !broken).length;
  $("site-count").textContent = t("{shown} 件表示 / 全 {total} 件(正常 {ok})", {
    shown: matched.length, total: sites.length, ok: working,
  });
  // 数が多いので先頭 300 件まで(検索で絞り込める)
  $("site-list").replaceChildren(...matched.slice(0, 300).map(([name, broken]) => el("li",
    { className: broken ? "broken" : "" },
    el("img", { loading: "lazy", src: faviconOf(siteDomain(name)) }),
    el("span", { className: "name", textContent: name, title: name }),
    broken && el("span", { className: "badge bad", textContent: t("不調") }),
  )));
}

$("site-search").addEventListener("input", renderSites);
$("site-hide-broken").addEventListener("change", renderSites);

// ───────── 自動保存(登録チャンネル) ─────────
/** 自動保存の入力欄: URL からサイトを判別して、そのサイトで使える項目だけを出す */
function renderSubServices() {
  const url = $("sub-url").value.trim().split(/\s+/)[0];
  const service = url ? detectService(url) : serviceById("other");
  const modes = service.modes || ["video", "audio", "thumbnail", "subtitles", "info"];
  const current = $("sub-mode").value || settings.mode;
  $("sub-mode").replaceChildren(...modes.map((m) => new Option(t(MODES[m]), m)));
  $("sub-mode").value = modes.includes(current) ? current : modes[0];
  if (!$("sub-preset").dataset.touched) $("sub-preset").value = settings.preset?.startsWith("audio") ? "best" : settings.preset || "best";
  $("sub-skip-shorts-label").classList.toggle("hidden", Boolean(url) && !service.shorts);
  $("sub-skip-live-label").classList.toggle("hidden", Boolean(url) && !service.live);
}
function selectSubService() {
  renderSubServices();
  renderPresets();
}
$("sub-url").addEventListener("input", renderSubServices);
$("sub-preset").addEventListener("change", () => ($("sub-preset").dataset.touched = "1"));

$("sub-add").addEventListener("click", async () => {
  // 改行・空白・カンマ区切りでいくつでもまとめて登録できる
  const urls = [...new Set($("sub-url").value.split(/[\s,]+/).filter((u) => /^https?:\/\//.test(u)))];
  if (!urls.length) return $("sub-url").focus();
  const overrides = {
    mode: $("sub-mode").value,
    preset: $("sub-preset").value,
    skip_shorts: $("sub-skip-shorts").checked,
    skip_live: $("sub-skip-live").checked,
  };
  $("sub-url").value = "";
  const failed = [];
  for (const url of urls) {
    try {
      await invoke("add_subscription", {
        url, newOnly: $("sub-new-only").checked, overrides, recordLive: $("sub-record-live").checked,
        group: $("sub-group").value.trim(), notifyOnly: $("sub-notify-only").checked,
      });
    } catch (e) {
      failed.push(`${url}\n  ${describeMessage(String(e))}`);
    }
  }
  if (failed.length) alert(`${t("登録できませんでした")}:\n${failed.join("\n")}`);
});
$("sub-check").addEventListener("click", () => invoke("check_subscriptions_now"));

let lastSubs = [];
const collapsedGroups = new Set();
function renderSubs(subs) {
  lastSubs = subs;
  const groups = [...new Set(subs.map((s) => s.group || ""))].sort((x, y) => (x === "") - (y === "") || x.localeCompare(y));
  $("sub-groups").replaceChildren(...groups.filter(Boolean).map((g) => new Option(g)));
  const rows = [];
  for (const group of groups) {
    const members = subs.filter((s) => (s.group || "") === group);
    if (groups.length > 1) {
      const closed = collapsedGroups.has(group);
      rows.push(el("li", {
        className: "sub-group", tabIndex: 0, role: "button", "aria-expanded": String(!closed),
        textContent: `${closed ? "▸" : "▾"} ${group || t("未分類")} (${members.length})`,
        onclick: () => { closed ? collapsedGroups.delete(group) : collapsedGroups.add(group); renderSubs(lastSubs); },
      }));
      if (closed) continue;
    }
    rows.push(...members.map(subscriptionRow));
  }
  $("subs").replaceChildren(...rows);
  $("subs-empty").classList.toggle("hidden", subs.length > 0);
}

function subscriptionRow(sub) {
  const last = sub.last_checked ? new Date(sub.last_checked * 1000).toLocaleString(document.documentElement.lang) : t("未確認");
  const interval = sub.check_times?.length ? t("毎日 {times}", { times: sub.check_times.join("・") })
    : sub.interval_minutes ? t("{n} 分ごと", { n: sub.interval_minutes }) : "";
  return el("li", { className: `job ${sub.enabled ? "" : "canceled"}` },
    el("img", { className: "favicon", src: faviconOf(sub.url), alt: "" }),
    el("div", { className: "info" },
      el("div", { className: "title", textContent: sub.name }),
      el("div", { className: "sub muted", textContent: [
        sub.engine === "gallery" ? t("画像・投稿") : sub.overrides?.mode && t(MODES[sub.overrides.mode]),
        sub.notify_only && t("知らせるだけ"),
        interval,
        sub.url,
        t("最終確認 {time}", { time: last }),
      ].filter(Boolean).join(" · ") }),
    ),
    el("div", { className: "actions" },
      button("編集", () => editSubscription(sub)),
      button(sub.record_live ? "配信録画: オン" : "配信録画: オフ", () => invoke("toggle_live_recording", { id: sub.id })),
      button(sub.enabled ? "停止" : "再開", () => invoke("toggle_subscription", { id: sub.id })),
      button("削除", () => {
        if (confirm(t("「{name}」の登録を解除しますか?(保存済みの動画は消えません)", { name: sub.name }))) {
          invoke("remove_subscription", { id: sub.id });
        }
      }),
    ),
  );
}

/** 登録先のグループ・確認間隔・知らせるだけを変える */
function editSubscription(sub) {
  const group = prompt(t("グループ(空欄で未分類)"), sub.group || "");
  if (group === null) return;
  const minutes = prompt(t("確認する間隔(分)。0 で設定と同じ"), String(sub.interval_minutes || 0));
  if (minutes === null) return;
  const times = prompt(t("決まった時刻に確認する(例: 7:00, 19:30。空欄で間隔どおり)"), (sub.check_times || []).join(", "));
  if (times === null) return;
  const notifyOnly = confirm(t("新着を知らせるだけにしますか?(OK: 知らせるだけ / キャンセル: 自動で保存)"));
  invoke("update_subscription", {
    id: sub.id, group, intervalMinutes: Math.max(0, Number(minutes) || 0), notifyOnly,
    checkTimes: times.split(/[,、\s]+/).filter(Boolean),
  });
}
// ───────── 保存プリセットの編集 ─────────
// 各項目は「設定に従う」(= 値を持たない)を選べる。選んだ項目だけがプリセットの中身になる
const FOLLOW = "";
const yesNo = [["true", "する"], ["false", "しない"]];
const PRESET_FIELDS = [
  ["基本", [
    ["mode", "保存するもの", () => Object.entries(MODES)],
  ]],
  ["動画", [
    ["preset", "画質", () => [["best", "最高画質"], ["2160", "4K まで"], ["1440", "1440p まで"], ["1080", "1080p まで"], ["720", "720p まで"], ["480", "480p まで"], ["360", "360p まで"]]],
    ["container", "形式", () => [["mp4", "MP4(どこでも再生できる)"], ["mkv", "MKV(何でも入る)"], ["webm", "WebM"]]],
    ["video_codec", "コーデック", () => [["any", "おまかせ"], ["h264", "H.264(互換性重視)"], ["vp9", "VP9"], ["av1", "AV1(高圧縮)"]]],
    ["recode_video", "動画の再エンコード", () => [["mp4", "MP4"], ["mkv", "MKV"], ["webm", "WebM"], ["mov", "MOV"]]],
  ]],
  ["音声", [
    ["audio_format", "音声の形式", () => [["mp3", "MP3"], ["m4a", "M4A"], ["opus", "Opus"], ["flac", "FLAC"], ["wav", "WAV"]]],
    ["audio_bitrate", "音質", () => [["320K", "320kbps"], ["192K", "192kbps"], ["128K", "128kbps"]]],
  ]],
  ["字幕", [
    ["subs", "字幕", () => [["none", "保存しない"], ["embed", "動画に埋め込む"], ["file", "別ファイルで保存"], ["both", "埋め込み + 別ファイル"], ["burn", "映像に焼き付ける"]]],
    ["subtitle_langs", "字幕の言語", "text"],
    ["auto_subtitles", "自動生成字幕も使う", () => yesNo],
  ]],
  ["一緒に保存するもの", [
    ["embed_thumbnail", "サムネイルを埋め込む", () => yesNo],
    ["embed_metadata", "タイトル・チャプター等を埋め込む", () => yesNo],
    ["write_thumbnail_file", "サムネイルを別ファイルで保存", () => yesNo],
    ["write_description", "概要欄を保存", () => yesNo],
    ["write_comments", "コメントを保存", () => yesNo],
    ["write_live_chat", "ライブのチャットを保存", () => yesNo],
    ["write_info_json", "動画情報(JSON)を保存", () => yesNo],
    ["sponsorblock_remove", "スポンサー区間を自動カット (YouTube)", () => yesNo],
    ["split_chapters", "チャプターごとにファイルを分ける", () => yesNo],
  ]],
  ["保存先", [
    ["output_dir", "保存先フォルダ", "text"],
    ["filename_template", "ファイル名・フォルダ分け", "filename"],
  ]],
];
let editingPreset = "standard";

/** 字幕の4つの値 ⇄ 1つの選択肢 */
function subsChoice(v) {
  if (!("burn_subtitles" in v || "embed_subtitles" in v || "write_subtitle_file" in v)) return FOLLOW;
  return v.burn_subtitles ? "burn" : v.embed_subtitles && v.write_subtitle_file ? "both"
    : v.embed_subtitles ? "embed" : v.write_subtitle_file ? "file" : "none";
}
function applySubsChoice(v, choice) {
  delete v.burn_subtitles; delete v.embed_subtitles; delete v.write_subtitle_file;
  if (choice === FOLLOW) return;
  v.burn_subtitles = choice === "burn";
  v.embed_subtitles = choice === "embed" || choice === "both";
  v.write_subtitle_file = choice === "file" || choice === "both";
}

/** 設定画面の値を、プリセットと同じ形(項目名 → 値)にして返す */
function mySettingsAsPreset() {
  const keys = PRESET_FIELDS.flatMap(([, fields]) => fields.map(([key]) => key))
    .flatMap((key) => (key === "subs" ? ["burn_subtitles", "embed_subtitles", "write_subtitle_file"] : [key]));
  const values = Object.fromEntries(keys.filter((key) => key in settings).map((key) => [key, settings[key]]));
  const meta = settings.ui?.mine || {};
  return { id: "", icon: meta.icon || "◇", name: meta.name || t("自分の設定"), note: meta.note || t("設定画面の値どおり"), values };
}

function renderPresetEditor() {
  // 最初は「自分の設定」と自分で保存したものだけ。標準のプリセットは設定で出せる
  const presets = allPresets();
  if (!presets.some((p) => p.id === editingPreset)) editingPreset = mineHidden() ? presets[0].id : "";
  const mine = el("li", {
    className: editingPreset ? "" : "active",
    onclick: () => { editingPreset = ""; renderPresetEditor(); },
  },
    el("span", { className: "preset-icon", textContent: mySettingsAsPreset().icon }),
    el("span", { textContent: mySettingsAsPreset().name }),
    !activePreset && el("small", { className: "badge ok", textContent: t("使用中") }));
  $("preset-list").replaceChildren(mineHidden() ? "" : mine, ...presets.map((p) => el("li", {
    className: `${p.id === editingPreset ? "active" : ""}`,
    onclick: () => { editingPreset = p.id; renderPresetEditor(); },
  },
    el("span", { className: "preset-icon", textContent: p.icon || "◆" }),
    el("span", { textContent: p.name }),
    p.id === activePreset && el("small", { className: "badge ok", textContent: t("使用中") }))));

  // 「自分の設定」は設定画面の値そのものを、同じ形で編集する
  const isMine = !editingPreset;
  const preset = isMine ? mySettingsAsPreset() : presets.find((p) => p.id === editingPreset);
  const values = { ...preset.values };
  const nameInput = el("input", { value: preset.name, maxLength: 30 });
  const iconInput = el("input", { value: preset.icon || "◆", maxLength: 2, className: "icon-input" });
  const noteInput = el("input", { value: preset.note || "", maxLength: 60 });

  const sections = PRESET_FIELDS.map(([title, fields]) => el("fieldset", {},
    el("legend", { textContent: t(title) }),
    el("div", { className: "grid2" }, ...fields.map(([key, label, kind]) => {
      if (kind === "filename") {
        const hidden = el("input", { type: "hidden", value: values.filename_template || settings.filename_template });
        const host = el("div", { className: "fn-host" });
        const follow = el("label", { className: `check ${isMine ? "hidden" : ""}` }, el("input", { type: "checkbox", checked: !isMine && !values.filename_template }), t("設定に従う"));
        const box = el("div", { className: "fn-field wide" }, el("span", { className: "fn-title", textContent: t(label) }), follow, host);
        const sync = () => {
          host.classList.toggle("hidden", follow.firstChild.checked);
          if (follow.firstChild.checked) delete values.filename_template; else values.filename_template = hidden.value;
        };
        FilenameBuilder.mount(host, hidden);
        hidden.addEventListener("change", sync);
        follow.firstChild.addEventListener("change", sync);
        sync();
        return box;
      }
      if (kind === "text") {
        const input = el("input", { value: values[key] || "", placeholder: isMine ? "" : t("空欄で設定に従う") });
        const store = () => { if (input.value.trim() || isMine) values[key] = input.value.trim(); else delete values[key]; };
        input.addEventListener("input", store);
        if (key !== "output_dir") return el("label", {}, t(label), input);
        // 保存先はフォルダを選んで決められるようにする
        const pick = button("選ぶ", async () => {
          const folder = await pickFolder(input.value || settings.output_dir);
          if (!folder) return;
          input.value = folder;
          store();
        }, "ghost");
        return el("label", {}, t(label), el("div", { className: "row" }, input, pick));
      }
      const select = el("select", {}, !isMine && new Option(t("設定に従う"), FOLLOW),
        ...kind().map(([v, text]) => new Option(t(text), v)));
      select.value = key === "subs" ? subsChoice(values) : key in values ? String(values[key]) : FOLLOW;
      select.addEventListener("change", () => {
        if (key === "subs") return applySubsChoice(values, select.value);
        if (select.value === FOLLOW) delete values[key];
        else values[key] = yesNo.some(([v]) => v === select.value) ? select.value === "true" : select.value;
      });
      return el("label", {}, t(label), select);
    }))));

  const save = async () => {
    const meta = { icon: iconInput.value.trim() || "◆", name: nameInput.value.trim() || t("保存した設定"), note: noteInput.value.trim() };
    settings.ui = settings.ui || {};
    if (isMine) {
      // 「自分の設定」の中身は設定画面の値そのもの。名前・アイコン・説明だけ別に持つ
      Object.assign(settings, values);
      settings.ui.mine = meta;
      await persistSettings();
      fillForm();
      renderPresets();
      return renderPresetEditor();
    }
    settings.ui.presets = settings.ui.presets.map((p) => (p.id === preset.id ? { ...p, ...meta, values } : p));
    await persistSettings();
    if (activePreset === preset.id) presetValues = { ...values };
    renderPresets();
    renderPresetEditor();
  };
  const removeMine = async () => {
    if (!(settings.ui?.presets || []).length) return alert(t("先に保存プリセットを1つ作ってください。「自分の設定」だけの時は消せません"));
    if (!confirm(t("「自分の設定」を一覧から消しますか?(設定画面の値はそのまま残ります)"))) return;
    settings.ui = { ...settings.ui, hide_mine: true };
    if (!activePreset) applyPreset(allPresets()[0]);
    await persistSettings();
    editingPreset = "";
    renderPresets();
    renderPresetEditor();
  };
  const actions = el("div", { className: "row" },
    button("保存", save, "primary"),
    button("複製", async () => {
      const copy = { id: `custom-${Date.now()}`, icon: iconInput.value || "◆", name: `${nameInput.value} (2)`, note: noteInput.value, values: { ...values } };
      settings.ui = { ...(settings.ui || {}), presets: [...(settings.ui?.presets || []), copy] };
      await persistSettings();
      editingPreset = copy.id;
      renderPresets();
      renderPresetEditor();
    }),
    button("これを使う", async () => {
      await save();
      if (isMine) clearPreset(); else applyPreset(allPresets().find((p) => p.id === preset.id));
      renderPresetEditor();
    }),
    el("span", { className: "spacer" }),
    button("削除", async () => {
      if (isMine) return removeMine();
      if (!confirm(t("プリセット「{name}」を削除しますか?", { name: preset.name }))) return;
      await deletePreset(preset.id);
      renderPresetEditor();
    }, "ghost small danger"),
  );
  $("preset-form").replaceChildren(
    isMine && el("p", { className: "muted", textContent: t("ここで変えると設定画面の値も変わります") }),
    el("div", { className: "preset-head" },
      el("label", {}, t("アイコン"), iconInput),
      el("label", { className: "grow" }, t("名前"), nameInput),
      el("label", { className: "grow" }, t("説明"), noteInput)),
    ...sections,
    actions,
  );
}
$("preset-new").addEventListener("click", async () => {
  const preset = { id: `custom-${Date.now()}`, icon: "◆", name: t("保存した設定"), note: "", values: { mode: "video" } };
  settings.ui = { ...(settings.ui || {}), presets: [...(settings.ui?.presets || []), preset] };
  await persistSettings();
  editingPreset = preset.id;
  renderPresets();
  renderPresetEditor();
});

// ───────── ログイン(会員限定・年齢制限などの動画用) ─────────
// [表示名, ログイン画面, そのサイトのドメイン(動画の URL から、どのログイン画面を開くか決める)]
const LOGIN_SITES = [
  ["YouTube", "https://accounts.google.com/ServiceLogin?continue=https://www.youtube.com/", ["youtube.com", "youtu.be"]],
  ["ニコニコ", "https://account.nicovideo.jp/login", ["nicovideo.jp", "nico.ms"]],
  ["X", "https://x.com/i/flow/login", ["x.com", "twitter.com"]],
  ["Instagram", "https://www.instagram.com/accounts/login/", ["instagram.com"]],
  ["TikTok", "https://www.tiktok.com/login", ["tiktok.com"]],
  ["Twitch", "https://www.twitch.tv/login", ["twitch.tv"]],
  ["Facebook", "https://www.facebook.com/login", ["facebook.com", "fb.watch"]],
  ["pixivFANBOX", "https://www.fanbox.cc/login", ["fanbox.cc"]],
  ["Patreon", "https://www.patreon.com/login", ["patreon.com"]],
  ["bilibili", "https://passport.bilibili.com/login", ["bilibili.com", "b23.tv"]],
];
let jobsWaitingForLogin = [];
const reloginAsked = new Set(); // 同じサイトで何度も開かないよう、1回の起動で1度だけ

/**
 * ログインが必要で失敗した時、ログインの方法が「Grabbit の中でログイン」ならログイン画面を自動で開く。
 * 閉じたら、そのサイトで待っていたものをまとめてやり直す。
 */
function maybePromptRelogin(job) {
  if (!ui("auto_relogin") || settings.cookies_browser !== "grabbit" || !needsLogin(job.message)) return;
  let host;
  try {
    host = new URL(job.url).hostname.split(".").slice(-2).join(".");
  } catch {
    return;
  }
  if (reloginAsked.has(host)) {
    jobsWaitingForLogin.push(job.id); // もう開いている(開いた)サイトなら、やり直す一覧に足すだけ
    return;
  }
  reloginAsked.add(host);
  showStatus(t("ログインが必要なため、ログイン画面を開きました。ログインして閉じると自動でやり直します"));
  startLogin(job.url, job.id);
}

/** エラー文がログイン関係か */
function needsLogin(message = "") {
  return /needs_login|ログイン|sign in|login|cookies|members|private video|age[- ]restricted|not a bot/i.test(message);
}

/** ログイン画面を開く。閉じたら設定を「Grabbit の中でログイン」にして、待っていたダウンロードをやり直す */
async function startLogin(url, jobId = null) {
  if (jobId != null) jobsWaitingForLogin.push(jobId);
  const host = new URL(url).hostname;
  const site = LOGIN_SITES.find(([, , domains]) => domains.some((d) => host === d || host.endsWith(`.${d}`)));
  try {
    await invoke("open_login", { url: site ? site[1] : url, title: t("ログインして、終わったらこの画面を閉じてください") });
  } catch (e) {
    alert(describeMessage(String(e)));
  }
}

async function renderLogins() {
  const sites = await invoke("login_sites");
  $("login-sites-quick").replaceChildren(...LOGIN_SITES.map(([name, url]) => el("button", {
    type: "button", className: "login-chip", onclick: () => startLogin(url),
  }, el("img", { className: "favicon", src: faviconOf(url) }), t(name))));
  $("login-list").replaceChildren(...(sites.length ? sites.map(({ site, expires }) => {
    const state = loginState(expires);
    return el("li", {},
      el("img", { className: "favicon", src: faviconOf(`https://${site}`) }),
      el("span", { textContent: site }),
      el("small", { className: `login-expiry ${state.kind}`, textContent: state.text }),
      state.kind !== "ok" && button("ログインし直す", () => startLogin(`https://${site}`), "primary small"),
      button("ログアウト", async () => {
        await invoke("forget_login", { site });
        renderLogins();
      }));
  }) : [el("li", { className: "muted", textContent: t("まだありません") })]));
  return sites;
}

/** ログインの期限の表示: 切れた / もうすぐ(7日以内) / まだ大丈夫 */
function loginState(expires) {
  if (!expires) return { kind: "ok", text: t("期限なし") };
  const days = (expires * 1000 - Date.now()) / 86_400_000;
  const date = new Date(expires * 1000).toLocaleDateString(document.documentElement.lang);
  if (days < 0) return { kind: "bad", text: t("期限切れ({date})", { date }) };
  if (days < 7) return { kind: "warn", text: t("もうすぐ切れます({date})", { date }) };
  return { kind: "ok", text: t("期限 {date}", { date }) };
}

/** 起動時に、切れた・もうすぐ切れるログインがあれば知らせる */
async function warnLoginExpiry() {
  const sites = await invoke("login_sites").catch(() => []);
  const soon = sites.filter(({ expires }) => loginState(expires).kind !== "ok").map(({ site }) => site);
  if (soon.length) showStatus(t("ログインが切れた・もうすぐ切れるサイトがあります: {sites}(設定 → ログインが必要な動画用 でログインし直せます)", { sites: soon.join(", ") }), "error");
}

$("login-open").addEventListener("click", () => {
  const url = $("login-url").value.trim();
  if (/^https?:\/\//.test(url)) startLogin(url);
  else $("login-url").focus();
});
// payload: ログインした印が見つかったか
listen("login-saved", async ({ payload: loggedIn }) => {
  if (loggedIn && settings.cookies_browser !== "grabbit") {
    settings.cookies_browser = "grabbit";
    const field = document.querySelector('[name="cookies_browser"]');
    if (field) field.value = "grabbit";
    await persistSettings();
  }
  renderLogins();
  if (loggedIn) for (const id of new Set(jobsWaitingForLogin)) invoke("retry_job", { id });
  jobsWaitingForLogin = [];
  reloginAsked.clear();
});
renderLogins();

listen("subscriptions", ({ payload }) => renderSubs(payload));

// ───────── 設定(変更したら自動保存) ─────────
const form = $("settings");

function fillLanguageSelect() {
  const select = $("language-select");
  select.replaceChildren(new Option(t("OS に合わせる"), "auto"));
  for (const [code, info] of Object.entries(LANGUAGES)) select.append(new Option(info.name, code));
}

/** 設定の見出し一覧(左側)を作る。押すとその場所へ移動する */
function buildSettingsIndex() {
  $("settings-index").replaceChildren(...[...form.querySelectorAll("fieldset[id]")].map((fieldset) =>
    el("button", {
      type: "button",
      textContent: fieldset.querySelector("legend").textContent,
      onclick: () => fieldset.scrollIntoView({ behavior: "smooth", block: "start" }),
    })));
}

/** 設定を文字で絞り込む(項目名・説明に含まれるものだけ出す) */
function filterSettings() {
  const query = $("settings-search").value.trim().toLowerCase();
  for (const fieldset of form.querySelectorAll("fieldset")) {
    const legendHit = fieldset.querySelector("legend").textContent.toLowerCase().includes(query);
    let any = false;
    for (const label of fieldset.querySelectorAll("label")) {
      const hit = !query || legendHit || label.textContent.toLowerCase().includes(query);
      label.classList.toggle("search-hidden", !hit);
      any ||= hit;
    }
    fieldset.classList.toggle("search-hidden", Boolean(query) && !any && !legendHit);
  }
}
$("settings-search").addEventListener("input", filterSettings);

function fillModeSelect() {
  $("settings-mode").replaceChildren(...Object.entries(MODES).map(([value, label]) => new Option(t(label), value)));
}

function fillForm() {
  for (const [key, value] of Object.entries(settings)) {
    const field = form.elements[key];
    if (!field || !field.name) continue;
    if (field.type === "checkbox") field.checked = value;
    else field.value = value;
    // 選択肢に無い値(手で書いた古い設定など)も消さずに出す
    if (field.tagName === "SELECT" && value && field.value !== String(value)) field.add(new Option(value, value, true, true));
  }
  for (const field of form.querySelectorAll("[data-ui]")) {
    const value = ui(field.dataset.ui);
    if (field.type === "checkbox") field.checked = value;
    else field.value = value;
  }
  $("sub-interval").textContent = t("{n} 分ごとに自動確認", { n: settings.check_interval_minutes });
}

function readForm() {
  for (const field of form.elements) {
    if (field.name) {
      if (field.type === "checkbox") settings[field.name] = field.checked;
      else if (field.type === "number") settings[field.name] = Math.max(Number(field.min) || 0, Number(field.value) || 0);
      else settings[field.name] = field.value;
    } else if (field.dataset.ui) {
      const key = field.dataset.ui;
      settings.ui = settings.ui || {};
      if (field.type === "checkbox") settings.ui[key] = field.checked;
      else if (field.type === "range") settings.ui[key] = Number(field.value);
      else settings.ui[key] = field.value;
    }
  }
}

let savedTimer;
async function persistSettings() {
  await invoke("save_settings", { settings });
  $("saved").textContent = `✓ ${t("保存しました")}`;
  clearTimeout(savedTimer);
  savedTimer = setTimeout(() => ($("saved").textContent = ""), 1500);
}

function refreshAll() {
  applyAppearance();
  fillLanguageSelect();
  fillModeSelect();
  fillRegionSelects();
  fillForm();
  settingsFilename.refresh();
  renderSubtitleLangPicker();
  renderNavEditor();
  renderFolderRules();
  fillColorInputs();
  renderLogins();
  renderSiteMethods();
  renderMenuEditor();
  renderNtfyNames();
  addHelpIcons();
  renderSaveLocation();
  buildSettingsIndex();
  renderServices();
  renderSubServices();
  renderPresets();
  renderSiteStrip();
  updateGoButton();
  renderJobs();
  renderSubs(lastSubs);
  renderSites();
  renderLibrary();
  sendDesktopTexts();
}

form.addEventListener("input", (e) => {
  // スライダーと色は動かしている最中から反映する
  if (e.target.type === "range" || e.target.type === "color") {
    readForm();
    applyAppearance();
  }
});
form.addEventListener("change", async (e) => {
  readForm();
  await persistSettings();
  if (e.target.dataset.ui) refreshAll();
  if (e.target.name === "update_channel") updateYtdlp();
  if (e.target.name === "check_interval_minutes") fillForm();
});

// ───────── 保存先の自動振り分け ─────────
const RULE_KINDS = [["", "すべて"], ["video", "動画"], ["audio", "音声だけ"], ["thumbnail", "サムネイルだけ"], ["subtitles", "字幕だけ"], ["info", "概要・情報だけ"]];

function renderFolderRules() {
  const rules = settings.folder_rules || [];
  const save = async () => {
    settings.folder_rules = rules;
    await persistSettings();
  };
  $("folder-rules").replaceChildren(...rules.map((rule, i) => {
    const kind = el("select", {}, ...RULE_KINDS.map(([v, label]) => new Option(t(label), v, false, v === rule.kind)));
    const site = el("input", { value: rule.site, placeholder: t("サイト(例: youtube)。空欄ですべて") });
    const folder = el("input", { value: rule.folder, placeholder: t("フォルダ(例: 音楽 / D:\\Music)") });
    kind.addEventListener("change", () => { rule.kind = kind.value; save(); });
    site.addEventListener("change", () => { rule.site = site.value.trim(); save(); });
    folder.addEventListener("change", () => { rule.folder = folder.value.trim(); save(); });
    return el("li", {}, kind, site, el("span", { textContent: "→" }), folder,
      iconButton("↑", "上へ", () => { if (i > 0) { [rules[i - 1], rules[i]] = [rules[i], rules[i - 1]]; save().then(renderFolderRules); } }),
      iconButton("✕", "削除", () => { rules.splice(i, 1); save().then(renderFolderRules); }));
  }));
}
$("folder-rule-add").addEventListener("click", async () => {
  settings.folder_rules = [...(settings.folder_rules || []), { kind: "audio", site: "", folder: t("音楽") }];
  await persistSettings();
  renderFolderRules();
});

// ───────── プロキシを試す ─────────
document.querySelectorAll(".proxy-test").forEach((btn) => btn.addEventListener("click", async () => {
  const name = btn.dataset.for;
  const result = document.querySelector(`.proxy-result[data-for="${name}"]`);
  result.textContent = t("試しています…");
  result.className = "proxy-result muted";
  try {
    const ms = await invoke("test_proxy", { proxy: form.elements[name].value });
    result.textContent = `✓ ${t("つながりました({ms} ミリ秒)", { ms })}`;
    result.className = "proxy-result ok";
  } catch (e) {
    result.textContent = `✕ ${describeMessage(String(e))}`;
    result.className = "proxy-result bad";
  }
}));

// ───────── 接続の診断 ─────────
async function runDiagnosis(url = "") {
  openTab("settings");
  const box = $("diagnosis");
  box.closest("fieldset").scrollIntoView({ behavior: "smooth" });
  box.replaceChildren(el("li", { className: "muted", textContent: t("調べています…(30 秒ほどかかります)") }));
  $("diagnose").disabled = true;
  try {
    const rows = await invoke("diagnose", { url: url || null });
    const icon = { ok: "✓", warn: "⚠", bad: "✕" };
    const copyText = [
      `Grabbit ${await invoke("app_version").catch(() => "")} / ${navigator.userAgent.match(/\(([^)]+)\)/)?.[1] || ""}`,
      url && `URL: ${url}`,
      ...rows.map(([name, status, detail]) => `${icon[status]} ${t(name)}: ${detail}`),
    ].filter(Boolean).join("\n");
    const copy = button("結果をコピー", async () => {
      try {
        await navigator.clipboard.writeText(copyText);
        copy.textContent = `✓ ${t("コピーしました")}`;
      } catch {
        copy.textContent = t("コピーできませんでした");
      }
    }, "ghost small");
    box.replaceChildren(...rows.map(([name, status, detail]) => el("li", { className: `diag ${status}` },
      el("b", { textContent: `${icon[status]} ${t(name)}` }), el("span", { textContent: describeMessage(detail) }))),
      el("li", { className: "diag-actions" }, copy, el("small", { className: "muted", textContent: t("Discord などで相談する時に貼り付けてください") })));
  } catch (e) {
    box.replaceChildren(el("li", { className: "diag bad", textContent: describeMessage(String(e)) }));
  }
  $("diagnose").disabled = false;
}
$("diagnose").addEventListener("click", () => runDiagnosis($("diagnose-url").value.trim()));

// ───────── サイトごとに覚えた取り方 ─────────
async function renderSiteMethods() {
  const methods = await invoke("list_site_methods").catch(() => ({}));
  const label = (m) => [
    m.cookies && t("ログイン情報を使う"),
    m.impersonate && t("ブラウザ偽装"),
    m.proxy && t("地域制限用のプロキシ"),
    m.engine === "gallery-dl" && t("画像用の方法"),
    m.engine === "direct" && t("直接保存"),
  ].filter(Boolean).join(" + ");
  const entries = Object.entries(methods);
  $("site-methods").replaceChildren(...(entries.length ? entries.map(([site, m]) => el("li", {},
    el("img", { className: "favicon", src: faviconOf(`https://${site}`) }),
    el("span", { textContent: site }),
    el("small", { className: "muted", textContent: label(m) }),
    button("忘れる", async () => {
      await invoke("forget_site_method", { site });
      renderSiteMethods();
    }))) : [el("li", { className: "muted", textContent: t("まだありません") })]));
}

// ───────── 似た報告を探す(ダウンロード機能の GitHub) ─────────
function searchSimilarReports(message = "") {
  // 動画ごとに違う部分(ID・URL・数字)を外して、エラーの決まった言い回しだけで探す
  const core = message.replace(/^ERROR:\s*/i, "").replace(/\[[^\]]*\]\s*/g, "").replace(/https?:\/\/\S+/g, "")
    .replace(/\b[\w-]{11}\b(?=:)/g, "").replace(/\d+/g, " ").replace(/\s+/g, " ").trim().slice(0, 120);
  invoke("open_url", { url: `https://github.com/yt-dlp/yt-dlp/issues?q=${encodeURIComponent(core || message.slice(0, 120))}` });
}

// ───────── 右クリックメニュー ─────────
// [id, 表示名]。設定の「右クリックメニューに出す項目」で選べる
const JOB_MENU = [["open", "開く"], ["folder", "フォルダで表示"], ["page", "元のページを開く"], ["copy", "URL をコピー"],
  ["retry", "再試行"], ["now", "今すぐ始める"], ["pause", "一時停止 / 再開"], ["cancel", "中止"], ["log", "記録を見る"], ["diagnose", "接続を診断"]];
const LIB_MENU = [["open", "開く"], ["folder", "フォルダで表示"], ["page", "元のページを開く"], ["copy", "URL をコピー"],
  ["trim", "切り抜き"], ["audio", "音声だけ取り出す"], ["waveform", "波形動画を作る"], ["karaoke", "カラオケ版"], ["vocals", "歌声を強めた版"],
  ["again", "もう一度ダウンロード"],
  ["remove", "一覧から消す"], ["delete", "ファイルも削除"]];

function menuEnabled(kind, id) {
  const chosen = ui(`menu_${kind}`);
  return !Array.isArray(chosen) || chosen.includes(id);
}

function jobMenu(job) {
  const actions = {
    open: job.file_path && (() => invoke("open_file", { path: job.file_path })),
    folder: job.file_path && (() => invoke("show_in_folder", { path: job.file_path })),
    page: () => invoke("open_url", { url: job.url }),
    copy: () => navigator.clipboard.writeText(job.url).catch(() => {}),
    retry: ["failed", "canceled", "done"].includes(job.status) && (() => invoke("retry_job", { id: job.id })),
    now: job.status === "waiting" && (() => invoke("start_now", { id: job.id })),
    pause: ["running", "waiting"].includes(job.status) ? () => invoke("pause_job", { id: job.id })
      : job.status === "paused" && (() => invoke("resume_job", { id: job.id })),
    cancel: ["running", "waiting", "paused"].includes(job.status) && (() => invoke("cancel_job", { id: job.id })),
    log: () => { openTab("logs"); openLog(job.id); },
    diagnose: () => runDiagnosis(job.url),
  };
  return JOB_MENU.filter(([id]) => actions[id] && menuEnabled("job", id)).map(([id, label]) => [label, actions[id]]);
}

function libraryMenu(entry, item, remove, open) {
  const media = entry.exists && (entry.kind === "video" || entry.kind === "audio");
  const actions = {
    open: entry.exists && open,
    folder: entry.exists && (() => invoke("show_in_folder", { path: entry.file_path })),
    page: () => invoke("open_url", { url: entry.url }),
    copy: () => navigator.clipboard.writeText(entry.url).catch(() => {}),
    trim: media && (() => toggleTrimPanel(item, entry)),
    audio: entry.exists && entry.kind === "video" && (() => convertMedia(entry, "audio")),
    waveform: entry.exists && entry.kind === "audio" && (() => convertMedia(entry, "waveform")),
    karaoke: entry.exists && entry.kind === "audio" && (() => convertMedia(entry, "karaoke")),
    vocals: entry.exists && entry.kind === "audio" && (() => convertMedia(entry, "vocals")),
    again: () => invoke("add_downloads", { text: entry.url, overrides: { mode: entry.kind } }),
    remove: () => remove(false),
    delete: entry.exists && (() => remove(true)),
  };
  return LIB_MENU.filter(([id]) => actions[id] && menuEnabled("library", id)).map(([id, label]) => [label, actions[id]]);
}

/** その場に小さなメニューを出す。Esc・外を押すと閉じる。矢印キーでも選べる */
function openContextMenu(event, items) {
  event.preventDefault();
  document.querySelector(".context-menu")?.remove();
  if (!items.length) return;
  const menu = el("div", { className: "context-menu", role: "menu" },
    ...items.map(([label, action]) => el("button", { type: "button", role: "menuitem", textContent: t(label),
      onclick: () => { menu.remove(); action(); } })));
  document.body.append(menu);
  const x = Math.min(event.clientX || 20, innerWidth - menu.offsetWidth - 8);
  const y = Math.min(event.clientY || 20, innerHeight - menu.offsetHeight - 8);
  Object.assign(menu.style, { left: `${x}px`, top: `${y}px` });
  menu.querySelector("button").focus();
  menu.addEventListener("keydown", (e) => {
    const buttons = [...menu.querySelectorAll("button")];
    const i = buttons.indexOf(document.activeElement);
    if (e.key === "ArrowDown") buttons[(i + 1) % buttons.length].focus();
    if (e.key === "ArrowUp") buttons[(i - 1 + buttons.length) % buttons.length].focus();
    if (e.key === "Escape") menu.remove();
  });
  setTimeout(() => document.addEventListener("click", () => menu.remove(), { once: true }));
}

function renderMenuEditor() {
  const column = (kind, title, list) => el("div", {},
    el("h4", { textContent: t(title) }),
    ...list.map(([id, label]) => {
      const box = el("input", { type: "checkbox", checked: menuEnabled(kind, id) });
      box.addEventListener("change", async () => {
        const current = Array.isArray(ui(`menu_${kind}`)) ? ui(`menu_${kind}`) : list.map(([i]) => i);
        settings.ui = { ...(settings.ui || {}), [`menu_${kind}`]: box.checked ? [...current, id] : current.filter((x) => x !== id) };
        await persistSettings();
      });
      return el("label", { className: "check" }, box, t(label));
    }));
  $("menu-editor").replaceChildren(column("job", "ダウンロード", JOB_MENU), column("library", "ライブラリ", LIB_MENU));
}

// ───────── 使い方ガイド(初めて起動した時だけ) ─────────
const GUIDE_STEPS = [
  ["ようこそ Grabbit へ", "動画のページの URL を貼るだけで保存できます。コピーした URL は自動で入力欄に入ります。"],
  ["保存のしかたを選ぶ", "URL を調べた後の「保存のしかた」で、「自分の設定」か自分で作った保存プリセットを選べます。保存プリセットは「保存プリセット」タブで作れます。"],
  ["ログインが必要な動画", "会員限定などの動画は、設定の「ログインが必要な動画用」で Grabbit の中からログインできます。"],
  ["困った時は", "失敗したダウンロードの「診断」で原因を調べられます。設定の右上の検索で、設定項目を探せます。"],
];
function showGuide(step = 0) {
  document.querySelector(".guide-backdrop")?.remove();
  const [title, text] = GUIDE_STEPS[step];
  const last = step === GUIDE_STEPS.length - 1;
  const finish = async () => {
    backdrop.remove();
    settings.ui = { ...(settings.ui || {}), guide_done: true };
    await persistSettings();
  };
  const backdrop = el("div", { className: "guide-backdrop", role: "dialog", "aria-modal": "true", "aria-label": t(title) },
    el("div", { className: "guide" },
      el("div", { className: "dots" }, ...GUIDE_STEPS.map((_, i) => el("span", { className: i === step ? "on" : "" }))),
      el("h3", { textContent: t(title) }),
      el("p", { textContent: t(text) }),
      el("div", { className: "row" },
        button("閉じる", finish),
        el("span", { className: "spacer" }),
        step > 0 && button("戻る", () => showGuide(step - 1)),
        button(last ? "はじめる" : "次へ", last ? finish : () => showGuide(step + 1), "primary small"))));
  document.body.append(backdrop);
  backdrop.querySelector(".primary").focus();
}
$("guide-open")?.addEventListener("click", () => showGuide());

// ───────── 設定の「?」の説明 ─────────
const HELP = {
  cookies_browser: "会員限定・年齢制限の動画を保存する時に使うログインの方法です。Chrome / Edge は読めないことが多いので「Grabbit の中でログイン」がおすすめです。",
  cookies_when: "「必要な時だけ」なら、まずログインなしで試し、ログインが要る時だけ使います。普段はこちらで大丈夫です。",
  update_channel: "ダウンロード機能の更新の受け取り方です。サイトの仕様変更にすぐ追いつくので nightly がおすすめです。",
  max_parallel: "同時にダウンロードする数です。多くしすぎるとサイトに止められることがあります。2〜3 がおすすめです。",
  concurrent_fragments: "1本を細かく分けて同時に取る数です。回線が速いなら増やすと速くなります。",
  rate_limit: "1本あたりの速さの上限です(例: 5M = 毎秒 5MB)。空欄で制限しません。",
  retries: "途中で切れた時に、その場でやり直す回数です。",
  impersonate: "サイトに弾かれた時に、普通のブラウザのふりをしてやり直します。",
  proxy: "別の場所を通してつなぎます。会社や学校のネットワークで必要な時に使います。",
  restrict_filenames: "ファイル名を英数字だけにします。古いテレビ・車などで文字化けする時に使います。",
  use_archive: "一度保存した動画を記録して、二度と保存しないようにします(自動保存で使われます)。",
  embed_metadata: "タイトル・投稿者・チャプターなどを動画ファイルの中に書き込みます。",
  sponsorblock_remove: "YouTube の動画から、宣伝などの区間をみんなの登録をもとに自動で切り取ります。",
  auto_subtitles: "サイトが自動で作った字幕も使います。正確さは字幕によります。",
  date_after: "これより前に投稿された動画を飛ばします。",
  max_downloads: "1回の確認で保存する本数の上限です。チャンネルを丸ごと落とさないようにする時に使います。",
  temp_dir: "ダウンロード途中のファイルを置く場所です。保存先が遅いドライブの時に速い SSD を指定します。",
  queue_order: "待っているものを、どの順番で始めるかです。大きさは分かったものだけで並べます。",
  pad_aspect: "縦長の動画に黒帯を足して横長にする(またはその逆)ことで、テレビやスマホで全画面表示しやすくします。",
  music_tags: "タイトルから曲を探して、正しい曲名・アーティスト・アルバムを書き込みます(MusicBrainz を使います)。",
  embed_lyrics: "歌詞のデータベース(LRCLIB)で見つかった歌詞を書き込みます。時間付きの歌詞なら対応プレーヤーで流れます。",
  verify_files: "保存した後に、最後まで読めるかを確かめます。壊れていたら自動でやり直します。",
  remember_methods: "ログインやブラウザ偽装など、そのサイトでうまくいった方法を覚えて次から最初に使います。",
  on_same_name: "保存先に同じ名前のファイルがあった時に、どうするかです。",
  pin_ytdlp: "更新で調子が悪くなった時に、今の版のまま使い続けるためのものです。",
};
function addHelpIcons() {
  for (const [name, text] of Object.entries(HELP)) {
    const field = form.elements[name];
    const label = field?.closest("label");
    if (!label || label.querySelector(".help")) continue;
    const help = el("button", { type: "button", className: "help", textContent: "?", "aria-label": t("説明"), title: t(text) });
    help.addEventListener("click", (e) => {
      e.preventDefault();
      e.stopPropagation();
      document.querySelector(".help-pop")?.remove();
      const pop = el("div", { className: "help-pop", role: "tooltip", textContent: t(text) });
      document.body.append(pop);
      const r = help.getBoundingClientRect();
      Object.assign(pop.style, { left: `${Math.min(r.left, innerWidth - 340)}px`, top: `${r.bottom + 6}px` });
      setTimeout(() => document.addEventListener("click", () => pop.remove(), { once: true }));
    });
    // ラベルの最初の文字のすぐ後ろに置く
    const firstText = [...label.childNodes].find((n) => n.nodeType === Node.TEXT_NODE && n.textContent.trim());
    if (firstText) firstText.after(help);
    else label.prepend(help);
  }
}

// ───────── 配信の録り方 ─────────
let liveFromStart = false;
$("pv-live").addEventListener("click", (e) => {
  const button = e.target.closest("button");
  if (!button) return;
  liveFromStart = button.dataset.live === "start";
  $("pv-live").querySelectorAll("button").forEach((b) => b.classList.toggle("active", b === button));
});

// ───────── 後で保存 ─────────
// 今は落とさず、あとでまとめて保存する一覧(設定に保存するので、閉じても残る)
function laterList() {
  return Array.isArray(ui("later")) ? ui("later") : [];
}
async function setLater(list) {
  settings.ui = { ...(settings.ui || {}), later: list };
  await persistSettings();
  renderLater();
}
function renderLater() {
  const list = laterList();
  $("later-all").disabled = list.length === 0;
  $("later-clear").disabled = list.length === 0;
  $("later-box").querySelector("summary").textContent = t("後で保存 ({n})", { n: list.length });
  $("later-list").replaceChildren(...list.map((item, i) => el("li", {},
    el("img", { className: "favicon", src: item.thumbnail || faviconOf(item.url), alt: "" }),
    el("span", { className: "grow", textContent: item.title || item.url, title: item.url }),
    button("保存", async () => {
      await invoke("add_downloads", { text: item.url, overrides: item.overrides || presetValues });
      setLater(list.filter((_, j) => j !== i));
    }),
    iconButton("✕", "消す", () => setLater(list.filter((_, j) => j !== i))))));
}
$("pv-later").addEventListener("click", async () => {
  if (!previewInfo) return;
  const item = { url: previewUrl, title: previewInfo.title, thumbnail: previewInfo.thumbnail, overrides: collectOverrides() };
  await setLater([...laterList().filter((x) => x.url !== item.url), item]);
  showStatus(t("「後で保存」に入れました"));
});
$("later-all").addEventListener("click", async () => {
  for (const item of laterList()) await invoke("add_downloads", { text: item.url, overrides: item.overrides || presetValues });
  setLater([]);
});
$("later-clear").addEventListener("click", () => {
  const before = laterList();
  setLater([]);
  offerUndo(t("{n} 件を「後で保存」から消しました", { n: before.length }), () => setLater(before));
});

// ───────── 自分で選ぶ色 ─────────
// 色の入力欄は空にできない(必ず何かの色が入る)ので、ほかの設定と同じ読み方をすると
// 触っていないのに黒などが保存されてしまう。動かした時だけ保存する
document.querySelectorAll("[data-color]").forEach((input) => {
  input.addEventListener("input", () => {
    settings.ui = { ...(settings.ui || {}), [input.dataset.color]: input.value };
    applyAppearance();
  });
  input.addEventListener("change", () => persistSettings());
});
function fillColorInputs() {
  const styles = getComputedStyle(document.documentElement);
  const toHex = (value) => {
    const v = value.trim();
    if (/^#[0-9a-f]{6}$/i.test(v)) return v;
    const m = v.match(/\d+/g);
    return m ? `#${m.slice(0, 3).map((n) => Number(n).toString(16).padStart(2, "0")).join("")}` : "#000000";
  };
  for (const [key, variable] of [["custom_bg", "--bg"], ["custom_panel", "--panel"], ["custom_text", "--text"]]) {
    const input = document.querySelector(`[data-color="${key}"]`);
    if (input) input.value = ui(key) || toHex(styles.getPropertyValue(variable));
  }
}

// ───────── 色をテーマに戻す ─────────
document.querySelectorAll(".color-reset").forEach((b) => b.addEventListener("click", async () => {
  const ui = { ...(settings.ui || {}) };
  delete ui[b.dataset.for];
  settings.ui = ui;
  await persistSettings();
  applyAppearance();
  fillColorInputs();
}));

// ───────── 失敗を書き出す・個人情報を消してコピー ─────────
async function exportFailed() {
  try {
    const path = await invoke("export_failed");
    showStatus(t("書き出しました: {path}(あとで URL 欄にドロップすると、まとめてやり直せます)", { path }));
    invoke("show_in_folder", { path });
  } catch (e) {
    showStatus(describeMessage(String(e)), "error");
  }
}
$("log-copy-safe").addEventListener("click", async () => {
  const text = await invoke("scrub_text", { text: $("log-text").textContent });
  navigator.clipboard.writeText(text).catch(() => {});
  $("log-copy-safe").textContent = `✓ ${t("コピーしました")}`;
  setTimeout(() => ($("log-copy-safe").textContent = t("個人情報を消してコピー")), 1500);
});

// ───────── ログイン情報の合言葉 ─────────
async function renderLoginLock() {
  const locked = await invoke("login_locked").catch(() => false);
  $("login-lock").classList.toggle("hidden", !locked);
  $("login-panel").classList.toggle("hidden", locked);
}
$("login-unlock").addEventListener("click", async () => {
  const ok = await invoke("unlock_login", { password: $("login-unlock-pass").value });
  $("login-unlock-pass").value = "";
  $("login-unlock-status").textContent = ok ? "" : `✕ ${t("合言葉が違います")}`;
  if (ok) {
    await renderLoginLock();
    renderLogins();
  }
});
$("login-unlock-pass").addEventListener("keydown", (e) => { if (e.key === "Enter") $("login-unlock").click(); });
$("login-password").addEventListener("click", async () => {
  const has = Boolean(settings.login_password_hash);
  const current = has ? prompt(t("今の合言葉")) : "";
  if (current === null) return;
  const next = prompt(t("新しい合言葉(4 文字以上。空欄で合言葉を外す)"));
  if (next === null) return;
  try {
    await invoke("set_login_password", { current, new: next });
    settings = await invoke("get_settings");
    alert(next ? t("合言葉をかけました。次に起動した時から求めます") : t("合言葉を外しました"));
  } catch (e) {
    alert(describeMessage(String(e)));
  }
});

// ───────── チャンネルの再生リスト一覧 ─────────
$("pv-channel-playlists").addEventListener("click", () => {
  const base = youtubeChannelBase(previewInfo?.url || previewUrl);
  if (!base) return;
  $("url").value = `${base}/playlists`;
  updateGoButton();
  go();
});

// ───────── ダウンロード機能の変更点・前の版 ─────────
$("ytdlp-changelog").addEventListener("click", async () => {
  const box = $("changelog");
  box.classList.toggle("hidden");
  if (box.classList.contains("hidden")) return;
  box.textContent = t("読み込んでいます…");
  try {
    const releases = await invoke("ytdlp_changelog");
    box.replaceChildren(...releases.flatMap(([tag, date, body]) => [el("h4", { textContent: `${tag} (${date})` }), el("div", { textContent: body || "-" })]));
  } catch (e) {
    box.textContent = describeMessage(String(e));
  }
});
$("ytdlp-rollback").addEventListener("click", async () => {
  if (!confirm(t("ダウンロード機能を1つ前の版に戻しますか?(戻した後は自動で更新しなくなります)"))) return;
  try {
    const version = await invoke("rollback_ytdlp");
    settings = await invoke("get_settings");
    refreshAll();
    alert(t("前の版に戻しました: {version}", { version }));
  } catch (e) {
    alert(describeMessage(String(e)));
  }
});

// ───────── スマホ連携(ntfy) ─────────
function renderNtfyNames() {
  const topic = settings.ntfy_topic || "…";
  document.querySelectorAll(".ntfy-topic-name").forEach((e) => (e.textContent = topic));
  document.querySelectorAll(".ntfy-send-name").forEach((e) => (e.textContent = `${topic}-send`));
  document.querySelectorAll(".ntfy-send-url").forEach((e) => (e.textContent = `${(settings.ntfy_server || "https://ntfy.sh").replace(/\/$/, "")}/${topic}-send`));
}
$("ntfy-generate").addEventListener("click", async () => {
  const random = [...crypto.getRandomValues(new Uint8Array(9))].map((b) => b.toString(36).padStart(2, "0")).join("").slice(0, 14);
  settings.ntfy_topic = `grabbit-${random}`;
  form.elements.ntfy_topic.value = settings.ntfy_topic;
  await persistSettings();
  renderNtfyNames();
});
$("ntfy-test").addEventListener("click", async () => {
  $("ntfy-status").textContent = t("送っています…");
  try {
    await invoke("ntfy_test", { server: settings.ntfy_server, topic: settings.ntfy_topic });
    $("ntfy-status").textContent = `✓ ${t("送りました。スマホに届いたか確かめてください")}`;
  } catch (e) {
    $("ntfy-status").textContent = `✕ ${describeMessage(String(e))}`;
  }
});

// ───────── 不具合報告・動作の重さ ─────────
$("bug-report").addEventListener("click", async () => {
  try {
    const text = await invoke("bug_report");
    await navigator.clipboard.writeText(text);
    alert(t("不具合報告用の情報をコピーしました。Discord に貼り付けてください(パスワードや保存先は伏せてあります)"));
  } catch (e) {
    alert(describeMessage(String(e)));
  }
});
let perfTimer;
$("perf-open").addEventListener("click", () => {
  const box = $("perf");
  box.classList.toggle("hidden");
  clearInterval(perfTimer);
  if (box.classList.contains("hidden")) return;
  const refresh = async () => {
    if (box.classList.contains("hidden") || box.offsetParent === null) return clearInterval(perfTimer);
    const stats = await invoke("performance_stats").catch(() => null);
    if (!stats) return;
    const rows = [...stats.processes].sort((a, b) => b.cpu - a.cpu);
    box.replaceChildren(
      el("div", { className: "muted", textContent: t("ダウンロード中 {running} 件 · 待ち {waiting} 件 · CPU {cpus} コア(100% = 1 コア分)", stats) }),
      el("table", {},
        el("tr", {}, el("th", { textContent: t("動いているもの") }), el("th", { textContent: "CPU" }), el("th", { textContent: t("メモリ") })),
        ...rows.map((p) => el("tr", {}, el("td", { textContent: p.name }), el("td", { textContent: `${p.cpu}%` }), el("td", { textContent: `${p.memory_mb} MB` })))),
      el("small", { className: "muted", textContent: t("FFmpeg が重い時は変換・焼き付け・縦横合わせを減らす、yt-dlp が多い時は同時ダウンロード数を減らすと軽くなります") }));
  };
  refresh();
  perfTimer = setInterval(refresh, 3000);
});

// ───────── フォルダを選ぶ ─────────
async function pickFolder(start) {
  try {
    return await invoke("pick_folder", { start: start || settings.output_dir || null });
  } catch (e) {
    // 選ぶ画面を出せなかったときは、理由を見せて手で入力してもらう
    alert(`${t("フォルダを選ぶ画面を開けませんでした。直接入力してください")}\n${e}`);
    return null;
  }
}
// 設定のフォームは送信しない(Enter キーや種類の無いボタンで画面が読み込み直されるのを防ぐ)
form.addEventListener("submit", (e) => e.preventDefault());
$("pick-output").addEventListener("click", async () => {
  const folder = await pickFolder(settings.output_dir);
  if (!folder) return;
  form.elements.output_dir.value = folder;
  form.elements.output_dir.dispatchEvent(new Event("change", { bubbles: true }));
});

// ───────── 別のフォルダへ移す ─────────
$("lib-move").addEventListener("click", async () => {
  const targets = libraryShown.filter((e) => e.exists);
  if (!targets.length) return;
  const dest = await pickFolder();
  if (!dest || !confirm(t("今表示している {n} 件を「{dest}」へ移しますか?", { n: targets.length, dest }))) return;
  try {
    const moved = await invoke("move_library", { ids: targets.map((e) => e.id), dest });
    $("verify-status").textContent = `✓ ${t("{n} 件を移しました", { n: moved })}`;
  } catch (e) {
    $("verify-status").textContent = `✕ ${describeMessage(String(e))}`;
  }
  loadLibrary();
});
listen("move_progress", ({ payload: [done, total, title] }) => {
  $("verify-status").textContent = t("移しています {done}/{total}: {title}", { done, total, title });
});


// ───────── 外への通信 ─────────
$("net-open").addEventListener("click", async () => {
  const box = $("net-list");
  box.classList.toggle("hidden");
  if (box.classList.contains("hidden")) return;
  const rows = await invoke("list_connections").catch(() => []);
  box.replaceChildren(
    el("div", { className: "muted", textContent: t("このアプリを起動してからつないだ先です(新しい順・最大 500 件)") }),
    el("table", {},
      el("tr", {}, el("th", { textContent: t("時刻") }), el("th", { textContent: t("つないだ先") }), el("th", { textContent: t("目的") })),
      ...rows.slice(0, 200).map((r) => el("tr", {},
        el("td", { textContent: new Date(r.time * 1000).toLocaleTimeString(document.documentElement.lang) }),
        el("td", { textContent: r.host }),
        el("td", { textContent: t(r.purpose) })))));
});

// ───────── エラーの統計 ─────────
const FAILURE_KINDS = { login: "ログインが必要", geo: "地域制限", network: "回線のエラー", blocked: "サイトに止められた", unavailable: "見られない動画", other: "その他" };
$("log-stats").addEventListener("click", async () => {
  const box = $("error-stats");
  box.classList.toggle("hidden");
  if (box.classList.contains("hidden")) return;
  const stats = await invoke("error_stats").catch(() => null);
  if (!stats) return;
  const peak = Math.max(1, ...stats.hours);
  box.replaceChildren(
    el("div", {},
      el("b", { textContent: t("失敗が多いサイト(全 {n} 件)", { n: stats.total }) }),
      el("table", {}, ...stats.sites.map(([site, n]) => el("tr", {}, el("td", { textContent: site }), el("td", { textContent: String(n) }))))),
    el("div", {},
      el("b", { textContent: t("時間帯(0〜23 時)") }),
      el("div", { className: "hours", "aria-label": t("時間帯ごとの失敗の数") },
        ...stats.hours.map((n, h) => el("div", { style: `height:${(n / peak) * 100}%`, title: `${h}:00 — ${n}` }))),
      el("b", { textContent: t("種類") }),
      el("table", {}, ...Object.entries(stats.kinds).map(([k, n]) => el("tr", {}, el("td", { textContent: t(FAILURE_KINDS[k] || k) }), el("td", { textContent: String(n) })))),
      button("統計を消す", async () => { await invoke("clear_error_stats"); box.classList.add("hidden"); })),
  );
});

// ───────── URL 欄の候補(サイト名・最近の URL) ─────────
let recentUrls = (() => { try { return JSON.parse(localStorage.getItem("grabbit-recent") || "[]"); } catch { return []; } })();
function rememberUrl(url) {
  recentUrls = [url, ...recentUrls.filter((u) => u !== url)].slice(0, 30);
  try { localStorage.setItem("grabbit-recent", JSON.stringify(recentUrls)); } catch { /* 覚えられなくても困らない */ }
}
let suggestIndex = -1;
function renderUrlSuggest() {
  const box = $("url-suggest");
  const text = $("url").value.trim().toLowerCase();
  if (!text || text.includes("\n") || /^https?:\/\//.test(text) && recentUrls.every((u) => !u.startsWith(text))) return box.classList.add("hidden");
  const fromRecent = recentUrls.filter((u) => u.toLowerCase().includes(text)).slice(0, 5).map((u) => [u, u]);
  const fromSites = POPULAR_SITES.filter((d) => d.includes(text)).slice(0, 5).map((d) => [`https://${d}/`, d]);
  const items = [...fromRecent, ...fromSites].slice(0, 8);
  suggestIndex = -1;
  box.classList.toggle("hidden", items.length === 0);
  box.replaceChildren(...items.map(([url, label]) => el("li", { role: "option", onclick: () => pickSuggestion(url) },
    el("img", { className: "favicon", src: faviconOf(url), alt: "" }), el("span", { textContent: label }))));
  [...box.children].forEach((li, i) => (li.dataset.url = items[i][0]));
}
function pickSuggestion(url) {
  $("url").value = url;
  $("url-suggest").classList.add("hidden");
  updateGoButton();
  $("url").focus();
}
$("url").addEventListener("input", renderUrlSuggest);
$("url").addEventListener("keydown", (e) => {
  const box = $("url-suggest");
  if (box.classList.contains("hidden")) return;
  const items = [...box.children];
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    suggestIndex = (suggestIndex + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
    items.forEach((li, i) => li.setAttribute("aria-selected", String(i === suggestIndex)));
  } else if (e.key === "Enter" && suggestIndex >= 0) {
    e.preventDefault();
    e.stopImmediatePropagation();
    pickSuggestion(items[suggestIndex].dataset.url);
  } else if (e.key === "Escape") {
    box.classList.add("hidden");
  }
}, true);
document.addEventListener("click", (e) => { if (!e.target.closest(".url-card")) $("url-suggest").classList.add("hidden"); });

// ───────── ヘルプ(よくある質問) ─────────
const FAQ = [
  ["保存できない・失敗する", "失敗したダウンロードの「診断」を押すと、回線・ログイン・部品のどこが原因か調べられます。「似た報告を探す」で、同じ問題の解決方法が見つかることもあります。"],
  ["会員限定・年齢制限の動画を保存したい", "設定 →「ログインが必要な動画用」で「Grabbit の中でログイン」を選び、サイトのボタンからログインしてください。"],
  ["ダウンロードが遅い", "設定 →「速度・通信」で「速さ優先」をオンにするか、同時ダウンロード数を減らしてください。「接続を診断」で回線の速さも測れます。"],
  ["字幕も保存したい・焼き付けたい", "設定 →「字幕・追加ファイル」で決められます。翻訳した字幕(YouTube)も選べます。よく使うなら保存プリセットにしておくと便利です。"],
  ["音楽だけ保存したい", "「保存するもの」で「音声だけ」を選ぶと音声で保存します。形式(MP3 など)や、曲名・歌詞・テンポを入れる設定もあります。"],
  ["チャンネルの新着を自動で保存したい", "設定 →「自動保存」に URL を登録します。「知らせるだけ」にすると、保存は自分で押せます。"],
  ["保存先・ファイル名を変えたい", "設定 →「保存先・ファイル名」で、フォルダ分けや名前の形を例を見ながら決められます。"],
  ["同じ動画を二重に保存してしまった", "ライブラリの種類で「重複」を選ぶと、まとめて整理できます。"],
  ["スマホで完了を知りたい・スマホから送りたい", "設定 →「スマホ連携」で ntfy のトピックを作り、スマホの ntfy アプリで購読します。"],
  ["更新したら調子が悪くなった", "設定の「ダウンロード機能」の「前の版に戻す」で、1つ前の版に戻せます。"],
  ["設定を別の PC に移したい", "設定 →「設定の引っ越し」で書き出して、別の PC で読み込みます。"],
  ["何がどこに通信しているか知りたい", "設定 →「記録・プライバシー」の「外への通信を見る」で、つないだ先と目的を確かめられます。"],
];
function renderFaq() {
  const query = $("faq-search").value.trim().toLowerCase();
  const words = query.split(/\s+/).filter(Boolean);
  const hits = FAQ.filter(([q, a]) => !words.length || words.some((w) => `${t(q)} ${t(a)} ${q} ${a}`.toLowerCase().includes(w)));
  $("faq").replaceChildren(...(hits.length ? hits.map(([q, a]) => el("details", {}, el("summary", { textContent: t(q) }), el("p", { textContent: t(a) })))
    : [el("p", { className: "muted", textContent: t("見つかりませんでした。Discord で質問できます") })]));
}
$("faq-search").addEventListener("input", renderFaq);

// ───────── この版の変更点(更新した後に1回だけ出す) ─────────
async function showReleaseNotes() {
  let text;
  try {
    text = await invoke("app_release_notes");
  } catch {
    text = t("変更点を読み込めませんでした。GitHub のリリースで確かめられます");
  }
  const version = await invoke("app_version").catch(() => "");
  const backdrop = el("div", { className: "guide-backdrop", role: "dialog", "aria-modal": "true" },
    el("div", { className: "guide" },
      el("h3", { textContent: t("Grabbit {version} の変更点", { version }) }),
      el("div", { className: "notes", textContent: text }),
      el("div", { className: "row" }, el("span", { className: "spacer" }), button("閉じる", () => backdrop.remove(), "primary small"))));
  document.body.append(backdrop);
}
$("notes-open").addEventListener("click", showReleaseNotes);
async function checkVersionChange() {
  const version = await invoke("app_version").catch(() => "");
  const last = ui("last_version");
  if (!version || last === version) return;
  settings.ui = { ...(settings.ui || {}), last_version: version };
  await persistSettings();
  if (last) showReleaseNotes(); // 初めて入れた時は出さない
}

listen("tools_tampered", ({ payload }) => {
  showStatus(t("部品が書き換えられていたため、入れ直しました: {list}", { list: payload.join(", ") }), "error");
});

// ───────── 起動時のお知らせ ─────────
async function showStartupNotices() {
  const notices = await invoke("startup_notices").catch(() => ({}));
  if (notices.settings_restored) showStatus(t("設定ファイルが壊れていたため、前回の控えから戻しました"), "error");
  if (notices.login_expired) showStatus(t("しばらく使っていなかったため、ログイン情報を消しました"));
}

// ───────── 設定の引っ越し ─────────
$("export-settings").addEventListener("click", async () => {
  try {
    const path = await invoke("export_settings");
    $("move-status").textContent = `✓ ${t("書き出しました: {path}", { path })}`;
    invoke("show_in_folder", { path });
  } catch (e) {
    $("move-status").textContent = `✕ ${describeMessage(String(e))}`;
  }
});
$("import-settings").addEventListener("click", () => $("import-file").click());
$("import-file").addEventListener("change", async () => {
  const file = $("import-file").files[0];
  $("import-file").value = "";
  if (!file || !confirm(t("今の設定を、このファイルの内容で置き換えますか?"))) return;
  try {
    settings = await invoke("import_settings", { text: await file.text() });
    lastSubs = await invoke("list_subscriptions");
    refreshAll();
    $("move-status").textContent = `✓ ${t("読み込みました。自動保存は、この PC での準備が終わると動き始めます")}`;
  } catch (e) {
    $("move-status").textContent = `✕ ${describeMessage(String(e))}`;
  }
});

$("reset").addEventListener("click", async () => {
  if (!confirm(t("設定を初期状態に戻しますか?"))) return;
  settings = await invoke("reset_settings");
  refreshAll();
});

async function showVersion() {
  const version = await invoke("ytdlp_version");
  $("ytdlp-version").textContent = version ? t("現在: {v}", { v: version }) : t("未導入");
  $("ytdlp-version-short").textContent = version ? t("エンジン {v}", { v: version }) : "";
}

async function updateYtdlp() {
  $("update-now").disabled = true;
  $("ytdlp-version").textContent = t("更新しています…");
  await invoke("update_ytdlp_now");
  await showVersion();
  $("update-now").disabled = false;
}
$("update-now").addEventListener("click", updateYtdlp);

// ───────── 準備状況 ─────────
// ───────── ライブラリ(保存したもの) ─────────
let library = [];
let libraryShown = []; // 今の絞り込みで見えているもの(名前の一括変更の対象)

async function loadLibrary() {
  library = await invoke("list_history");
  renderLibrary();
}
listen("history_changed", () => {
  if (!$("tab-library").classList.contains("hidden")) loadLibrary();
});

function formatSize(bytes) {
  if (!bytes) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** i).toFixed(i ? 1 : 0)} ${units[i]}`;
}

const LIBRARY_SORTS = {
  new: (a, b) => b.saved_at - a.saved_at,
  old: (a, b) => a.saved_at - b.saved_at,
  name: (a, b) => a.title.localeCompare(b.title),
  size: (a, b) => b.size - a.size,
};

/** ファイルの場所のドライブ(C: など。Mac・Linux は最初のフォルダ) */
function driveOf(path) {
  const windows = path.match(/^([a-zA-Z]:)/);
  return windows ? windows[1].toUpperCase() : `/${path.split("/").filter(Boolean)[0] || ""}`;
}
function folderOf(path) {
  return path.replace(/[\\/][^\\/]*$/, "");
}

/** ドライブ・フォルダの選択肢を、保存したものの場所から作る */
function fillLibraryLocations() {
  const fill = (select, values, allLabel) => {
    const current = select.value;
    select.replaceChildren(new Option(allLabel, ""), ...values.map((v) => new Option(v, v)));
    select.value = values.includes(current) ? current : "";
  };
  const drives = [...new Set(library.map((e) => driveOf(e.file_path)))].sort();
  fill($("lib-drive"), drives, t("すべてのドライブ"));
  const drive = $("lib-drive").value;
  const folders = [...new Set(library.filter((e) => !drive || driveOf(e.file_path) === drive).map((e) => folderOf(e.file_path)))].sort();
  fill($("lib-folder"), folders, t("すべてのフォルダ"));
  const uploaders = [...new Set(library.map((e) => e.uploader).filter(Boolean))].sort((a, b) => a.localeCompare(b));
  fill($("lib-uploader"), uploaders, t("すべての投稿者"));
}

function renderLibrary() {
  const query = $("lib-search").value.trim().toLowerCase();
  const kind = $("lib-kind").value;
  const groups = duplicateGroups();
  fillLibraryLocations();
  const drive = $("lib-drive").value;
  const folder = $("lib-folder").value;
  const uploader = $("lib-uploader").value;
  // 「100-200」の形の範囲(後ろが空なら上限なし)。値が分からない(0)ものは範囲指定の時は出さない
  const inRange = (value, range) => {
    if (!range) return true;
    const [min, max] = range.split("-").map((n) => (n === "" ? Infinity : Number(n)));
    return value > 0 && value >= min && value <= (Number.isFinite(max) ? max : Infinity);
  };
  const [lengthRange, qualityRange, sizeRange] = ["lib-length", "lib-quality", "lib-size"].map((id) => $(id).value);
  const shown = library
    .filter((e) => (kind === "trash") === Boolean(e.trashed)) // ごみ箱のものは「ごみ箱」を選んだ時だけ
    .filter((e) => !uploader || e.uploader === uploader)
    .filter((e) => inRange(e.duration, lengthRange) && inRange(e.height, qualityRange) && inRange(e.size, sizeRange))
    .filter((e) => !drive || driveOf(e.file_path) === drive)
    .filter((e) => !folder || folderOf(e.file_path) === folder)
    .filter((e) => kind !== "duplicates" || groups.has(e.id))
    .filter((e) => !kind || kind === "duplicates" || kind === "trash" || e.kind === kind || (kind === "file" && !["video", "audio", "thumbnail"].includes(e.kind)))
    .filter((e) => !query || e.title.toLowerCase().includes(query) || e.file_path.toLowerCase().includes(query))
    .sort(kind === "duplicates"
      ? (a, b) => groups.get(a.id) - groups.get(b.id) || b.saved_at - a.saved_at // 重複は組ごとに並べる
      : LIBRARY_SORTS[$("lib-sort").value]);
  $("lib-dedupe").classList.toggle("hidden", kind !== "duplicates" || shown.length === 0);
  libraryShown = shown;
  const total = library.reduce((sum, e) => sum + (e.exists ? e.size : 0), 0);
  $("lib-summary").textContent = library.length ? t("{n} 件 · 合計 {size}", { n: shown.length, size: formatSize(total) }) : "";
  $("library-empty").classList.toggle("hidden", library.length > 0);

  // 数が多いので先頭 500 件まで(検索で絞り込める)
  $("library").replaceChildren(...shown.slice(0, 500).map((entry) => {
    const open = () => entry.exists && invoke("open_file", { path: entry.file_path });
    const remove = async (deleteFile) => {
      const question = deleteFile
        ? t("「{name}」のファイルをごみ箱へ移しますか?", { name: entry.title })
        : t("「{name}」を一覧から消しますか?(ファイルは残ります)", { name: entry.title });
      // 一覧から消すだけなら、確かめずに消して「元に戻す」を出す
      if (deleteFile && !confirm(question)) return;
      try {
        await invoke("remove_history", { ids: [entry.id], deleteFile });
      } catch (e) {
        alert(String(e));
      }
      loadLibrary();
      if (!deleteFile) {
        offerUndo(t("「{name}」を一覧から消しました", { name: entry.title }), async () => {
          await invoke("restore_history", { entries: [entry] });
          loadLibrary();
        });
      }
    };
    const canTrim = entry.exists && (entry.kind === "video" || entry.kind === "audio");
    const item = el("li", { className: `lib-item ${entry.exists ? "" : "missing"} ${entry.drive_missing ? "drive-off" : ""}`, tabIndex: 0,
      oncontextmenu: (e) => openContextMenu(e, libraryMenu(entry, item, remove, open)) },
      el("img", { className: "lib-thumb", src: entry.thumbnail || faviconOf(entry.url), loading: "lazy", onclick: open }),
      el("div", { className: "lib-body" },
        el("div", { className: "lib-title", textContent: entry.title, title: entry.file_path }),
        el("div", { className: "lib-meta", textContent: [
          t(MODES[entry.kind] || "ファイル・画像"),
          entry.uploader,
          entry.height ? `${entry.height}p` : "",
          entry.duration ? formatDuration(entry.duration) : "",
          formatSize(entry.size),
          new Date(entry.saved_at * 1000).toLocaleDateString(document.documentElement.lang),
          entry.trashed ? t("ごみ箱にあります") : entry.drive_missing ? t("ドライブがつながっていません") : !entry.exists && t("ファイルが見つかりません"),
          goneSources.has(entry.id) && `⚠ ${t("元のページが見られなくなっています")}`,
          brokenFiles.has(entry.id) && `⚠ ${t("壊れています")}: ${brokenFiles.get(entry.id)}`,
        ].filter(Boolean).join(" · ") }),
      ),
      el("div", { className: "lib-actions" },
        entry.exists && button("開く", open),
        entry.exists && button("フォルダ", () => invoke("show_in_folder", { path: entry.file_path })),
        button("元のページ", () => invoke("open_url", { url: entry.url })),
        button("一覧から消す", () => remove(false)),
        canTrim && button("切り抜き", () => toggleTrimPanel(item, entry)),
        entry.exists && entry.kind === "video" && button("音声だけ取り出す", () => convertMedia(entry, "audio")),
        entry.exists && entry.kind === "audio" && button("波形動画を作る", () => convertMedia(entry, "waveform")),
        entry.exists && entry.kind === "audio" && button("カラオケ版", () => convertMedia(entry, "karaoke")),
        entry.exists && entry.kind === "audio" && button("歌声を強めた版", () => convertMedia(entry, "vocals")),
        entry.trashed && button("ごみ箱から戻す", async () => {
          try { await invoke("restore_from_trash", { id: entry.id }); } catch (e) { alert(describeMessage(String(e))); }
          loadLibrary();
        }, "primary small"),
        kind === "duplicates" && button("これだけ残す", () => keepOnly(entry, groups), "primary small"),
        entry.exists && button("ファイルも削除", () => remove(true)),
      ),
    );
    return item;
  }));
}

/** 切り抜きの入力欄を、その項目の下に開く(もう一度押すと閉じる) */
function toggleTrimPanel(item, entry) {
  const open = item.querySelector(".trim-panel");
  if (open) return open.remove();
  const start = el("input", { placeholder: "0:30", className: "trim-time" });
  const end = el("input", { placeholder: "1:45", className: "trim-time" });
  const exact = el("input", { type: "checkbox" });
  const status = el("span", { className: "muted" });
  const run = button("切り抜く", async () => {
    run.disabled = true;
    status.textContent = t("切り抜いています…");
    try {
      await invoke("trim_media", { id: entry.id, start: start.value, end: end.value, exact: exact.checked });
      status.textContent = `✓ ${t("できました。ライブラリに追加しました")}`;
    } catch (e) {
      status.textContent = `✕ ${describeMessage(String(e))}`;
    }
    run.disabled = false;
  }, "primary small");
  item.append(el("div", { className: "trim-panel" },
    el("label", {}, t("始まり"), start),
    el("label", {}, t("終わり"), end),
    el("label", { className: "check", title: t("オフ: 一瞬で終わり画質もそのまま(数秒ずれることがあります)") },
      exact, t("時間を正確に(少し時間がかかります)")),
    run, status));
  start.focus();
}

$("lib-search").addEventListener("input", renderLibrary);
$("lib-uploader").addEventListener("change", renderLibrary);
["lib-length", "lib-quality", "lib-size"].forEach((id) => $(id).addEventListener("change", renderLibrary));
$("lib-kind").addEventListener("change", renderLibrary);
$("lib-sort").addEventListener("change", renderLibrary);
$("lib-drive").addEventListener("change", () => {
  $("lib-folder").value = "";
  renderLibrary();
});
$("lib-folder").addEventListener("change", renderLibrary);
// ───────── ファイル名の一括変更 ─────────
const RENAME_TOKENS = [["{title}", "タイトル"], ["{date}", "保存日"], ["{site}", "サイト"], ["{n}", "連番"], ["{kind}", "種類"], ["{name}", "今の名前"]];
let renameTimer;

$("lib-rename").addEventListener("click", () => {
  const panel = $("rename-panel");
  panel.classList.toggle("hidden");
  if (panel.classList.contains("hidden")) return;
  $("rename-tokens").replaceChildren(...RENAME_TOKENS.map(([token, label]) => el("button", {
    type: "button", className: "fn-chip", textContent: `${t(label)} ${token}`,
    onclick: () => {
      const input = $("rename-pattern");
      const at = input.selectionStart ?? input.value.length;
      // 前の文字とくっつかないよう、必要なら空白をはさむ
      const before = input.value.slice(0, at);
      const gap = before && !/[\s\-_\[(]$/.test(before) ? " " : "";
      input.value = before + gap + token + input.value.slice(at);
      previewRename();
    },
  })));
  if (!$("rename-pattern").value) $("rename-pattern").value = "{title}";
  previewRename();
});
$("rename-pattern").addEventListener("input", () => {
  clearTimeout(renameTimer);
  renameTimer = setTimeout(previewRename, 250);
});

function renameTargets() {
  return libraryShown.filter((e) => e.exists).slice(0, 500);
}

/** 変える前に、新しい名前を先頭の数件だけ見せる */
async function previewRename() {
  const targets = renameTargets();
  $("rename-count").textContent = t("対象: 今表示している {n} 件", { n: targets.length });
  try {
    const rows = await invoke("rename_library", { ids: targets.slice(0, 5).map((e) => e.id), pattern: $("rename-pattern").value, dryRun: true });
    $("rename-preview").replaceChildren(...rows.map(([, old, next]) => el("li", {},
      el("span", { className: "muted", textContent: old }), " → ", el("b", { textContent: next }))));
  } catch (e) {
    $("rename-preview").replaceChildren(el("li", { className: "bad", textContent: describeMessage(String(e)) }));
  }
}

$("rename-run").addEventListener("click", async () => {
  const targets = renameTargets();
  if (!targets.length || !confirm(t("{n} 件のファイル名を変えますか?", { n: targets.length }))) return;
  try {
    const rows = await invoke("rename_library", { ids: targets.map((e) => e.id), pattern: $("rename-pattern").value, dryRun: false });
    $("rename-count").textContent = `✓ ${t("{n} 件の名前を変えました", { n: rows.length })}`;
  } catch (e) {
    alert(describeMessage(String(e)));
  }
  loadLibrary();
});

// ───────── ライブラリの検査 ─────────
let brokenFiles = new Map(); // id → 壊れていた理由(検査した後だけ)
let verifying = false;

$("lib-verify").addEventListener("click", async () => {
  if (verifying) return;
  const targets = libraryShown.filter((e) => e.exists && (e.kind === "video" || e.kind === "audio"));
  if (!targets.length) return showStatus(t("検査できる動画・音声がありません"), "error");
  verifying = true;
  $("lib-verify").disabled = true;
  try {
    const broken = await invoke("verify_library", { ids: targets.map((e) => e.id) });
    brokenFiles = new Map(broken);
    $("verify-status").textContent = broken.length
      ? `⚠ ${t("{n} 件中 {bad} 件が壊れていました", { n: targets.length, bad: broken.length })}`
      : `✓ ${t("{n} 件すべて問題ありませんでした", { n: targets.length })}`;
    $("lib-refetch").classList.toggle("hidden", broken.length === 0);
    renderLibrary();
  } catch (e) {
    $("verify-status").textContent = `✕ ${describeMessage(String(e))}`;
  }
  verifying = false;
  $("lib-verify").disabled = false;
});
listen("verify_progress", ({ payload: [done, total, title] }) => {
  $("verify-status").textContent = t("検査中 {done}/{total}: {title}", { done, total, title });
});

/** 壊れていたものをごみ箱へ移して、同じ URL から取り直す */
$("lib-refetch").addEventListener("click", async () => {
  const broken = library.filter((e) => brokenFiles.has(e.id));
  if (!broken.length || !confirm(t("壊れていた {n} 件をごみ箱へ移して、取り直しますか?", { n: broken.length }))) return;
  await invoke("remove_history", { ids: broken.map((e) => e.id), deleteFile: true }).catch(() => {});
  for (const e of broken) {
    await invoke("add_downloads", { text: e.url, overrides: { mode: e.kind } }).catch(() => {});
  }
  brokenFiles.clear();
  $("lib-refetch").classList.add("hidden");
  $("verify-status").textContent = `✓ ${t("{n} 件を取り直しています", { n: broken.length })}`;
  loadLibrary();
});

// ───────── 音声の取り出し・波形動画 ─────────
async function convertMedia(entry, kind) {
  showStatus(kind === "audio" ? t("音声を取り出しています…") : t("波形動画を作っています…"));
  openTab("library");
  try {
    await invoke("convert_media", { id: entry.id, kind });
    $("verify-status").textContent = `✓ ${t("できました。ライブラリに追加しました")}`;
  } catch (e) {
    $("verify-status").textContent = `✕ ${describeMessage(String(e))}`;
  }
  showStatus("");
}

// ───────── 元のページの確認 ─────────
let goneSources = new Map();
$("lib-sources").addEventListener("click", async () => {
  const targets = libraryShown.filter((e) => e.url?.startsWith("http"));
  if (!targets.length) return;
  $("lib-sources").disabled = true;
  try {
    goneSources = new Map(await invoke("check_sources", { ids: targets.map((e) => e.id) }));
    $("verify-status").textContent = goneSources.size
      ? `⚠ ${t("{n} 件中 {bad} 件は、元のページが見られなくなっています(保存しておいてよかったもの)", { n: targets.length, bad: goneSources.size })}`
      : `✓ ${t("{n} 件すべて、元のページは今も見られます", { n: targets.length })}`;
    renderLibrary();
  } catch (e) {
    $("verify-status").textContent = `✕ ${describeMessage(String(e))}`;
  }
  $("lib-sources").disabled = false;
});
listen("source_progress", ({ payload: [done, total, title] }) => {
  $("verify-status").textContent = t("確認中 {done}/{total}: {title}", { done, total, title });
});

// ───────── 重複の整理 ─────────
/**
 * 同じものを二重に保存していないか調べ、組ごとに番号を付ける(id → 組の番号)。
 * 「同じ動画(URL)で同じ種類」か「同じ種類で大きさがぴったり同じ」なら同じ物とみなす。
 * ファイルが残っているものだけを見る。
 */
function duplicateGroups() {
  const files = library.filter((e) => e.exists);
  const parent = new Map(files.map((e) => [e.id, e.id]));
  const root = (id) => (parent.get(id) === id ? id : root(parent.get(id)));
  const join = (a, b) => parent.set(root(a), root(b));
  const firstByKey = new Map();
  for (const e of files) {
    const keys = [`url|${e.kind}|${videoKey(e.url)}`];
    if (e.size > 1024 * 1024) keys.push(`size|${e.kind}|${e.size}`); // 小さなファイルは偶然そろうので見ない
    for (const key of keys) {
      if (firstByKey.has(key)) join(e.id, firstByKey.get(key));
      else firstByKey.set(key, e.id);
    }
  }
  const members = new Map();
  for (const e of files) members.set(root(e.id), [...(members.get(root(e.id)) || []), e.id]);
  const groups = new Map();
  let n = 0;
  for (const ids of members.values()) {
    if (ids.length < 2) continue;
    n += 1;
    for (const id of ids) groups.set(id, n);
  }
  return groups;
}

/** 組の中でこれだけ残し、ほかのファイルを削除する */
async function keepOnly(entry, groups) {
  const others = library.filter((e) => e.id !== entry.id && groups.get(e.id) === groups.get(entry.id));
  if (!others.length) return;
  const list = others.map((e) => `・${e.file_path}`).join("\n");
  if (!confirm(t("次の {n} 個のファイルをごみ箱へ移します", { n: others.length }) + `\n${list}`)) return;
  await invoke("remove_history", { ids: others.map((e) => e.id), deleteFile: true }).catch((e) => alert(String(e)));
  loadLibrary();
}

/** すべての組で、いちばん新しく保存したものだけを残す */
$("lib-dedupe").addEventListener("click", async () => {
  const groups = duplicateGroups();
  const newest = new Map();
  for (const e of library.filter((e) => groups.has(e.id))) {
    const g = groups.get(e.id);
    if (!newest.has(g) || e.saved_at > newest.get(g).saved_at) newest.set(g, e);
  }
  const remove = library.filter((e) => groups.has(e.id) && newest.get(groups.get(e.id)) !== e);
  if (!remove.length) return;
  if (!confirm(t("{groups} 組の重複から、新しく保存したものだけを残して {n} 個のファイルをごみ箱へ移します", { groups: newest.size, n: remove.length }))) return;
  await invoke("remove_history", { ids: remove.map((e) => e.id), deleteFile: true }).catch((e) => alert(String(e)));
  loadLibrary();
});

$("lib-clean").addEventListener("click", async () => {
  // 外付けドライブを抜いているだけのものは消さない(つなげばまた見られる)
  const missing = library.filter((e) => !e.exists && !e.drive_missing).map((e) => e.id);
  if (missing.length === 0) return;
  await invoke("remove_history", { ids: missing, deleteFile: false });
  loadLibrary();
});

// ───────── コピーした URL ─────────
// コピーした URL は、聞かずにそのまま入力欄へ入れる(1件なら設定に合わせてすぐ調べる)
/** 外から来た URL を入力欄に入れる(1件なら調べるところまで。保存は人が押す) */
function receiveUrls(text) {
  openTab("jobs");
  $("url").value = text;
  updateGoButton();
  if (ui("auto_analyze") && urlsInInput().length === 1) go();
}
listen("clipboard_urls", ({ payload }) => receiveUrls(payload));
// スマホ(ntfy)から送られた URL
listen("phone_urls", ({ payload }) => receiveUrls(payload));

// ───────── トレイ・通知の文言(今の言語で渡す) ─────────
function sendDesktopTexts() {
  invoke("set_desktop_texts", { texts: {
    open: t("Grabbit を開く"),
    folder: t("保存先を開く"),
    quit: t("終了"),
    done: t("保存しました"),
    failed: t("ダウンロードできませんでした"),
    new_items: t("新着があります"),
  } }).catch(() => {});
}
listen("tray_open_folder", () => invoke("open_folder", { path: settings.output_dir }));

// ───────── アプリ本体の更新 ─────────
function showAppUpdate(info) {
  $("app-update-text").textContent = t("新しい版 {v} があります", { v: info.version });
  $("app-update").classList.remove("hidden");
}
listen("app_update", ({ payload }) => showAppUpdate(payload));
// 本体の更新中は、部品のダウンロードと同じ形(何%・バー)で上に出す
listen("app_update_start", ({ payload }) => {
  $("app-update").classList.add("hidden");
  setupLabel = t("アプリの更新(v{v})", { v: payload });
  renderSetup(0, 0, "app");
});
listen("app_update_progress", ({ payload: [done, total] }) => renderSetup(done, total, "app"));
listen("app_update_installing", () => {
  $("setup").replaceChildren(el("div", { textContent: `${setupLabel} — ${t("インストールしています。終わると自動で再起動します…")}` }));
});
$("app-update-later").addEventListener("click", () => $("app-update").classList.add("hidden"));
$("app-update-install").addEventListener("click", async () => {
  $("app-update-install").disabled = true;
  try {
    await invoke("install_app_update");
  } catch (e) {
    $("app-update-text").textContent = `${t("更新できませんでした")}: ${e}`;
    $("app-update-install").disabled = false;
  }
});
// 設定の一番上のボタンも同じ確認をする
$("settings-update-check").addEventListener("click", () => {
  document.getElementById("sec-app").scrollIntoView({ behavior: "smooth" });
  $("app-update-check").click();
});
$("app-update-check").addEventListener("click", async () => {
  $("app-version-status").textContent = t("確認しています…");
  try {
    const info = await invoke("check_app_update");
    if (info) {
      showAppUpdate(info);
      $("app-version-status").textContent = t("新しい版 {v} があります", { v: info.version });
    } else {
      $("app-version-status").textContent = t("最新の版です");
    }
  } catch (e) {
    $("app-version-status").textContent = updateErrorText(e);
  }
});

// ───────── 保存済みの印・重複 ─────────
let savedKeys = new Map(); // 動画の見分け用キー → 保存した記録

/** 同じ動画かを見分けるキー(YouTube は動画ID、ほかは追跡用の値を除いた URL) */
function videoKey(url) {
  const youtube = url.match(/(?:youtu\.be\/|[?&]v=|\/shorts\/|\/live\/)([\w-]{11})/);
  if (youtube) return `yt:${youtube[1]}`;
  return url.replace(/[?&](si|utm_[a-z]+|feature|ref)=[^&]*/g, "").replace(/\/$/, "");
}

async function loadSavedKeys() {
  const list = await invoke("list_history").catch(() => []);
  savedKeys = new Map(list.filter((e) => e.exists).map((e) => [videoKey(e.url), e]));
}
listen("history_changed", loadSavedKeys);

/** 保存済みなら印の文(いつ保存したか)を返す */
function savedLabel(url) {
  const entry = savedKeys.get(videoKey(url));
  if (!entry) return "";
  return `✓ ${t("保存済み")} ${new Date(entry.saved_at * 1000).toLocaleDateString(document.documentElement.lang)}`;
}

/** 保存済みの動画を保存しようとした時の扱い。保存してよければ true */
function allowDuplicate(url) {
  if (!savedKeys.has(videoKey(url))) return true;
  if (settings.on_duplicate === "allow") return true;
  if (settings.on_duplicate === "skip") {
    showStatus(t("保存済みのため飛ばしました"));
    return false;
  }
  return confirm(t("この動画は保存済みです。もう一度保存しますか?"));
}

// ───────── URL リストの読み込み(ファイル・ドロップ・「送る」メニュー) ─────────
async function importText(text) {
  const urls = [...new Set(extractUrls(text))];
  if (urls.length === 0) return showStatus(t("URL が見つかりませんでした"), "error");
  const fresh = settings.on_duplicate === "skip" ? urls.filter((u) => !savedKeys.has(videoKey(u))) : urls;
  const { added, skipped } = await invoke("add_downloads", { text: fresh.join("\n"), overrides: presetValues });
  const removed = skipped + urls.length - fresh.length;
  const message = t("{n} 件を追加しました", { n: added });
  showStatus(removed ? `${message}(${t("重複 {n} 件は除外", { n: removed })})` : message);
  openTab("jobs");
}

async function importFiles(files) {
  const texts = await Promise.all([...files].map((f) => f.text()));
  // .url ショートカットの "URL=" も拾えるようにする
  importText(texts.join("\n").replaceAll("URL=", " "));
}

listen("incoming_urls", ({ payload }) => importText(payload));

// ───────── 空き容量・省電力 ─────────
async function refreshSystemStatus() {
  const status = await invoke("system_status").catch(() => null);
  if (!status) return;
  showConditions(status);
  $("disk-status").textContent = status.free_bytes != null ? t("空き {size}", { size: formatSize(status.free_bytes) }) : "";
}
function showConditions({ low_space, power_saving }) {
  $("low-space").classList.toggle("hidden", !low_space);
  $("power-status").classList.toggle("hidden", !power_saving);
}
listen("conditions", ({ payload }) => showConditions(payload));
setInterval(refreshSystemStatus, 30000);

$("add-send-to").addEventListener("click", async () => {
  try {
    await invoke("add_to_send_to");
    alert(t("「送る」メニューに追加しました"));
  } catch (e) {
    alert(String(e));
  }
});

// ───────── 字幕の言語を選ぶ ─────────
const SUBTITLE_LANGS = ["ja", "en", "ko", "zh-Hans", "zh-Hant", "es", "fr", "de", "pt", "ru", "it", "id", "th", "vi", "ar", "hi", "tr", "nl", "pl"];

function renderSubtitleLangPicker() {
  const input = document.querySelector('#settings [name="subtitle_langs"]');
  const chosen = input.value.split(",").map((l) => l.trim()).filter(Boolean);
  const set = (list) => {
    input.value = list.join(",");
    input.dispatchEvent(new Event("change", { bubbles: true }));
    renderSubtitleLangPicker();
  };
  const all = [...new Set([...SUBTITLE_LANGS, ...chosen.filter((l) => l !== "all")])];
  $("sub-lang-picker").replaceChildren(
    ...all.map((code) => el("button", {
      type: "button",
      className: `fn-chip ${chosen.includes(code) ? "active" : ""}`,
      textContent: `${languageName(code)} (${code})`,
      onclick: () => set(chosen.includes(code) ? chosen.filter((c) => c !== code) : [...chosen.filter((c) => c !== "all"), code]),
    })),
    el("button", {
      type: "button",
      className: `fn-chip ${chosen.includes("all") ? "active" : ""}`,
      textContent: t("すべての言語"),
      onclick: () => set(chosen.includes("all") ? ["ja"] : ["all"]),
    }),
  );
}

/** 更新の確認に失敗した理由を分かりやすく(まだ公開された版が無い時など) */
function updateErrorText(error) {
  const text = String(error);
  if (/release JSON|404|Not Found/i.test(text)) return t("まだ公開された版がありません(最初の版が出ると自動で更新されます)");
  if (/network|connect|dns|timed out/i.test(text)) return t("インターネットに接続できず、確認できませんでした");
  return `${t("確認できませんでした")}: ${text}`;
}

// ───────── 作者タブ・リンク ─────────
// data-url を持つボタンは押すとブラウザで開く
document.addEventListener("click", (e) => {
  const target = e.target.closest("[data-url]");
  if (target) invoke("open_url", { url: target.dataset.url });
});
invoke("app_version").then((v) => {
  $("about-version").textContent = v;
  $("app-version-now").textContent = `v${v}`;
  $("app-version-short").textContent = `Grabbit v${v}`;
}).catch(() => {});
$("about-update-check").addEventListener("click", async () => {
  $("about-update-status").textContent = t("確認しています…");
  try {
    const info = await invoke("check_app_update");
    $("about-update-status").textContent = info ? t("新しい版 {v} があります", { v: info.version }) : t("最新の版です");
    if (info) showAppUpdate(info);
  } catch (e) {
    $("about-update-status").textContent = updateErrorText(e);
  }
});

// ───────── 保存先の表示(左下) ─────────
function renderSaveLocation() {
  const path = settings.output_dir || "";
  const drive = path.match(/^([a-zA-Z]):/);
  $("save-drive").textContent = drive ? t("{d} ドライブ", { d: drive[1].toUpperCase() }) : "";
  const parts = path.replace(/^[a-zA-Z]:/, "").split(/[\\/]/).filter(Boolean);
  // 長い時は最後の2つだけ見せる(全体はマウスを載せると出る)
  $("save-path").textContent = (parts.length > 2 ? "… › " : "") + parts.slice(-2).join(" › ");
  $("open-folder").title = path;
}

// ───────── 部品の点検・修復 ─────────
// ───────── 部品ごとの管理(消す・入れ直す・取得元を選ぶ) ─────────
async function renderTools() {
  const tools = await invoke("list_tools").catch(() => []);
  const custom = settings.custom_repos || [];
  $("tool-list").replaceChildren(...tools.map((tool) => {
    const repo = el("select", {},
      new Option(`${t("標準")}(${tool.default_repo})`, ""),
      ...custom.filter((r) => r !== tool.default_repo).map((r) => new Option(r, r)));
    repo.value = tool.repo;
    repo.addEventListener("change", async (e) => {
      e.stopPropagation();
      settings.tool_repos = { ...(settings.tool_repos || {}) };
      if (repo.value) settings.tool_repos[tool.name] = repo.value; else delete settings.tool_repos[tool.name];
      await persistSettings();
      showStatus(t("取得元を変えました。「入れ直す」を押すと、その取得元から入れます"));
    });
    const run = (command, doneText) => async (e) => {
      e.target.disabled = true;
      try {
        await invoke(command, { name: tool.name });
        showStatus(t(doneText, { name: tool.label }));
      } catch (err) {
        showStatus(describeMessage(String(err)), "error");
      }
      renderTools();
    };
    // オン・オフ(yt-dlp は必ず使うので切れない)
    const toggle = el("input", { type: "checkbox", checked: tool.enabled, disabled: tool.name === "yt-dlp" });
    toggle.addEventListener("change", async (e) => {
      // 設定画面の自動保存に拾わせない(古い値で上書きされるため)
      e.stopPropagation();
      if (!toggle.checked && tool.name === "ffmpeg"
        && !confirm(t("FFmpeg をオフにすると、動画と音声を1つにまとめられないため画質が下がったり、変換・字幕の埋め込みなどが使えなくなります。オフにしますか?"))) {
        toggle.checked = true;
        return;
      }
      toggle.disabled = true; // 終わるまで二重に押せないようにする
      try {
        await invoke("set_tool_enabled", { name: tool.name, enabled: toggle.checked });
        // 画面側の設定も同じにしておく(後で設定を保存した時に元に戻らないように)
        const off = new Set(settings.disabled_tools || []);
        if (toggle.checked) off.delete(tool.name); else off.add(tool.name);
        settings.disabled_tools = [...off];
      } catch (err) {
        toggle.checked = !toggle.checked;
        showStatus(describeMessage(String(err)), "error");
      }
      renderTools();
    });
    return el("div", { className: `tool-row ${tool.enabled ? "" : "tool-off"}` },
      el("label", { className: "check", title: t("使う") }, toggle, el("b", { textContent: t(tool.label) })),
      el("small", { className: "muted", textContent: !tool.enabled ? t("オフ") : tool.installed ? formatSize(tool.size) : t("入っていません") }),
      el("label", {}, t("取得元"), repo),
      tool.enabled && button("入れ直す", run("reinstall_tool", "{name} を入れ直しました")),
      tool.enabled && tool.installed && button("削除", async (e) => {
        if (!confirm(t("{name} を消しますか?(必要な部品は次に使う時・次の起動時に入れ直されます)", { name: t(tool.label) }))) return;
        await run("remove_tool", "{name} を消しました")(e);
      }, "ghost small danger"));
  }));
}
$("repo-add").addEventListener("click", async () => {
  const repo = $("repo-add-input").value.trim().replace(/^https:\/\/github\.com\//, "").replace(/\/+$/, "");
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo) || /(^|\/)\.\.?($|\/)/.test(repo)) {
    return showStatus(t("「持ち主/リポジトリ名」の形で入れてください(例: yt-dlp/yt-dlp)"), "error");
  }
  settings.custom_repos = [...new Set([...(settings.custom_repos || []), repo])];
  await persistSettings();
  $("repo-add-input").value = "";
  renderTools();
});

$("repair-tools").addEventListener("click", async () => {
  $("ytdlp-version").textContent = t("点検しています…");
  try {
    const fixed = await invoke("repair_tools");
    $("ytdlp-version").textContent = fixed.length ? t("直しました: {list}", { list: fixed.join(", ") }) : t("問題はありませんでした");
  } catch (e) {
    $("ytdlp-version").textContent = `${t("直せませんでした")}: ${e}`;
  }
});
listen("tools_repaired", ({ payload }) => showStatus(t("ダウンロードの部品を自動で直しました: {list}", { list: payload.join(", ") })));

function onReady() {
  $("setup").classList.add("hidden");
  refreshSystemStatus();
  // 「送る」メニューや URL を渡して起動された時は、その URL を受け取る
  invoke("startup_urls").then((text) => text && importText(text)).catch(() => {});
  warnLoginExpiry();
  showStartupNotices();
  checkVersionChange();
  renderFaq();
  renderLater();
  renderLoginLock();
  if (!ui("guide_done")) showGuide();
  showVersion();
  setTimeout(() => invoke("list_sites").then((list) => (sites = list)).catch(() => {}), 3000);
}
// 初回の準備(部品のダウンロード)。「初期ダウンロード(yt-dlp) 45%」のように出す
let setupLabel = "";
function renderSetup(done = 0, total = 0, kind = "tools") {
  const percent = total ? Math.min(100, Math.floor((done / total) * 100)) : 0;
  // 受け取り始める前は「接続中」、受け取り終わった後は「展開中」と出して、止まって見えないようにする
  const finished = kind === "app" ? t("インストールしています…") : t("展開しています…");
  const phase = !done ? ` — ${t("接続しています…")}` : total && done >= total ? ` — ${finished}` : "";
  const amount = (total ? ` ${percent}%(${formatSize(done)} / ${formatSize(total)})` : done ? ` ${formatSize(done)}` : "") + phase;
  $("setup").replaceChildren(
    el("div", { textContent: `${t(setupLabel)}${amount}` }),
    total ? el("div", { className: "bar setup-bar" }, el("span", { style: `width:${percent}%` })) : "",
    el("small", { textContent: kind === "app"
      ? t("Grabbit 本体を新しい版にしています。終わると自動で再起動します")
      : setupLabel.startsWith("入れ直し") || readyOnce
      ? t("部品をダウンロードしています。終わるまでそのままお待ちください")
      : t("最初の1回だけ、動かすのに必要な部品をダウンロードしています。終わるまでそのままお待ちください") }));
  $("setup").classList.remove("hidden");
}
listen("setup", ({ payload }) => {
  setupLabel = payload;
  renderSetup();
});
listen("setup_progress", ({ payload: [done, total] }) => {
  if (setupLabel && !$("setup").classList.contains("hidden")) renderSetup(done, total);
});
listen("ready", onReady);

// ───────── 起動 ─────────
// 前回の見た目(テーマ・色・言語)を、設定を読み込む前にすぐ当てる
try {
  settings = { ui: JSON.parse(localStorage.getItem("grabbit-ui") || "{}") };
  applyAppearance();
  renderServices();
  renderSiteStrip();
} catch {
  // 初回起動などで無ければ、設定を読み込んでから当てる
}

/** 前の版の設定の後始末 */
function migrateOldUiSettings() {
  // 前の版では保存プリセットのタブが既定で隠れていたので、一度だけ出し直す
  if (settings.ui?.nav_hidden?.includes("presets") && !settings.ui.presets_tab_restored) {
    settings.ui.nav_hidden = settings.ui.nav_hidden.filter((id) => id !== "presets");
    settings.ui.presets_tab_restored = true;
    invoke("save_settings", { settings }).catch(() => {});
  }
  // 前の版の色の設定(触っていなくても黒が保存されてしまっていたもの)は使わずに消す
  if (settings.ui && ["color_bg", "color_panel", "color_text"].some((k) => k in settings.ui)) {
    for (const k of ["color_bg", "color_panel", "color_text"]) delete settings.ui[k];
    try { localStorage.removeItem("grabbit-ui"); } catch { /* 消せなくても困らない */ }
    invoke("save_settings", { settings }).catch(() => {});
  }
}

(async () => {
  settings = await invoke("get_settings");
  migrateOldUiSettings();
  activePreset = settings.ui?.active_preset || "";
  // 消したプリセットを使う設定のままなら「自分の設定」に戻す
  if (!allPresets().some((p) => p.id === activePreset)) activePreset = "";
  if (!activePreset && mineHidden()) activePreset = allPresets()[0].id;
  presetValues = { ...(allPresets().find((p) => p.id === activePreset)?.values || {}) };
  renderPresets();
  renderLater();
  await loadSavedKeys();
  (await invoke("list_jobs")).forEach((j) => jobs.set(j.id, j));
  lastSubs = await invoke("list_subscriptions");
  refreshAll();
  // 描き終わってからウィンドウを出す(空白の画面を見せない)
  requestAnimationFrame(() => invoke("show_window").catch(() => {}));
  if (await invoke("is_ready")) onReady();
  $("url").focus();
})();
