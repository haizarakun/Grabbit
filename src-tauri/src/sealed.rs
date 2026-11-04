//! 設定・記録ファイルの暗号化(Windows の DPAPI: このパソコンのこのユーザーでしか開けない)。
//! 暗号化したファイルは先頭に印(GRBENC1)を付け、読む時は印を見て自動で戻す。
//! 印が無いものはそのまま読むので、暗号化をやめても前のファイルを読める。

use std::sync::atomic::{AtomicBool, Ordering};

const MARK: &[u8] = b"GRBENC1\n";

/// 書く時に暗号化するか(設定の「記録を暗号化して保存する」)
static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::SeqCst);
}

/// 書く形にする。暗号化できない環境では、そのままの文字にする
pub fn seal(text: &str) -> Vec<u8> {
    if ENABLED.load(Ordering::SeqCst) {
        if let Some(sealed) = crate::login::protect_bytes(text.as_bytes()) {
            let mut out = MARK.to_vec();
            out.extend(sealed);
            return out;
        }
    }
    text.as_bytes().to_vec()
}

/// 読んだ中身を文字に戻す。暗号化されていなければそのまま
pub fn open(bytes: &[u8]) -> Option<String> {
    match bytes.strip_prefix(MARK) {
        Some(sealed) => crate::login::unprotect_bytes(sealed).and_then(|plain| String::from_utf8(plain).ok()),
        None => String::from_utf8(bytes.to_vec()).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(open(&seal("{\"a\":1}")).as_deref(), Some("{\"a\":1}"));
    }
}
