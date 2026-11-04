// ファイル名の組み立て。yt-dlp の書き方(%(title)s など)を知らなくても、
// 「フォルダ分け」と「名前に入れるもの」を選ぶだけで決められるようにする。
// 組み立てた結果は、隣の隠し入力欄(input)にテンプレートとして書き込む。

(() => {
  const { t } = window.I18N;

  // フォルダ分けの選択肢 → テンプレートのフォルダ部分
  const FOLDERS = [
    ["none", "分けない", ""],
    ["channel", "チャンネルごと", "%(uploader,channel|Unknown)s/"],
    ["site", "サイトごと", "%(extractor_key)s/"],
    ["playlist", "プレイリストごと", "%(playlist_title,playlist|Other)s/"],
    ["month", "投稿した年月ごと", "%(upload_date>%Y-%m|Unknown)s/"],
    ["channel_playlist", "チャンネル → プレイリスト", "%(uploader,channel|Unknown)s/%(playlist_title,playlist|Other)s/"],
  ];

  // 名前に入れられるもの(この順番で並ぶ)
  const PARTS = [
    ["index", "番号(プレイリスト内)", "%(playlist_index|)s"],
    ["date", "投稿日", "%(upload_date>%Y-%m-%d|)s"],
    ["channel", "チャンネル名", "%(uploader,channel|)s"],
    ["title", "タイトル", "%(title).150B"],
    ["quality", "画質", "%(height|)sp"],
    ["id", "動画ID", "[%(id)s]"],
  ];

  const SEPARATORS = [[" - ", "ハイフン( - )"], ["_", "下線( _ )"], [" ", "空白"]];

  const DEFAULT_STATE = { folder: "channel", parts: ["title"], separator: " - " };

  /** 選んだ内容からテンプレートを作る */
  function build(state) {
    const folder = FOLDERS.find(([key]) => key === state.folder)?.[2] ?? "";
    const parts = PARTS.filter(([key]) => state.parts.includes(key)).map(([, , template]) => template);
    if (parts.length === 0) parts.push("%(title).150B");
    return `${folder}${parts.join(state.separator)}.%(ext)s`;
  }

  /** テンプレートが選択肢から作れる形なら、その選択内容に戻す(手書きなら null) */
  function parse(template) {
    for (const [folderKey] of FOLDERS) {
      for (const [separator] of SEPARATORS) {
        for (let mask = 1; mask < 1 << PARTS.length; mask++) {
          const state = { folder: folderKey, separator, parts: PARTS.filter((_, i) => mask & (1 << i)).map(([key]) => key) };
          if (build(state) === template) return state;
        }
      }
    }
    return null;
  }

  /** テンプレートを、例の値で実際のファイル名にして見せる */
  function example(template, sample) {
    const values = {
      title: "Me at the zoo", uploader: "jawed", channel: "jawed", id: "jNQXAC9IVRw", ext: "mp4",
      upload_date: "20050423", extractor_key: "Youtube", playlist_title: "My playlist", playlist: "My playlist",
      playlist_index: "1", height: "1080",
      ...Object.fromEntries(Object.entries(sample || {}).filter(([, v]) => v)),
    };
    return template.replace(/%\(([^)]+)\)[-#0+ \d.]*[a-zA-Z]/g, (whole, expr) => {
      const [field, fallback = ""] = expr.split("|");
      const [names, dateFormat] = field.split(">");
      const name = names.split(",").map((n) => n.split(".")[0]).find((n) => values[n]);
      if (!name) return fallback;
      const value = String(values[name]);
      if (dateFormat && /^\d{8}$/.test(value)) {
        return dateFormat.replace("%Y", value.slice(0, 4)).replace("%m", value.slice(4, 6)).replace("%d", value.slice(6, 8));
      }
      return value;
    });
  }

  /** フォルダ分けの例: 「保存先 › jawed › 動画」の形 */
  function folderExample(template, sample) {
    const folders = example(template, sample).split("/").filter(Boolean);
    return [t("保存先"), ...folders, t("動画ファイル")].join(" › ");
  }

  /**
   * 組み立て画面を host の中に作る。
   * input: テンプレートを書き込む入力欄(設定フォームの項目など)
   * sample: 例に使う値を返す関数(プレビュー中の動画など)
   */
  function mount(host, input, sample = () => ({})) {
    const el = (tag, props = {}, ...children) => {
      const node = Object.assign(document.createElement(tag), props);
      node.append(...children.filter(Boolean));
      return node;
    };

    const folder = el("div", { className: "fn-folders" });
    const separator = el("select", { className: "fn-separator" });
    const parts = el("div", { className: "fn-parts" });
    const preview = el("div", { className: "fn-example" });
    const raw = el("input", { className: "fn-raw", spellcheck: false });
    const rawBox = el("details", { className: "fn-raw-box" }, el("summary", { textContent: t("直接書く(上級者向け)") }), raw);
    host.replaceChildren(
      el("div", { className: "fn-label", textContent: t("① 保存先の中をどうフォルダ分けするか") }),
      folder,
      el("div", { className: "fn-label", textContent: t("② ファイル名に入れるもの(押した順ではなく下の順に並びます)") }),
      parts,
      el("div", { className: "fn-row" }, el("label", {}, t("区切り"), separator)),
      preview,
      rawBox,
    );

    let state = { ...DEFAULT_STATE };

    function render() {
      // フォルダ分けは、実際にできるフォルダの形を例で見せて選んでもらう
      folder.replaceChildren(...FOLDERS.map(([key, label, template]) => el("button", {
        type: "button",
        className: `fn-folder-card ${key === state.folder ? "active" : ""}`,
        onclick: () => {
          state.folder = key;
          commit(build(state));
        },
      },
        el("b", { textContent: t(label) }),
        el("code", { textContent: folderExample(template, sample()) }))));
      separator.replaceChildren(...SEPARATORS.map(([value, label]) => new Option(t(label), value, false, value === state.separator)));
      parts.replaceChildren(...PARTS.map(([key, label]) => el("button", {
        type: "button",
        className: `fn-chip ${state.parts.includes(key) ? "active" : ""}`,
        textContent: t(label),
        onclick: () => {
          state.parts = state.parts.includes(key) ? state.parts.filter((p) => p !== key) : [...state.parts, key];
          if (state.parts.length === 0) state.parts = ["title"];
          commit(build(state));
        },
      })));
      preview.textContent = `${t("例:")} ${t("保存先")} › ${example(input.value, sample()).split("/").join(" › ")}`;
      raw.value = input.value;
    }

    function commit(template) {
      input.value = template;
      input.dispatchEvent(new Event("change", { bubbles: true }));
      render();
    }

    separator.addEventListener("change", () => {
      state.separator = separator.value;
      commit(build(state));
    });
    raw.addEventListener("change", () => {
      state = parse(raw.value) || state;
      commit(raw.value || build(state));
    });

    /** 入力欄の値が外から変わった時に呼ぶ */
    function refresh() {
      const parsed = parse(input.value);
      if (parsed) state = parsed;
      rawBox.open = !parsed && Boolean(input.value);
      render();
    }
    refresh();
    return { refresh };
  }

  window.FilenameBuilder = { mount, build, parse, example, DEFAULT_STATE };
})();
