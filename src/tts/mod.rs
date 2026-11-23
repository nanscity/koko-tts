pub mod kokoro;

pub trait TtsEngine: Send + Sync {
    fn synthesize(&self, text: &str, voice: &str, speed: f32) -> Result<Vec<f32>, String>;
    #[allow(dead_code)]
    fn available_voices(&self) -> Vec<String>;
}
