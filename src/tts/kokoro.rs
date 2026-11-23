use super::TtsEngine;
use misaki_rs::{Language, G2P};
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};

#[allow(dead_code)]
pub const SAMPLE_RATE: u32 = 24000;

pub struct KokoroEngine {
    session: Mutex<Session>,
    vocab: HashMap<char, i64>,
    voices: HashMap<String, Vec<f32>>,
    g2p: Mutex<G2P>,
}

impl KokoroEngine {
    pub fn new(model_dir: &Path) -> Result<Self, String> {
        // Set synthesis worker thread to below-normal priority so other apps stay smooth
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }

        let onnx_path = model_dir.join("model_quantized.onnx");
        if !onnx_path.exists() {
            return Err(format!("Model file not found at: {}", onnx_path.display()));
        }

        let env = Arc::new(
            ort::init()
                .with_name("KokoroTTS")
                .build()
                .map_err(|e| format!("ORT init error: {}", e))?,
        );

        let session = Session::builder(&env)
            .map_err(|e| format!("ORT session builder error: {}", e))?
            .with_intra_threads(4)
            .map_err(|e| format!("ORT thread config error: {}", e))?
            .commit_from_file(&onnx_path)
            .map_err(|e| format!("ORT model load error: {}", e))?;

        // Load tokenizer.json vocab
        let tok_path = model_dir.join("tokenizer.json");
        let vocab = load_vocab(&tok_path)?;

        // Load available voice files
        let voices_dir = model_dir.join("voices");
        let mut voices = HashMap::new();

        if let Ok(entries) = fs::read_dir(&voices_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("bin") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if let Ok(vec) = load_voice_file(&path) {
                            voices.insert(stem.to_string(), vec);
                        }
                    }
                }
            }
        }

        if voices.is_empty() {
            return Err("No valid voice embeddings found in models directory".to_string());
        }

        let g2p = G2P::new(Language::EnglishUS);

        let engine = Self {
            session: Mutex::new(session),
            vocab,
            voices,
            g2p: Mutex::new(g2p),
        };

        // Startup warm-up synthesis to prime the ONNX runtime
        let _ = engine.synthesize("Warmup", "af_bella", 1.0);

        Ok(engine)
    }

    pub fn get_models_dir() -> PathBuf {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        let dir = PathBuf::from(appdata).join("KokoroTTS").join("models");
        let _ = fs::create_dir_all(&dir);
        let _ = fs::create_dir_all(dir.join("voices"));
        dir
    }

    pub fn download_models_if_needed<F>(progress: F) -> Result<PathBuf, String>
    where
        F: Fn(f32, &str) + Send + Sync,
    {
        let models_dir = Self::get_models_dir();
        let voices_dir = models_dir.join("voices");

        // List of core required files
        let files = [
            (
                "model_quantized.onnx",
                models_dir.join("model_quantized.onnx"),
                "https://huggingface.co/onnx-community/Kokoro-82M-ONNX/resolve/main/onnx/model_quantized.onnx",
                92_360_543u64,
                Some("kokoro_quantized.onnx"),
            ),
            (
                "tokenizer.json",
                models_dir.join("tokenizer.json"),
                "https://huggingface.co/onnx-community/Kokoro-82M-ONNX/raw/main/tokenizer.json",
                4_000u64,
                Some("tokenizer.json"),
            ),
            (
                "voices/af_bella.bin",
                voices_dir.join("af_bella.bin"),
                "https://huggingface.co/onnx-community/Kokoro-82M-ONNX/resolve/main/voices/af_bella.bin",
                524_288u64,
                Some("voices/af_bella.bin"),
            ),
            (
                "voices/af_nicole.bin",
                voices_dir.join("af_nicole.bin"),
                "https://huggingface.co/onnx-community/Kokoro-82M-ONNX/resolve/main/voices/af_nicole.bin",
                524_288u64,
                Some("voices/af_nicole.bin"),
            ),
            (
                "voices/af_sarah.bin",
                voices_dir.join("af_sarah.bin"),
                "https://huggingface.co/onnx-community/Kokoro-82M-ONNX/resolve/main/voices/af_sarah.bin",
                524_288u64,
                Some("voices/af_sarah.bin"),
            ),
            (
                "voices/af_sky.bin",
                voices_dir.join("af_sky.bin"),
                "https://huggingface.co/onnx-community/Kokoro-82M-ONNX/resolve/main/voices/af_sky.bin",
                524_288u64,
                Some("voices/af_sky.bin"),
            ),
            (
                "voices/am_adam.bin",
                voices_dir.join("am_adam.bin"),
                "https://huggingface.co/onnx-community/Kokoro-82M-ONNX/resolve/main/voices/am_adam.bin",
                524_288u64,
                Some("voices/am_adam.bin"),
            ),
            (
                "voices/am_michael.bin",
                voices_dir.join("am_michael.bin"),
                "https://huggingface.co/onnx-community/Kokoro-82M-ONNX/resolve/main/voices/am_michael.bin",
                524_288u64,
                Some("voices/am_michael.bin"),
            ),
            (
                "voices/bf_emma.bin",
                voices_dir.join("bf_emma.bin"),
                "https://huggingface.co/onnx-community/Kokoro-82M-ONNX/resolve/main/voices/bf_emma.bin",
                524_288u64,
                Some("voices/bf_emma.bin"),
            ),
            (
                "voices/bf_isabella.bin",
                voices_dir.join("bf_isabella.bin"),
                "https://huggingface.co/onnx-community/Kokoro-82M-ONNX/resolve/main/voices/bf_isabella.bin",
                524_288u64,
                Some("voices/bf_isabella.bin"),
            ),
            (
                "voices/bm_george.bin",
                voices_dir.join("bm_george.bin"),
                "https://huggingface.co/onnx-community/Kokoro-82M-ONNX/resolve/main/voices/bm_george.bin",
                524_288u64,
                Some("voices/bm_george.bin"),
            ),
            (
                "voices/bm_lewis.bin",
                voices_dir.join("bm_lewis.bin"),
                "https://huggingface.co/onnx-community/Kokoro-82M-ONNX/resolve/main/voices/bm_lewis.bin",
                524_288u64,
                Some("voices/bm_lewis.bin"),
            ),
        ];

        let total_files = files.len();
        for (idx, (name, target_path, url, expected_size, local_candidate)) in files.iter().enumerate() {
            let overall_base = idx as f32 / total_files as f32;
            let file_weight = 1.0 / total_files as f32;

            // Check if already present and valid
            if target_path.exists() {
                if let Ok(metadata) = fs::metadata(target_path) {
                    if metadata.len() >= *expected_size * 9 / 10 {
                        progress(overall_base + file_weight, &format!("Ready: {}", name));
                        continue;
                    }
                }
            }

            // Check if available locally in current directory or workspace
            if let Some(cand) = local_candidate {
                let local_path = Path::new(cand);
                if local_path.exists() {
                    if let Ok(meta) = fs::metadata(local_path) {
                        if meta.len() >= *expected_size * 9 / 10 {
                            progress(overall_base + file_weight * 0.5, &format!("Copying local: {}", name));
                            let _ = fs::copy(local_path, target_path);
                            progress(overall_base + file_weight, &format!("Installed: {}", name));
                            continue;
                        }
                    }
                }
            }

            // Otherwise download from HF
            progress(overall_base, &format!("Downloading: {}", name));
            match download_file_with_progress(url, target_path, |p| {
                progress(overall_base + file_weight * p, &format!("Downloading {} ({:.0}%)", name, p * 100.0));
            }) {
                Ok(_) => {}
                Err(e) => {
                    if name.starts_with("voices/") && voices_dir.join("af_bella.bin").exists() {
                        eprintln!("Optional voice download failed: {} ({})", name, e);
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        progress(1.0, "All models downloaded and verified.");
        Ok(models_dir)
    }
}

impl TtsEngine for KokoroEngine {
    fn synthesize(&self, text: &str, voice: &str, speed: f32) -> Result<Vec<f32>, String> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Text to phonemes
        let phon_str = {
            let g2p_guard = self.g2p.lock().map_err(|e| e.to_string())?;
            let (raw, _) = g2p_guard.g2p(trimmed).map_err(|e| format!("G2P error: {}", e))?;
            raw
        };
        let phonemes = normalize_phonemes(&phon_str);

        // 2. Map phonemes to token IDs with boundary tokens [0 ... 0]
        let mut tokens: Vec<i64> = Vec::with_capacity(phonemes.len() + 2);
        tokens.push(0); // BOS pad
        for c in phonemes.chars() {
            if let Some(&id) = self.vocab.get(&c) {
                tokens.push(id);
            } else {
                #[cfg(debug_assertions)]
                eprintln!("[Kokoro TTS Warning] Phoneme character not in vocabulary: {:?} (U+{:04X})", c, c as u32);
            }
        }
        tokens.push(0); // EOS pad

        // Ensure token sequence does not exceed model maximum (512 tokens; cap safely under 450)
        if tokens.len() > 450 {
            tokens.truncate(449);
            tokens.push(0);
        }

        let token_len = tokens.len();
        if token_len < 3 {
            // Nothing to speak
            return Ok(Vec::new());
        }

        // 3. Voice embedding style tensor
        let voice_key = if self.voices.contains_key(voice) {
            voice
        } else {
            "af_bella"
        };
        let voice_floats = self.voices.get(voice_key).ok_or_else(|| "Voice not found".to_string())?;

        let style_row = (token_len - 1).min(510);
        let style_slice = &voice_floats[style_row * 256..(style_row + 1) * 256];

        // 4. Create tensors
        let input_ids = ndarray::Array2::<i64>::from_shape_vec((1, token_len), tokens)
            .map_err(|e| e.to_string())?;
        let style = ndarray::Array2::<f32>::from_shape_vec((1, 256), style_slice.to_vec())
            .map_err(|e| e.to_string())?;
        let speed_val = speed.clamp(0.5, 2.0);
        let speed_tensor = ndarray::Array1::<f32>::from_vec(vec![speed_val]);

        // 5. Run inference with intra-thread limit (4)
        let mut session_guard = self.session.lock().map_err(|e| e.to_string())?;
        let outputs = session_guard
            .run(ort::inputs![
                "input_ids" => Tensor::from_array(input_ids).map_err(|e| e.to_string())?,
                "style" => Tensor::from_array(style).map_err(|e| e.to_string())?,
                "speed" => Tensor::from_array(speed_tensor).map_err(|e| e.to_string())?,
            ])
            .map_err(|e| format!("Inference error: {}", e))?;

        // 6. Extract audio PCM samples
        let (_shape, samples) = outputs["waveform"]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Extract waveform error: {}", e))?;

        let mut audio = samples.to_vec();
        // Append ~75ms of silence (1800 samples at 24kHz) to prevent truncation of the last phoneme
        // and provide a clean acoustic gap between chunks
        let silence_samples = (SAMPLE_RATE as f32 * 0.075) as usize;
        audio.resize(audio.len() + silence_samples, 0.0f32);

        Ok(audio)
    }

    fn available_voices(&self) -> Vec<String> {
        let mut v: Vec<String> = self.voices.keys().cloned().collect();
        v.sort();
        v
    }
}

/// Normalizes phonemes generated by misaki-rs:
/// - Maps misaki's decomposed affricates with zero-width joiners `d\u{200d}ʒ` and `dʒ` to single symbol `ʤ` (U+02A4, token 82).
/// - Maps `t\u{200d}ʃ` and `tʃ` to single symbol `ʧ` (U+02A7, token 133).
/// - Maps dental/alveolo-palatal variants `d\u{200d}ʑ` and `t\u{200d}ɕ` to `ʤ` and `ʧ`.
/// - Strips zero-width joiners (`\u{200d}`, `\u{200c}`) so only valid Kokoro vocab phonemes remain.
pub fn normalize_phonemes(text: &str) -> String {
    let mut s = text.to_string();
    s = s.replace("d\u{200d}ʒ", "ʤ");
    s = s.replace("t\u{200d}ʃ", "ʧ");
    s = s.replace("dʒ", "ʤ");
    s = s.replace("tʃ", "ʧ");
    s = s.replace("d\u{200d}ʑ", "ʤ");
    s = s.replace("t\u{200d}ɕ", "ʧ");
    s = s.replace("dʑ", "ʤ");
    s = s.replace("tɕ", "ʧ");
    s = s.replace('\u{200d}', "");
    s = s.replace('\u{200c}', "");
    s
}

fn load_vocab(path: &Path) -> Result<HashMap<char, i64>, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("Failed to read tokenizer: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    let vocab_obj = json
        .get("model")
        .and_then(|m| m.get("vocab"))
        .and_then(|v| v.as_object())
        .ok_or_else(|| "Invalid tokenizer.json structure".to_string())?;

    let mut map = HashMap::new();
    for (key, val) in vocab_obj {
        if let Some(id) = val.as_i64() {
            if let Some(ch) = key.chars().next() {
                map.insert(ch, id);
            }
        }
    }
    Ok(map)
}

fn load_voice_file(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = fs::read(path).map_err(|e| format!("Failed to read voice {}: {}", path.display(), e))?;
    if bytes.len() < 512 * 256 * 4 {
        return Err(format!("Voice file too small: {} bytes", bytes.len()));
    }
    let floats: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    Ok(floats)
}

fn download_file_with_progress<F>(url: &str, target: &Path, progress: F) -> Result<(), String>
where
    F: Fn(f32),
{
    let resp = ureq::get(url)
        .call()
        .map_err(|e| format!("Download request failed: {}", e))?;

    let total_len = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let mut reader = resp.into_body().into_reader();
    let temp_path = target.with_extension("downloading");
    let mut file = File::create(&temp_path).map_err(|e| format!("Create file failed: {}", e))?;

    let mut downloaded = 0u64;
    let mut buf = [0u8; 65536];

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Read stream failed: {}", e))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("Write failed: {}", e))?;
        downloaded += n as u64;

        if total_len > 0 {
            progress((downloaded as f32 / total_len as f32).min(1.0));
        }
    }

    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    fs::rename(&temp_path, target).map_err(|e| format!("Rename file failed: {}", e))?;
    progress(1.0);
    Ok(())
}
