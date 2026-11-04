// 多言語対応。画面の文言は日本語の原文をキーにして、各言語の訳へ置き換える。
// 訳は lang/*.js が KEYS と同じ順番の配列で登録する(I18N.add)。
// 新しい言語を足す時は lang/en.js をコピーして訳し、index.html に <script> を1行足すだけ。
(() => {
  // 訳す対象の原文(この順番が各言語ファイルの配列の順番になる)
  const KEYS = [
    "ダウンロード",
    "自動保存",
    "対応サイト",
    "設定",
    "保存先を開く",
    "URL を貼り付け・ドロップ(複数行でまとめてダウンロード)",
    "調べる",
    "ダウンロードできます",
    "保存する形式",
    "閉じる",
    "絞り込み",
    "すべて",
    "進行中",
    "完了",
    "失敗",
    "完了分を消す",
    "URL を貼るだけ。何がダウンロードできるか調べてから保存します。",
    "チャンネルやプレイリストを登録すると、新しい動画を自動で保存します。",
    "チャンネル / プレイリストの URL",
    "登録",
    "今後の新着だけ保存する(過去の動画は落とさない)",
    "今すぐ確認",
    "まだ登録がありません。",
    "動画は yt-dlp、画像・イラストは gallery-dl、ファイルの直リンクは直接保存。順に自動で試します。",
    "サイト名で検索(例: niconico, pixiv, twitch)",
    "不調のサイトを隠す",
    "表示・言語",
    "言語",
    "テーマ",
    "OS に合わせる",
    "ライト",
    "ダーク",
    "アクセントカラー",
    "文字の大きさ",
    "一覧の見た目",
    "リスト",
    "サムネイル大きめ",
    "コンパクト",
    "一覧の見出しに出すもの",
    "動画タイトル",
    "ファイル名",
    "エラーの表示",
    "わかりやすい説明",
    "原文そのまま",
    "両方",
    "サムネイル",
    "サイトのアイコン",
    "保存先のパス",
    "速度・残り時間",
    "取得方法(yt-dlp など)",
    "対応サイトのアイコン列",
    "URL を貼ったらすぐ調べる",
    "基本",
    "保存先",
    "開く",
    "画質",
    "最高画質",
    "4K まで",
    "1080p まで",
    "720p まで",
    "音声のみ (MP3)",
    "音声のみ (M4A)",
    "ログインが必要な動画用",
    "使わない",
    "Firefox(おすすめ)",
    "Chrome / Edge は起動中だと読めないことがあります。その時はログインなしで自動的にやり直します。",
    "自動保存の確認間隔(分)",
    "yt-dlp の更新チャンネル",
    "stable(安定版)",
    "nightly(毎日更新・おすすめ)",
    "master(最新・不安定)",
    "今すぐ更新",
    "詳細設定",
    "画質・形式",
    "動画の形式",
    "MP4(どこでも再生可)",
    "MKV(高品質)",
    "優先する映像コーデック",
    "おまかせ",
    "H.264(互換性重視)",
    "AV1(高圧縮)",
    "音声のビットレート(例: 192K、空欄で最高)",
    "動画の再エンコード",
    "しない(速い)",
    "変換後も元のファイルを残す",
    "同じ画質ならフリーな形式(WebM 等)を優先",
    "埋め込み・後処理",
    "サムネイルを埋め込む",
    "タイトル・チャプター等を埋め込む",
    "字幕を埋め込む",
    "チャプターごとにファイルを分ける",
    "スポンサー区間を自動カット (YouTube)",
    "カットする区間",
    "字幕・追加ファイル",
    "字幕の言語(例: ja,en)",
    "字幕の形式",
    "そのまま",
    "サムネイルの形式",
    "自動生成字幕も使う",
    "字幕を別ファイルでも保存",
    "サムネイルを別ファイルで保存",
    "概要欄を保存",
    "動画情報(JSON)を保存",
    "ショート動画を除外",
    "ライブ配信・配信予定を除外",
    "この日付以降だけ(YYYYMMDD / now-7days)",
    "1回で落とす最大本数(0 で無制限)",
    "最小サイズ(例: 10M)",
    "最大サイズ(例: 2G)",
    "プレイリストの範囲(例: 1-10,15)",
    "一部だけ保存(時間指定)",
    "条件式(--match-filter)",
    "保存先・ファイル名",
    "ファイル名・フォルダ分け",
    "例:",
    "作業用フォルダ(空欄で保存先と同じ)",
    "ファイル名を英数字だけにする",
    "保存済みの動画はスキップする(重複防止)",
    "一時ファイル(.part)を使わない",
    "速度・通信",
    "同時ダウンロード数",
    "分割ダウンロード数",
    "速度制限(例: 5M)",
    "再試行回数",
    "動画ごとの待ち時間(秒)",
    "通信タイムアウト(秒、0 で既定)",
    "プロキシ",
    "ブラウザ偽装",
    "自動(失敗した時だけ)",
    "常に Chrome として接続",
    "常に Edge として接続",
    "常に Safari として接続",
    "常に Firefox として接続",
    "cookies.txt の場所",
    "地域制限の回避を試みる",
    "IPv4 で接続する",
    "ライブ配信",
    "配信の最初から録画する",
    "開始前の配信は始まるまで待つ",
    "上級者向け",
    "yt-dlp 追加オプション",
    "初期設定に戻す",
    "待機中",
    "ダウンロード中",
    "キャンセル",
    "おまかせ最高画質",
    "音声のみ MP3",
    "音声のみ M4A",
    "保存済みのためスキップ",
    "ブラウザのログイン情報を読めないため、ログインなしで再挑戦中…",
    "ブラウザ偽装で再挑戦中…",
    "画像・ギャラリー用の方法で再挑戦中…",
    "ファイルとして直接保存を試しています…",
    "ログイン情報なしで保存しました",
    "ブラウザのログイン情報を読めませんでした。Chrome / Edge を閉じてから試すか、設定で Firefox か cookies.txt を使ってください。",
    "この URL からは動画や画像が見つかりませんでした。",
    "非公開の動画です。見られるアカウントのログイン情報が必要です。",
    "ログインが必要です。設定の「ログインが必要な動画用」でブラウザを選んでください。",
    "年齢制限があります。ログイン情報が必要です。",
    "お住まいの地域では見られない動画です。",
    "アクセスが多すぎて制限されています。しばらく待つか、待ち時間を設定してください。",
    "サイトに拒否されました。ブラウザ偽装やログイン情報で通ることがあります。",
    "ページが見つかりません。URL を確認してください。",
    "DRM(コピー防止)付きのため保存できません。",
    "通信がタイムアウトしました。回線を確認してもう一度試してください。",
    "動画の結合・変換に失敗しました。",
    "保存先の空き容量が足りません。",
    "保存先に書き込めません。保存先を変えてください。",
    "ほか 1700 以上のサイトに対応 →",
    "まとめてダウンロード",
    "URL を貼り付けてください",
    "{n} 件を追加しました",
    "何がダウンロードできるか調べています…",
    "ダウンロードできませんでした",
    "画像・ギャラリー",
    "ファイル直リンク",
    "{n} 件",
    "かんたん",
    "個別の形式",
    "音声",
    "{n} 件をダウンロード",
    "ダウンロードを開始しました",
    "全 {n} 件",
    "進行中 {n}",
    "完了 {n}",
    "失敗 {n}",
    "残り {time}",
    "中止",
    "再試行",
    "フォルダ",
    "読み込み中…",
    "読み込めませんでした",
    "{shown} 件表示 / 全 {total} 件(正常 {ok})",
    "不調",
    "登録できませんでした",
    "未確認",
    "最終確認 {time}",
    "停止",
    "再開",
    "削除",
    "「{name}」の登録を解除しますか?(保存済みの動画は消えません)",
    "{n} 分ごとに自動確認",
    "保存しました",
    "設定を初期状態に戻しますか?",
    "現在: {v}",
    "未導入",
    "更新しています…",
    "yt-dlp をダウンロードしています…",
    "ffmpeg をダウンロードしています…",
    "Deno(YouTube対策用)をダウンロードしています…",
  ];

  const LANGUAGES = { ja: { name: "日本語", rtl: false } };
  const dictionaries = {};
  let current = "ja";

  function add(code, name, translations, options = {}) {
    if (translations.length !== KEYS.length) {
      console.warn(`[i18n] ${code}: 訳の数 ${translations.length} が原文の数 ${KEYS.length} と違います`);
    }
    LANGUAGES[code] = { name, rtl: Boolean(options.rtl) };
    dictionaries[code] = new Map(KEYS.map((key, i) => [key, translations[i]]).filter(([, v]) => v));
  }

  // 後から増えた文言(I18N.extend で登録)。KEYS と同じく画面の固定文言として訳す
  const extraKeys = new Set();

  function extend(code, map) {
    if (!dictionaries[code]) dictionaries[code] = new Map();
    for (const [key, value] of Object.entries(map)) {
      extraKeys.add(key);
      dictionaries[code].set(key, value);
    }
  }

  /** 設定値("auto" か言語コード)から、実際に使う言語を決める。未対応の言語は英語にする */
  function resolveLanguage(preference) {
    if (preference && preference !== "auto" && LANGUAGES[preference]) return preference;
    for (const tag of navigator.languages || [navigator.language]) {
      if (LANGUAGES[tag]) return tag;
      if (tag.startsWith("zh")) return /TW|HK|MO|Hant/i.test(tag) ? "zh-TW" : "zh-CN";
      const base = tag.split("-")[0];
      const match = Object.keys(LANGUAGES).find((code) => code === base || code.startsWith(`${base}-`));
      if (match) return match;
    }
    return "en";
  }

  /** 原文を今の言語に訳す。{name} の部分は params の値で埋める */
  function t(text, params = {}) {
    // その言語の訳が無ければ英語、それも無ければ日本語のまま出す
    const translated = current === "ja" ? text : dictionaries[current]?.get(text) ?? dictionaries.en?.get(text) ?? text;
    return translated.replace(/\{(\w+)\}/g, (whole, name) => (name in params ? params[name] : whole));
  }

  const normalize = (s) => s.replace(/\s+/g, " ").trim();
  const originalText = new WeakMap(); // テキストノード → 原文
  const ATTRIBUTES = ["placeholder", "title", "label"];

  /** 画面に書かれた固定の文言をまとめて訳す */
  function applyTranslations(lang) {
    current = lang;
    const known = new Set([...KEYS, ...extraKeys]);
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (!originalText.has(node)) {
        const key = normalize(node.textContent);
        if (!known.has(key)) continue;
        originalText.set(node, { key, raw: node.textContent });
      }
      const { key, raw } = originalText.get(node);
      const lead = raw.match(/^\s*/)[0];
      const trail = raw.match(/\s*$/)[0];
      node.textContent = lead + t(key) + trail;
    }
    for (const attr of ATTRIBUTES) {
      for (const element of document.querySelectorAll(`[${attr}]`)) {
        const store = `i18n${attr}`;
        if (!(store in element.dataset)) {
          if (!known.has(normalize(element.getAttribute(attr)))) continue;
          element.dataset[store] = normalize(element.getAttribute(attr));
        }
        element.setAttribute(attr, t(element.dataset[store]));
      }
    }
  }

  // has: 訳があるか(足りない訳を探す道具用)
  const has = (code, key) => Boolean(dictionaries[code]?.has(key));
  window.I18N = { KEYS, LANGUAGES, add, extend, t, has, resolveLanguage, applyTranslations };
})();
