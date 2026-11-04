//! 音楽の分析と加工: テンポ(BPM)・キーを調べる、歌声を弱める(カラオケ版)・強める。
//! 音の解析は ffmpeg で音を数字の並び(PCM)にしてから、ここで計算する。

use crate::download::command;
use crate::tools;
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

const RATE: usize = 11_025; // 解析用に落とした標本化周波数(テンポとキーには十分)

fn ffmpeg() -> PathBuf {
    tools::bin_dir().join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" })
}

/// 曲の真ん中あたり 60 秒を、モノラルの数字の並びにする
async fn decode(file: &Path) -> Result<Vec<f32>> {
    let mut cmd = command(&ffmpeg());
    cmd.args(["-v", "error", "-ss", "30", "-t", "60", "-i"])
        .arg(file)
        .args(["-ac", "1", "-ar", &RATE.to_string(), "-f", "f32le", "-"]);
    let output = cmd.output().await?;
    if !output.status.success() || output.stdout.len() < RATE * 4 * 5 {
        // 30 秒より短い曲は頭から読み直す
        let mut cmd = command(&ffmpeg());
        cmd.args(["-v", "error", "-t", "60", "-i"]).arg(file).args(["-ac", "1", "-ar", &RATE.to_string(), "-f", "f32le", "-"]);
        let output = cmd.output().await?;
        if output.stdout.len() < RATE * 4 * 5 {
            bail!("音を読み取れませんでした");
        }
        return Ok(to_samples(&output.stdout));
    }
    Ok(to_samples(&output.stdout))
}

fn to_samples(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

/// テンポ(BPM)。音の強さの変わり目の繰り返しの間隔から求める(60〜180 の範囲)
pub fn estimate_bpm(samples: &[f32]) -> Option<f32> {
    const HOP: usize = 256;
    // 短い区間ごとの音の強さ → 強くなった分だけを拾う(拍の頭)
    let energy: Vec<f32> = samples.chunks(HOP).map(|c| c.iter().map(|s| s * s).sum::<f32>()).collect();
    let raw: Vec<f32> = energy.windows(2).map(|w| (w[1] - w[0]).max(0.0)).collect();
    // 拍の位置が区間の境目でずれても拾えるよう、前後をならす
    let onset: Vec<f32> = raw.windows(3).map(|w| w[0] * 0.5 + w[1] + w[2] * 0.5).collect();
    if onset.len() < 100 {
        return None;
    }
    let frames_per_sec = RATE as f32 / HOP as f32;
    let lag_for = |bpm: f32| (60.0 * frames_per_sec / bpm).round() as usize;
    let (min_lag, max_lag) = (lag_for(180.0), lag_for(60.0));
    let mut best = (0usize, 0f32);
    for lag in min_lag..=max_lag.min(onset.len() - 1) {
        // 重なる長さで割り、よくあるテンポ(120 前後)を少し優先する(倍・半分の取り違えを防ぐ)
        let overlap = (onset.len() - lag) as f32;
        let bpm = 60.0 * frames_per_sec / lag as f32;
        let prior = (-0.5 * (bpm / 120.0).log2().powi(2)).exp();
        let score: f32 = onset.iter().zip(&onset[lag..]).map(|(a, b)| a * b).sum::<f32>() / overlap * prior;
        if score > best.1 {
            best = (lag, score);
        }
    }
    (best.0 > 0).then(|| (60.0 * frames_per_sec / best.0 as f32 * 10.0).round() / 10.0)
}

/// キー(例: "C" / "Am")。12 の音の高さの強さを集め、長調・短調の型(Krumhansl)と比べる
pub fn estimate_key(samples: &[f32]) -> Option<String> {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    const MAJOR: [f32; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
    const MINOR: [f32; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];
    const BLOCK: usize = 4096;
    let mut chroma = [0f32; 12];
    // C3〜B5 の各音の強さを、ゲルツェル法で 1 音ずつ測る
    for block in samples.chunks(BLOCK).step_by(2) {
        if block.len() < BLOCK {
            break;
        }
        for midi in 48..84 {
            let freq = 440.0 * 2f32.powf((midi as f32 - 69.0) / 12.0);
            let w = 2.0 * std::f32::consts::PI * freq / RATE as f32;
            let coeff = 2.0 * w.cos();
            let (mut s1, mut s2) = (0f32, 0f32);
            for &x in block {
                let s0 = x + coeff * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            chroma[midi % 12] += (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt();
        }
    }
    if chroma.iter().sum::<f32>() <= 0.0 {
        return None;
    }
    let correlation = |profile: &[f32; 12], root: usize| {
        let mean_c = chroma.iter().sum::<f32>() / 12.0;
        let mean_p = profile.iter().sum::<f32>() / 12.0;
        let (mut num, mut dc, mut dp) = (0f32, 0f32, 0f32);
        for i in 0..12 {
            let c = chroma[(i + root) % 12] - mean_c;
            let p = profile[i] - mean_p;
            num += c * p;
            dc += c * c;
            dp += p * p;
        }
        num / (dc.sqrt() * dp.sqrt()).max(1e-9)
    };
    let mut best = (f32::MIN, String::new());
    for root in 0..12 {
        for (profile, suffix) in [(&MAJOR, ""), (&MINOR, "m")] {
            let r = correlation(profile, root);
            if r > best.0 {
                best = (r, format!("{}{suffix}", NAMES[root]));
            }
        }
    }
    Some(best.1)
}

/// テンポとキーを調べる
pub async fn analyze(file: &Path) -> Result<(Option<f32>, Option<String>)> {
    let samples = decode(file).await?;
    Ok((estimate_bpm(&samples), estimate_key(&samples)))
}

/// 歌声を弱めた版(カラオケ風)/ 歌声を強めた版を作る。
/// 真ん中から聞こえる音(多くの曲では歌声)を引く・残すだけの簡単な方法なので、曲によって効き方が違う
pub async fn vocal_version(file: &Path, keep_vocals: bool) -> Result<PathBuf> {
    let label = if keep_vocals { "vocals" } else { "karaoke" };
    let stem = file.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let ext = file.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "m4a".into());
    let ext = if matches!(ext.as_str(), "mp3" | "m4a" | "flac" | "wav" | "opus" | "ogg") { ext } else { "m4a".into() };
    let mut output = file.with_file_name(format!("{stem} [{label}].{ext}"));
    let mut n = 2;
    while output.exists() {
        output = file.with_file_name(format!("{stem} [{label}] ({n}).{ext}"));
        n += 1;
    }
    let filter = if keep_vocals {
        // 左右に共通の音(真ん中)だけを残し、歌の高さの範囲を通す
        "pan=mono|c0=0.5*c0+0.5*c1,highpass=f=150,lowpass=f=6000"
    } else {
        // 左右の差だけを残すと真ん中の歌声が消える。低音も消えるので、低音だけ足し戻す
        "asplit[a][b];[a]pan=stereo|c0=c0-c1|c1=c1-c0[side];[b]lowpass=f=150[bass];[side][bass]amix=inputs=2:weights=1 1"
    };
    let mut cmd = command(&ffmpeg());
    cmd.args(["-y", "-loglevel", "error", "-i"]).arg(file).args(["-vn", "-af", filter]).arg(&output);
    let result = cmd.output().await?;
    if !result.status.success() {
        let _ = std::fs::remove_file(&output);
        bail!("作れませんでした(モノラルの曲では使えません): {}", String::from_utf8_lossy(&result.stderr).trim());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bpm_of_a_click_track() {
        // 120 BPM(0.5 秒ごと)のクリック音
        let mut samples = vec![0f32; RATE * 20];
        for beat in 0..40 {
            let start = beat * RATE / 2;
            for s in &mut samples[start..start + 200] {
                *s = 0.9;
            }
        }
        let bpm = estimate_bpm(&samples).unwrap();
        assert!((bpm - 120.0).abs() < 3.0, "{bpm}");
    }

    #[test]
    fn key_of_a_c_major_chord() {
        let tone = |f: f32, i: usize| (2.0 * std::f32::consts::PI * f * i as f32 / RATE as f32).sin();
        let samples: Vec<f32> = (0..RATE * 6).map(|i| tone(261.63, i) + tone(329.63, i) + tone(392.0, i)).collect();
        assert_eq!(estimate_key(&samples).as_deref(), Some("C"));
    }
}
