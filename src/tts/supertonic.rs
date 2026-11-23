use ndarray::{Array1, Array2, Array3};
use ort::session::Session;
use ort::value::Tensor;
use rand::Rng;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use unicode_normalization::UnicodeNormalization;
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};

pub const SUPERTONIC_SAMPLE_RATE: u32 = 44100;

#[derive(Debug, Clone, Deserialize)]
pub struct SupertonicConfig {
    pub ae: AeConfig,
    pub ttl: TtlConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AeConfig {
    pub sample_rate: i32,
    pub base_chunk_size: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TtlConfig {
    pub chunk_compress_factor: i32,
    pub latent_dim: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VoiceStyleData {
    pub style_ttl: StyleComponent,
    pub style_dp: StyleComponent,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StyleComponent {
    pub data: Vec<Vec<Vec<f32>>>,
    pub dims: Vec<usize>,
}

#[derive(Clone)]
pub struct Style {
    pub ttl: Array3<f32>,
    pub dp: Array3<f32>,
}

pub struct SupertonicEngine {
    dp_session: Mutex<Session>,
    text_enc_session: Mutex<Session>,
    vector_est_session: Mutex<Session>,
    vocoder_session: Mutex<Session>,
    config: SupertonicConfig,
    indexer: Vec<i64>,
    voice_styles: HashMap<String, Arc<Style>>,
    clean_punct_regex: Regex,
    clean_spaces_regex: Regex,
    ends_punct_regex: Regex,
}

impl SupertonicEngine {
    pub fn new(model_dir: &Path) -> Result<Self, String> {
        // Run synthesis thread with below-normal priority so system remains responsive
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }

        let onnx_dir = model_dir.join("onnx");
        let styles_dir = model_dir.join("voice_styles");

        if !onnx_dir.exists() {
            return Err(format!("Supertonic ONNX directory not found: {}", onnx_dir.display()));
        }

        // 1. Load config
        let cfg_path = onnx_dir.join("tts.json");
        let cfg_file = File::open(&cfg_path).map_err(|e| format!("Failed to open {}: {}", cfg_path.display(), e))?;
        let config: SupertonicConfig = serde_json::from_reader(BufReader::new(cfg_file))
            .map_err(|e| format!("Failed to parse tts.json: {}", e))?;

        // 2. Load Unicode indexer
        let indexer_path = onnx_dir.join("unicode_indexer.json");
        let indexer_file = File::open(&indexer_path).map_err(|e| format!("Failed to open {}: {}", indexer_path.display(), e))?;
        let indexer: Vec<i64> = serde_json::from_reader(BufReader::new(indexer_file))
            .map_err(|e| format!("Failed to parse unicode_indexer.json: {}", e))?;

        // 3. Initialize ORT sessions
        let env = super::get_or_init_ort_env()?;

        let dp_path = onnx_dir.join("duration_predictor.onnx");
        let dp_session = Session::builder(&env)
            .map_err(|e| format!("ORT builder error: {}", e))?
            .with_intra_threads(4)
            .map_err(|e| format!("ORT thread config error: {}", e))?
            .commit_from_file(&dp_path)
            .map_err(|e| format!("Failed to load duration predictor: {}", e))?;

        let text_enc_path = onnx_dir.join("text_encoder.onnx");
        let text_enc_session = Session::builder(&env)
            .map_err(|e| format!("ORT builder error: {}", e))?
            .with_intra_threads(4)
            .map_err(|e| format!("ORT thread config error: {}", e))?
            .commit_from_file(&text_enc_path)
            .map_err(|e| format!("Failed to load text encoder: {}", e))?;

        let vector_est_path = onnx_dir.join("vector_estimator.onnx");
        let vector_est_session = Session::builder(&env)
            .map_err(|e| format!("ORT builder error: {}", e))?
            .with_intra_threads(4)
            .map_err(|e| format!("ORT thread config error: {}", e))?
            .commit_from_file(&vector_est_path)
            .map_err(|e| format!("Failed to load vector estimator: {}", e))?;

        let vocoder_path = onnx_dir.join("vocoder.onnx");
        let vocoder_session = Session::builder(&env)
            .map_err(|e| format!("ORT builder error: {}", e))?
            .with_intra_threads(4)
            .map_err(|e| format!("ORT thread config error: {}", e))?
            .commit_from_file(&vocoder_path)
            .map_err(|e| format!("Failed to load vocoder: {}", e))?;

        // 4. Load Voice Styles
        let mut voice_styles = HashMap::new();
        if styles_dir.exists() {
            if let Ok(entries) = fs::read_dir(&styles_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("json") {
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            let voice_id = stem.to_uppercase();
                            if let Ok(style) = Self::load_single_voice_style(&path) {
                                voice_styles.insert(voice_id, Arc::new(style));
                            }
                        }
                    }
                }
            }
        }

        if voice_styles.is_empty() {
            return Err("No Supertonic voice styles found in voice_styles directory".to_string());
        }

        let clean_punct_regex = Regex::new(r"\s+([,.!?;:])").map_err(|e| e.to_string())?;
        let clean_spaces_regex = Regex::new(r"\s+").map_err(|e| e.to_string())?;
        let ends_punct_regex = Regex::new(r#"[.!?;:,'"\u{201C}\u{201D}\u{2018}\u{2019})\]}…。」』】〉》›»]$"#).map_err(|e| e.to_string())?;

        let engine = Self {
            dp_session: Mutex::new(dp_session),
            text_enc_session: Mutex::new(text_enc_session),
            vector_est_session: Mutex::new(vector_est_session),
            vocoder_session: Mutex::new(vocoder_session),
            config,
            indexer,
            voice_styles,
            clean_punct_regex,
            clean_spaces_regex,
            ends_punct_regex,
        };

        // Warmup inference on startup
        let _ = engine.synthesize_internal("Warmup", "F1", 1.0);

        Ok(engine)
    }

    fn load_single_voice_style(path: &Path) -> Result<Style, String> {
        let file = File::open(path).map_err(|e| format!("Cannot open {}: {}", path.display(), e))?;
        let reader = BufReader::new(file);
        let data: VoiceStyleData = serde_json::from_reader(reader).map_err(|e| format!("Parse error {}: {}", path.display(), e))?;

        let ttl_dims = &data.style_ttl.dims;
        let dp_dims = &data.style_dp.dims;

        let mut ttl_flat = Vec::new();
        for batch in &data.style_ttl.data {
            for row in batch {
                ttl_flat.extend_from_slice(row);
            }
        }

        let mut dp_flat = Vec::new();
        for batch in &data.style_dp.data {
            for row in batch {
                dp_flat.extend_from_slice(row);
            }
        }

        let ttl = Array3::from_shape_vec((1, ttl_dims[1], ttl_dims[2]), ttl_flat)
            .map_err(|e| format!("Shape error TTL: {}", e))?;
        let dp = Array3::from_shape_vec((1, dp_dims[1], dp_dims[2]), dp_flat)
            .map_err(|e| format!("Shape error DP: {}", e))?;

        Ok(Style { ttl, dp })
    }

    pub fn get_models_dir() -> PathBuf {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        let dir = PathBuf::from(appdata).join("KokoroTTS").join("supertonic");
        let _ = fs::create_dir_all(dir.join("onnx"));
        let _ = fs::create_dir_all(dir.join("voice_styles"));
        dir
    }

    pub fn download_models_if_needed<F>(progress: F) -> Result<PathBuf, String>
    where
        F: Fn(f32, &str) + Send + Sync,
    {
        let base_dir = Self::get_models_dir();
        let onnx_dir = base_dir.join("onnx");
        let styles_dir = base_dir.join("voice_styles");

        let files: Vec<(&str, PathBuf, &str, u64)> = vec![
            (
                "onnx/duration_predictor.onnx",
                onnx_dir.join("duration_predictor.onnx"),
                "https://huggingface.co/Supertone/supertonic-3/resolve/main/onnx/duration_predictor.onnx",
                3_700_000,
            ),
            (
                "onnx/text_encoder.onnx",
                onnx_dir.join("text_encoder.onnx"),
                "https://huggingface.co/Supertone/supertonic-3/resolve/main/onnx/text_encoder.onnx",
                36_000_000,
            ),
            (
                "onnx/vector_estimator.onnx",
                onnx_dir.join("vector_estimator.onnx"),
                "https://huggingface.co/Supertone/supertonic-3/resolve/main/onnx/vector_estimator.onnx",
                256_000_000,
            ),
            (
                "onnx/vocoder.onnx",
                onnx_dir.join("vocoder.onnx"),
                "https://huggingface.co/Supertone/supertonic-3/resolve/main/onnx/vocoder.onnx",
                101_000_000,
            ),
            (
                "onnx/tts.json",
                onnx_dir.join("tts.json"),
                "https://huggingface.co/Supertone/supertonic-3/resolve/main/onnx/tts.json",
                8_000,
            ),
            (
                "onnx/unicode_indexer.json",
                onnx_dir.join("unicode_indexer.json"),
                "https://huggingface.co/Supertone/supertonic-3/resolve/main/onnx/unicode_indexer.json",
                270_000,
            ),
            // Voice styles
            ("voice_styles/F1.json", styles_dir.join("F1.json"), "https://huggingface.co/Supertone/supertonic-3/resolve/main/voice_styles/F1.json", 290_000),
            ("voice_styles/F2.json", styles_dir.join("F2.json"), "https://huggingface.co/Supertone/supertonic-3/resolve/main/voice_styles/F2.json", 290_000),
            ("voice_styles/F3.json", styles_dir.join("F3.json"), "https://huggingface.co/Supertone/supertonic-3/resolve/main/voice_styles/F3.json", 290_000),
            ("voice_styles/F4.json", styles_dir.join("F4.json"), "https://huggingface.co/Supertone/supertonic-3/resolve/main/voice_styles/F4.json", 290_000),
            ("voice_styles/F5.json", styles_dir.join("F5.json"), "https://huggingface.co/Supertone/supertonic-3/resolve/main/voice_styles/F5.json", 290_000),
            ("voice_styles/M1.json", styles_dir.join("M1.json"), "https://huggingface.co/Supertone/supertonic-3/resolve/main/voice_styles/M1.json", 290_000),
            ("voice_styles/M2.json", styles_dir.join("M2.json"), "https://huggingface.co/Supertone/supertonic-3/resolve/main/voice_styles/M2.json", 290_000),
            ("voice_styles/M3.json", styles_dir.join("M3.json"), "https://huggingface.co/Supertone/supertonic-3/resolve/main/voice_styles/M3.json", 290_000),
            ("voice_styles/M4.json", styles_dir.join("M4.json"), "https://huggingface.co/Supertone/supertonic-3/resolve/main/voice_styles/M4.json", 290_000),
            ("voice_styles/M5.json", styles_dir.join("M5.json"), "https://huggingface.co/Supertone/supertonic-3/resolve/main/voice_styles/M5.json", 290_000),
        ];

        let total_files = files.len();
        for (idx, (name, target_path, url, expected_size)) in files.iter().enumerate() {
            let overall_base = idx as f32 / total_files as f32;
            let file_weight = 1.0 / total_files as f32;

            if target_path.exists() {
                if let Ok(metadata) = fs::metadata(target_path) {
                    if metadata.len() >= *expected_size * 9 / 10 {
                        progress(overall_base + file_weight, &format!("Ready: {}", name));
                        continue;
                    }
                }
            }

            progress(overall_base, &format!("Downloading: {}", name));
            download_file_with_progress(url, target_path, |p| {
                progress(overall_base + file_weight * p, &format!("Downloading {} ({:.0}%)", name, p * 100.0));
            })?;
        }

        Ok(base_dir)
    }

    fn preprocess_text(&self, text: &str, lang: &str) -> String {
        // 1. Unicode NFKD normalization
        let mut t: String = text.nfkd().collect();

        // 2. Dash and quotation replacements
        let replacements = [
            ("–", "-"),
            ("‑", "-"),
            ("—", "-"),
            ("_", " "),
            ("\u{201C}", "\""),
            ("\u{201D}", "\""),
            ("\u{2018}", "'"),
            ("\u{2019}", "'"),
            ("´", "'"),
            ("`", "'"),
            ("[", " "),
            ("]", " "),
            ("|", " "),
            ("/", " "),
            ("#", " "),
            ("→", " "),
            ("←", " "),
            ("@", " at "),
            ("e.g.,", "for example, "),
            ("i.e.,", "that is, "),
        ];

        for (from, to) in &replacements {
            t = t.replace(from, to);
        }

        // Clean punctuation spacing
        t = self.clean_punct_regex.replace_all(&t, "$1").to_string();
        t = self.clean_spaces_regex.replace_all(&t, " ").to_string();
        t = t.trim().to_string();

        if !t.is_empty() && !self.ends_punct_regex.is_match(&t) {
            t.push('.');
        }

        format!("<{}>{}</{}>", lang, t, lang)
    }

    pub fn available_voices(&self) -> Vec<String> {
        let mut voices = vec![
            "st_f1".to_string(),
            "st_f2".to_string(),
            "st_f3".to_string(),
            "st_f4".to_string(),
            "st_f5".to_string(),
            "st_m1".to_string(),
            "st_m2".to_string(),
            "st_m3".to_string(),
            "st_m4".to_string(),
            "st_m5".to_string(),
        ];
        voices.sort();
        voices
    }

    pub fn synthesize_internal(&self, text: &str, voice: &str, speed: f32) -> Result<(Vec<f32>, u32), String> {
        let clean_voice = voice
            .strip_prefix("st_")
            .unwrap_or(voice)
            .to_uppercase();

        let style = self
            .voice_styles
            .get(&clean_voice)
            .or_else(|| self.voice_styles.get("F1"))
            .ok_or_else(|| format!("Unknown voice style: {}", voice))?;

        let processed_text = self.preprocess_text(text, "en");
        if processed_text.trim().is_empty() {
            return Ok((Vec::new(), SUPERTONIC_SAMPLE_RATE));
        }

        let unicode_vals: Vec<usize> = processed_text.chars().map(|c| c as usize).collect();
        let len = unicode_vals.len();

        let mut row = vec![0i64; len];
        for (i, &val) in unicode_vals.iter().enumerate() {
            if val < self.indexer.len() {
                row[i] = self.indexer[val];
            } else {
                row[i] = -1;
            }
        }

        let text_ids = Array2::from_shape_vec((1, len), row)
            .map_err(|e| format!("Array2 shape error: {}", e))?;
        let text_mask = Array3::<f32>::ones((1, 1, len));

        // 1. Predict Duration
        let mut dp_guard = self.dp_session.lock().map_err(|e| e.to_string())?;
        let dp_out = dp_guard
            .run(ort::inputs![
                "text_ids" => Tensor::from_array(text_ids.clone()).map_err(|e| e.to_string())?,
                "style_dp" => Tensor::from_array(style.dp.clone()).map_err(|e| e.to_string())?,
                "text_mask" => Tensor::from_array(text_mask.clone()).map_err(|e| e.to_string())?,
            ])
            .map_err(|e| format!("Duration predictor error: {}", e))?;

        let (_d_shape, dur_slice) = dp_out["duration"]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Extract duration error: {}", e))?;

        let mut duration = dur_slice[0].max(0.1);
        if speed > 0.0 {
            duration /= speed;
        }

        // 2. Text Encoder
        let mut te_guard = self.text_enc_session.lock().map_err(|e| e.to_string())?;
        let te_out = te_guard
            .run(ort::inputs![
                "text_ids" => Tensor::from_array(text_ids).map_err(|e| e.to_string())?,
                "style_ttl" => Tensor::from_array(style.ttl.clone()).map_err(|e| e.to_string())?,
                "text_mask" => Tensor::from_array(text_mask.clone()).map_err(|e| e.to_string())?,
            ])
            .map_err(|e| format!("Text encoder error: {}", e))?;

        let (te_shape, te_slice) = te_out["text_emb"]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Extract text_emb error: {}", e))?;
        let te_shape_vec: Vec<usize> = te_shape.iter().map(|&x| x as usize).collect();
        let text_emb = Array3::from_shape_vec(
            (te_shape_vec[0], te_shape_vec[1], te_shape_vec[2]),
            te_slice.to_vec(),
        )
        .map_err(|e| format!("Reshape text_emb error: {}", e))?;

        // 3. Sample Noisy Latent
        let sample_rate = self.config.ae.sample_rate;
        let base_chunk_size = self.config.ae.base_chunk_size;
        let chunk_compress = self.config.ttl.chunk_compress_factor;
        let latent_dim = self.config.ttl.latent_dim;

        let chunk_size = (base_chunk_size * chunk_compress) as usize;
        let wav_len_max = (duration * sample_rate as f32) as usize;
        let latent_len = (wav_len_max + chunk_size - 1) / chunk_size;
        let latent_dim_val = (latent_dim * chunk_compress) as usize;

        let mut noisy_latent = Array3::<f32>::zeros((1, latent_dim_val, latent_len));
        let mut rng = rand::thread_rng();

        for d in 0..latent_dim_val {
            for t in 0..latent_len {
                // Box-Muller transform for N(0, 1) standard normal distribution
                let u1: f32 = rng.gen::<f32>().max(1e-7);
                let u2: f32 = rng.gen::<f32>();
                let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos();
                noisy_latent[[0, d, t]] = z;
            }
        }

        let latent_mask = Array3::<f32>::ones((1, 1, latent_len));

        // 4. Vector Estimator (Flow-Matching Denoising)
        // 6 steps produces crisp voice synthesis with sub-second CPU latency
        let total_step = 6usize;
        let total_step_arr = Array1::from_elem(1, total_step as f32);
        let mut cur_xt = noisy_latent;

        let mut ve_guard = self.vector_est_session.lock().map_err(|e| e.to_string())?;
        for step in 0..total_step {
            let step_arr = Array1::from_elem(1, step as f32);
            let ve_out = ve_guard
                .run(ort::inputs![
                    "noisy_latent" => Tensor::from_array(cur_xt).map_err(|e| e.to_string())?,
                    "text_emb" => Tensor::from_array(text_emb.clone()).map_err(|e| e.to_string())?,
                    "style_ttl" => Tensor::from_array(style.ttl.clone()).map_err(|e| e.to_string())?,
                    "latent_mask" => Tensor::from_array(latent_mask.clone()).map_err(|e| e.to_string())?,
                    "text_mask" => Tensor::from_array(text_mask.clone()).map_err(|e| e.to_string())?,
                    "current_step" => Tensor::from_array(step_arr).map_err(|e| e.to_string())?,
                    "total_step" => Tensor::from_array(total_step_arr.clone()).map_err(|e| e.to_string())?,
                ])
                .map_err(|e| format!("Vector estimator error at step {}: {}", step, e))?;

            let (dl_shape, dl_slice) = ve_out["denoised_latent"]
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("Extract denoised_latent error: {}", e))?;
            let dl_shape_vec: Vec<usize> = dl_shape.iter().map(|&x| x as usize).collect();
            cur_xt = Array3::from_shape_vec(
                (dl_shape_vec[0], dl_shape_vec[1], dl_shape_vec[2]),
                dl_slice.to_vec(),
            )
            .map_err(|e| format!("Reshape denoised_latent error: {}", e))?;
        }

        // 5. Vocoder
        let mut voc_guard = self.vocoder_session.lock().map_err(|e| e.to_string())?;
        let voc_out = voc_guard
            .run(ort::inputs![
                "latent" => Tensor::from_array(cur_xt).map_err(|e| e.to_string())?,
            ])
            .map_err(|e| format!("Vocoder error: {}", e))?;

        let (_wav_shape, wav_slice) = voc_out["wav_tts"]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Extract wav_tts error: {}", e))?;

        let wav_len = (duration * sample_rate as f32) as usize;
        let mut audio = wav_slice[..wav_len.min(wav_slice.len())].to_vec();

        // Append ~75ms silence gap to prevent clipping the final word
        let silence_samples = (sample_rate as f32 * 0.075) as usize;
        audio.resize(audio.len() + silence_samples, 0.0f32);

        Ok((audio, sample_rate as u32))
    }
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

    drop(file);
    let _ = fs::remove_file(target);
    fs::rename(&temp_path, target).map_err(|e| format!("Rename failed: {}", e))?;
    Ok(())
}
