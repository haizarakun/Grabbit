# Grabbit

[日本語](#日本語) | [English](#english)

## 目次 / Contents

| 日本語 | English |
|---|---|
| [対応 OS](#対応-os) | [Supported OS](#supported-os) |
| [対応言語](#対応言語) | [Languages](#languages) |
| [はじめての方へ(インストール方法)](#はじめての方へインストール方法) | [Getting started](#getting-started-installation) |
| [使い方](#使い方) | [How to use](#how-to-use) |
| [できること](#できること) | [Features](#features) |
| [困ったとき](#困ったとき) | [Troubleshooting](#troubleshooting) |
| [その他(翻訳・注意・ライセンスなど)](#その他--other) | [Other (translations, notice, license…)](#その他--other) |

---

## 日本語

URL を貼るだけで、動画・音声・画像・サムネイル・字幕を保存できる、無料・オープンソースのダウンローダーです。

### 対応 OS
| OS | 対応バージョン | ダウンロードするファイル |
|---|---|---|
| Windows | 10 / 11 (64 ビット) | `Grabbit_x.x.x_x64-setup.exe` |
| macOS | 11 Big Sur 以降 (Intel / Apple シリコン両対応)※動作未確認 | `Grabbit_x.x.x_universal.dmg` |
| Linux | Ubuntu 22.04 以降など (64 ビット) | `Grabbit_x.x.x_amd64.AppImage` または `.deb` |

※ macOS 版はビルドできることだけ確認していて、実機ではまだ試していません。動かない時は Discord で教えてください。

### 対応言語
日本語 / English / 简体中文 / 繁體中文 / 한국어 / Español / Français / Deutsch / Português / Русский(設定画面で切り替え)

### はじめての方へ(インストール方法)
OS を入れたばかりの何も入っていない PC でも、下の手順だけで使えます。
yt-dlp・FFmpeg・gallery-dl・Deno などの部品は、アプリが初回起動時に自動でダウンロードするので、自分で入れる必要はありません(インターネット接続が必要です)。

1. [Releases ページ](https://github.com/haizarakun/Grabbit/releases/latest) を開きます。
2. 下の方の **Assets** から、上の表の自分の OS 用のファイルをダウンロードします。
3. OS ごとに次の手順で起動します。

**Windows**
事前に入れるもの: なし。画面表示に使う「Microsoft Edge WebView2」が無い PC では、インストーラーが自動で入れます。
1. `…-setup.exe` をダブルクリックします。
2. 「Windows によって PC が保護されました」と出たら、**詳細情報** → **実行** を押します(新しいアプリなので出ることがあります)。
3. 画面どおりに進めると、スタートメニューとデスクトップに Grabbit ができます。

**macOS**
事前に入れるもの: なし(Homebrew なども不要)。
1. `.dmg` を開き、Grabbit を **アプリケーション** フォルダへドラッグします。
2. 初回は Grabbit を **右クリック → 開く** → **開く** を押します。
   「壊れているため開けません」と出たら、ターミナルで次を実行してからもう一度開いてください。
   ```
   xattr -cr /Applications/Grabbit.app
   ```

**Linux**
- **.deb(Ubuntu / Debian / Linux Mint など・おすすめ)**: 必要なライブラリ(WebKitGTK など)も一緒に自動で入ります。ダウンロードしたフォルダでターミナルを開いて:
  ```
  sudo apt update
  sudo apt install ./Grabbit_*.deb
  ```
  アプリ一覧に Grabbit が出ます。
- **AppImage(Fedora / Arch など、その他のディストリビューション)**: 動かすのに FUSE 2 が必要です。無い場合は先に入れます。
  | ディストリビューション | コマンド |
  |---|---|
  | Ubuntu 24.04 以降 | `sudo apt install libfuse2t64` |
  | Ubuntu 22.04 / Debian | `sudo apt install libfuse2` |
  | Fedora | `sudo dnf install fuse-libs` |
  | Arch | `sudo pacman -S fuse2` |

  そのあと、ファイルを右クリック → プロパティ → 「実行可能」にチェック(またはターミナルで `chmod +x Grabbit_*.AppImage`)して、ダブルクリックで起動します。

### 使い方
1. 起動すると、最初の 1 回だけ必要な部品(yt-dlp・ffmpeg など)を自動でダウンロードします。少し待ってください。
2. 保存したい動画のページの URL をコピーして、Grabbit の入力欄に貼ります。→「後で保存」に入ります。
3. 「保存」を押すとダウンロードが始まります。中身を確かめてから保存したい時は、URL を入れて **調べる** を押します。
4. 保存したファイルは **ライブラリ** タブで見られます。保存先は **設定** で変えられます。
5. よく使う保存方法は **保存プリセット** タブで登録できます。

アプリと部品は自動で最新に更新されます。

### できること
- **1,700 以上のサイトに対応**: YouTube・ニコニコ・X・TikTok・Instagram・Twitch・bilibili・SoundCloud・pixiv など
- 動画 / 音声だけ / サムネイルだけ / 字幕だけ / 概要・情報だけ を選んで保存
- 画質・形式・ファイル名・保存先・字幕の言語・一部分だけの保存などを細かく設定
- チャンネルやプレイリストを登録して新しい投稿を自動保存
- 会員限定・年齢制限の動画は、アプリ内でログインして保存
- テーマ・色・文字の大きさなど見た目を自由に変更

### 困ったとき
- 保存できない時は、失敗した行の **診断** を押すと原因と対処が表示されます。
- 質問・不具合・要望は [Discord](https://discord.gg/hXv7G3xtVf) で受け付けています。お知らせもここで出します。

---

## English

A free, open-source downloader: paste a URL to save videos, audio, images, thumbnails and subtitles.

### Supported OS
| OS | Versions | File to download |
|---|---|---|
| Windows | 10 / 11 (64-bit) | `Grabbit_x.x.x_x64-setup.exe` |
| macOS | 11 Big Sur or later (Intel and Apple silicon) — not yet tested | `Grabbit_x.x.x_universal.dmg` |
| Linux | Ubuntu 22.04 or later, etc. (64-bit) | `Grabbit_x.x.x_amd64.AppImage` or `.deb` |

* The macOS build compiles but hasn't been tested on a real Mac yet. If it doesn't work, please tell us on Discord.

### Languages
日本語 / English / 简体中文 / 繁體中文 / 한국어 / Español / Français / Deutsch / Português / Русский (switch in Settings)

### Getting started (installation)
These steps work on a fresh OS install with nothing else on it.
The tools Grabbit uses (yt-dlp, FFmpeg, gallery-dl, Deno) are downloaded automatically on first launch, so you don't install them yourself (an internet connection is required).

1. Open the [Releases page](https://github.com/haizarakun/Grabbit/releases/latest).
2. Under **Assets**, download the file for your OS from the table above.
3. Launch it as follows.

**Windows**
Prerequisites: none. If "Microsoft Edge WebView2" (used to draw the window) is missing, the installer adds it automatically.
1. Double-click `…-setup.exe`.
2. If you see "Windows protected your PC", click **More info** → **Run anyway** (this can appear for new apps).
3. Follow the installer. Grabbit is added to the Start menu and desktop.

**macOS**
Prerequisites: none (no Homebrew needed).
1. Open the `.dmg` and drag Grabbit into **Applications**.
2. The first time, **right-click Grabbit → Open** → **Open**.
   If macOS says the app "is damaged", run this in Terminal and open it again:
   ```
   xattr -cr /Applications/Grabbit.app
   ```

**Linux**
- **.deb (Ubuntu / Debian / Linux Mint, etc. — recommended)**: required libraries such as WebKitGTK are installed automatically. Open a terminal in the download folder and run:
  ```
  sudo apt update
  sudo apt install ./Grabbit_*.deb
  ```
  Grabbit then appears in your app menu.
- **AppImage (Fedora, Arch and other distributions)**: needs FUSE 2. Install it first if missing:
  | Distribution | Command |
  |---|---|
  | Ubuntu 24.04 or later | `sudo apt install libfuse2t64` |
  | Ubuntu 22.04 / Debian | `sudo apt install libfuse2` |
  | Fedora | `sudo dnf install fuse-libs` |
  | Arch | `sudo pacman -S fuse2` |

  Then right-click the file → Properties → tick "Allow executing" (or run `chmod +x Grabbit_*.AppImage`) and double-click it.

### How to use
1. On first launch Grabbit downloads the tools it needs (yt-dlp, ffmpeg, etc.) once. Please wait a moment.
2. Copy the URL of a video page and paste it into Grabbit. It goes into **Save later**.
3. Press **Save** to start downloading. To check the contents first, enter the URL and press **Check**.
4. Saved files appear in the **Library** tab. Change the save folder in **Settings**.
5. Save your favourite settings in the **Presets** tab.

The app and its tools update themselves automatically.

### Features
- **1,700+ sites**: YouTube, Niconico, X, TikTok, Instagram, Twitch, bilibili, SoundCloud, pixiv and more
- Save video, audio only, thumbnail only, subtitles only, or info only
- Fine control over quality, format, file name, folder, subtitle language and clipping
- Subscribe to channels or playlists to save new uploads automatically
- Members-only / age-restricted videos via in-app login
- Themes, colours, font size and more

### Troubleshooting
- If a download fails, press **Diagnose** on that row to see the cause and a fix.
- Questions, bug reports, requests and announcements: [Discord](https://discord.gg/hXv7G3xtVf).

---

## その他 / Other

### 翻訳を手伝う / Translations
`ui/lang/en.js` をコピーして訳し、`ui/index.html` に `<script>` を 1 行足すだけで言語を増やせます。
Copy `ui/lang/en.js`, translate it, and add one `<script>` line to `ui/index.html` to add a language.

### 注意 / Notice
DRM で保護された動画は保存できません。各サイトの利用規約と、お住まいの国の著作権法を守って使ってください。
DRM-protected videos cannot be saved. Follow each site's terms of service and your local copyright law.

### 使っているもの / Built with
[yt-dlp](https://github.com/yt-dlp/yt-dlp)、[gallery-dl](https://github.com/mikf/gallery-dl)、[FFmpeg](https://ffmpeg.org/)、[Deno](https://deno.com/)、[Tauri](https://tauri.app/)

### ライセンス / License
MIT

### コミュニティ / Community
- GitHub: https://github.com/haizarakun/Grabbit
- Discord: https://discord.gg/hXv7G3xtVf

### 名前・アイコンについて / Trademark
コードは MIT ライセンスで自由に使えますが、「Grabbit」の名前とアイコンは作者のものです。
改変版・派生版を配布する時は、別の名前とアイコンを使い、公式版と誤解されないようにしてください。
公式の配布元は https://github.com/haizarakun/Grabbit だけです。それ以外の場所にある「Grabbit」は非公式です。

The source code is MIT-licensed, but the "Grabbit" name and icon are not covered by that license.
Forks and modified versions must use a different name and icon and must not present themselves as the official app.
The only official source is https://github.com/haizarakun/Grabbit.
