pub mod kokoro;
pub mod supertonic;

use kokoro::KokoroEngine;
use supertonic::SupertonicEngine;
use std::sync::{Arc, OnceLock};

static GLOBAL_ORT_ENV: OnceLock<Arc<ort::environment::Environment>> = OnceLock::new();

pub fn get_or_init_ort_env() -> Result<Arc<ort::environment::Environment>, String> {
    if let Some(env) = GLOBAL_ORT_ENV.get() {
        return Ok(env.clone());
    }

    let env = Arc::new(
        ort::init()
            .with_name("KokoroSupertonicTTS")
            .build()
            .map_err(|e| format!("ORT init error: {}", e))?,
    );

    let _ = GLOBAL_ORT_ENV.set(env.clone());
    Ok(env)
}

pub trait TtsEngine: Send + Sync {
    fn synthesize(&self, text: &str, voice: &str, speed: f32) -> Result<(Vec<f32>, u32), String>;
    #[allow(dead_code)]
    fn available_voices(&self) -> Vec<String>;
}

impl TtsEngine for SupertonicEngine {
    fn synthesize(&self, text: &str, voice: &str, speed: f32) -> Result<(Vec<f32>, u32), String> {
        self.synthesize_internal(text, voice, speed)
    }

    fn available_voices(&self) -> Vec<String> {
        self.available_voices()
    }
}

pub struct CompositeEngine {
    kokoro: Arc<KokoroEngine>,
    supertonic: Arc<SupertonicEngine>,
}

impl CompositeEngine {
    pub fn new(kokoro: Arc<KokoroEngine>, supertonic: Arc<SupertonicEngine>) -> Self {
        Self { kokoro, supertonic }
    }

    pub fn is_supertonic(voice: &str) -> bool {
        let v = voice.to_ascii_lowercase();
        v.starts_with("st_")
            || v.starts_with("st:")
            || matches!(
                v.as_str(),
                "f1" | "f2" | "f3" | "f4" | "f5" | "m1" | "m2" | "m3" | "m4" | "m5"
            )
    }
}

impl TtsEngine for CompositeEngine {
    fn synthesize(&self, text: &str, voice: &str, speed: f32) -> Result<(Vec<f32>, u32), String> {
        if Self::is_supertonic(voice) {
            self.supertonic.synthesize(text, voice, speed)
        } else {
            self.kokoro.synthesize(text, voice, speed)
        }
    }

    fn available_voices(&self) -> Vec<String> {
        let mut voices = self.kokoro.available_voices();
        voices.extend(self.supertonic.available_voices());
        voices
    }
}
