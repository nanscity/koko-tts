use crate::audio::AudioHandle;
use crate::tts::TtsEngine;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, sleep};
use std::time::Duration;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayStatus {
    Idle,
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackState {
    pub status: PlayStatus,
    pub lines: Vec<String>,
    pub current_index: usize,
    pub voice: String,
    pub speed: f32,
}

#[derive(Debug, Clone, Serialize)]
pub enum PlaybackEvent {
    StateChanged(PlaybackState),
    LineChanged(usize),
    Notice(String),
}

pub struct PlaybackCoordinator {
    engine: Arc<dyn TtsEngine>,
    state: Arc<parking_lot::Mutex<PlaybackState>>,
    audio_cache: Arc<parking_lot::Mutex<Vec<Option<(Vec<f32>, u32)>>>>,
    playing_chunk: Arc<parking_lot::Mutex<Option<usize>>>,
    epoch: Arc<AtomicU64>,
    event_tx: broadcast::Sender<PlaybackEvent>,
    audio: AudioHandle,
}

impl PlaybackCoordinator {
    pub fn new(engine: Arc<dyn TtsEngine>) -> Result<Arc<Self>, String> {
        let audio = AudioHandle::new()?;
        let (event_tx, _) = broadcast::channel(128);

        let initial_state = PlaybackState {
            status: PlayStatus::Idle,
            lines: Vec::new(),
            current_index: 0,
            voice: "af_bella".to_string(),
            speed: 1.0,
        };

        let coordinator = Arc::new(Self {
            engine,
            state: Arc::new(parking_lot::Mutex::new(initial_state)),
            audio_cache: Arc::new(parking_lot::Mutex::new(Vec::new())),
            playing_chunk: Arc::new(parking_lot::Mutex::new(None)),
            epoch: Arc::new(AtomicU64::new(0)),
            event_tx,
            audio,
        });

        // Spawn playback controller loop
        let coord_weak = Arc::downgrade(&coordinator);
        thread::Builder::new()
            .name("koko_coord".to_string())
            .spawn(move || {
                while let Some(coord) = coord_weak.upgrade() {
                    coord.playback_loop_step();
                    sleep(Duration::from_millis(20));
                }
            })
            .map_err(|e| format!("Failed to spawn coordinator thread: {}", e))?;

        Ok(coordinator)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PlaybackEvent> {
        self.event_tx.subscribe()
    }

    pub fn get_state(&self) -> PlaybackState {
        self.state.lock().clone()
    }

    pub fn available_voices(&self) -> Vec<String> {
        self.engine.available_voices()
    }

    pub fn play_text(&self, text: &str, voice: &str, speed: f32) {
        let lines = split_into_chunks(text);
        if lines.is_empty() {
            return;
        }

        let new_epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.audio.stop();
        *self.playing_chunk.lock() = None;

        {
            let mut state = self.state.lock();
            state.status = PlayStatus::Playing;
            state.lines = lines.clone();
            state.current_index = 0;
            state.voice = voice.to_string();
            state.speed = speed;
        }

        {
            let mut cache = self.audio_cache.lock();
            *cache = vec![None; lines.len()];
        }

        self.broadcast_state();

        // Spawn synthesis lookahead worker for this epoch
        self.trigger_lookahead_synthesis(new_epoch, 0);
    }

    pub fn pause(&self) {
        let mut state = self.state.lock();
        if state.status == PlayStatus::Playing {
            state.status = PlayStatus::Paused;
            self.audio.pause();
            drop(state);
            self.broadcast_state();
        }
    }

    pub fn resume(&self) {
        let mut state = self.state.lock();
        if state.status == PlayStatus::Paused {
            state.status = PlayStatus::Playing;
            self.audio.resume();
            drop(state);
            self.broadcast_state();
        }
    }

    pub fn stop(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.audio.stop();
        *self.playing_chunk.lock() = None;

        let mut state = self.state.lock();
        state.status = PlayStatus::Stopped;
        drop(state);
        self.broadcast_state();
    }

    pub fn next_line(&self) {
        let (current, total) = {
            let state = self.state.lock();
            (state.current_index, state.lines.len())
        };
        if current + 1 < total {
            self.jump_to_line(current + 1);
        }
    }

    pub fn prev_line(&self) {
        let current = self.state.lock().current_index;
        if current > 0 {
            self.jump_to_line(current - 1);
        }
    }

    pub fn forward_10s(&self) {
        let (current, total) = {
            let state = self.state.lock();
            (state.current_index, state.lines.len())
        };
        if total == 0 {
            return;
        }
        let target = (current + 2).min(total - 1);
        self.jump_to_line(target);
    }

    pub fn backward_10s(&self) {
        let current = self.state.lock().current_index;
        let target = if current > 2 { current - 2 } else { 0 };
        self.jump_to_line(target);
    }

    pub fn jump_to_line(&self, target_index: usize) {
        let total = self.state.lock().lines.len();
        if target_index >= total {
            return;
        }

        let new_epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.audio.stop();
        *self.playing_chunk.lock() = None;

        {
            let mut state = self.state.lock();
            state.current_index = target_index;
            state.status = PlayStatus::Playing;
        }

        self.broadcast_line_change(target_index);
        self.broadcast_state();

        // Trigger lookahead from target index
        self.trigger_lookahead_synthesis(new_epoch, target_index);
    }

    fn trigger_lookahead_synthesis(&self, epoch: u64, start_index: usize) {
        let engine = self.engine.clone();
        let state_arc = self.state.clone();
        let cache_arc = self.audio_cache.clone();
        let epoch_arc = self.epoch.clone();
        let event_tx = self.event_tx.clone();

        thread::Builder::new()
            .name("koko_synth".to_string())
            .spawn(move || {
                unsafe {
                    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
                    let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
                }

                let (lines, voice, speed) = {
                    let state = state_arc.lock();
                    (state.lines.clone(), state.voice.clone(), state.speed)
                };

                let total = lines.len();
                // Synthesize target index first, then lookahead 2-3 chunks
                for idx in start_index..total {
                    if epoch_arc.load(Ordering::SeqCst) != epoch {
                        // Epoch changed (stopped or jumped), abort synthesis immediately
                        return;
                    }

                    // Check if current playback has drifted too far ahead
                    let current = state_arc.lock().current_index;
                    if idx > current + 3 {
                        // Only stay 2-3 chunks ahead
                        while state_arc.lock().current_index + 3 < idx {
                            if epoch_arc.load(Ordering::SeqCst) != epoch {
                                return;
                            }
                            sleep(Duration::from_millis(50));
                        }
                    }

                    // Check if already synthesized
                    let is_cached = {
                        let cache = cache_arc.lock();
                        cache.get(idx).and_then(|opt| opt.as_ref()).is_some()
                    };

                    if !is_cached {
                        let line_text = lines[idx].clone();
                        let synth_engine = engine.clone();
                        let synth_voice = voice.clone();

                        // Wrap synthesis in catch_unwind to guarantee a panic never kills the synthesis thread
                        let synth_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                            synth_engine.synthesize(&line_text, &synth_voice, speed)
                        }));

                        let audio_result = match synth_res {
                            Ok(Ok((samples, sample_rate))) if !samples.is_empty() => (samples, sample_rate),
                            Ok(Ok((_empty, sample_rate))) => {
                                // Empty audio returned, supply minimal silence (100ms) so playback moves on
                                let sil_len = (sample_rate as f32 * 0.1) as usize;
                                (vec![0.0f32; sil_len], sample_rate)
                            }
                            Ok(Err(err)) => {
                                eprintln!("[TTS Error] Chunk {}: {}", idx + 1, err);
                                let _ = event_tx.send(PlaybackEvent::Notice(format!(
                                    "Notice: chunk {} failed ({}), skipping",
                                    idx + 1,
                                    err
                                )));
                                // Insert 100ms silence so playback loop doesn't stall
                                (vec![0.0f32; 2400], 24000)
                            }
                            Err(panic_payload) => {
                                eprintln!("[TTS Panic] Chunk {}: {:?}", idx + 1, panic_payload);
                                let _ = event_tx.send(PlaybackEvent::Notice(format!(
                                    "Notice: chunk {} panic during inference, skipping",
                                    idx + 1
                                )));
                                // Insert 100ms silence so playback doesn't stall
                                (vec![0.0f32; 2400], 24000)
                            }
                        };

                        if epoch_arc.load(Ordering::SeqCst) == epoch {
                            let mut cache = cache_arc.lock();
                            if idx < cache.len() {
                                cache[idx] = Some(audio_result);
                            }
                        }
                    }
                }
            })
            .expect("Failed to spawn synthesis thread");
    }

    fn playback_loop_step(&self) {
        let (status, current_idx, total) = {
            let state = self.state.lock();
            (state.status, state.current_index, state.lines.len())
        };

        if status != PlayStatus::Playing || total == 0 || current_idx >= total {
            return;
        }

        if self.audio.is_empty() {
            let mut playing_guard = self.playing_chunk.lock();
            if let Some(finished_idx) = *playing_guard {
                // The chunk that was playing has finished!
                *playing_guard = None;
                drop(playing_guard);
                self.advance_to_next(finished_idx, total);
                return;
            }

            // No chunk actively playing: check if audio for current_index is ready in cache
            let maybe_audio = {
                let cache = self.audio_cache.lock();
                cache.get(current_idx).and_then(|opt| opt.clone())
            };

            if let Some((audio, sample_rate)) = maybe_audio {
                if !audio.is_empty() {
                    *playing_guard = Some(current_idx);
                    drop(playing_guard);
                    self.audio.play(audio, sample_rate);
                } else {
                    // Empty chunk, advance immediately
                    drop(playing_guard);
                    self.advance_to_next(current_idx, total);
                }
            }
        }
    }

    fn advance_to_next(&self, current_idx: usize, total: usize) {
        if current_idx + 1 < total {
            let next_idx = current_idx + 1;
            {
                let mut state = self.state.lock();
                state.current_index = next_idx;
            }
            self.broadcast_line_change(next_idx);
            self.broadcast_state();
        } else {
            // Reached the end
            let mut state = self.state.lock();
            state.status = PlayStatus::Stopped;
            drop(state);
            self.audio.stop();
            self.broadcast_state();
        }
    }

    fn broadcast_state(&self) {
        let state = self.state.lock().clone();
        let _ = self.event_tx.send(PlaybackEvent::StateChanged(state));
    }

    fn broadcast_line_change(&self, line: usize) {
        let _ = self.event_tx.send(PlaybackEvent::LineChanged(line));
    }
}

pub fn split_into_chunks(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return chunks;
    }

    // Split paragraphs
    for para in trimmed.lines() {
        let p = para.trim();
        if p.is_empty() {
            continue;
        }

        // Split sentences by punctuation (.!?;)
        let mut start = 0;
        let chars: Vec<(usize, char)> = p.char_indices().collect();
        let len = chars.len();

        for i in 0..len {
            let (_, c) = chars[i];
            let is_boundary = match c {
                '.' | '!' | '?' | ';' | ':' => {
                    if i + 1 < len {
                        chars[i + 1].1.is_whitespace()
                    } else {
                        true
                    }
                }
                _ => false,
            };

            if is_boundary {
                let end = if i + 1 < len { chars[i + 1].0 } else { p.len() };
                let sentence = p[start..end].trim();
                if !sentence.is_empty() {
                    for piece in split_long_sentence(sentence, 200) {
                        if !piece.is_empty() {
                            chunks.push(piece);
                        }
                    }
                }
                start = end;
            }
        }

        if start < p.len() {
            let remainder = p[start..].trim();
            if !remainder.is_empty() {
                for piece in split_long_sentence(remainder, 200) {
                    if !piece.is_empty() {
                        chunks.push(piece);
                    }
                }
            }
        }
    }

    // Optimization: make first chunk short (< ~100 characters) for instant audio startup
    if !chunks.is_empty() && chunks[0].len() > 100 {
        let first = &chunks[0];
        if let Some(pos) = first[..100].rfind(|c| c == ',' || c == '—' || c == '-') {
            if pos > 20 {
                let chunk1 = first[..=pos].trim().to_string();
                let chunk2 = first[pos + 1..].trim().to_string();
                chunks.remove(0);
                chunks.insert(0, chunk2);
                chunks.insert(0, chunk1);
            }
        }
    }

    chunks
}

/// Splits long sentences into natural clauses (< max_len characters, well within Kokoro token limits)
/// using clause markers (;, :, ,, —, -) or whitespace word boundaries.
fn split_long_sentence(sentence: &str, max_len: usize) -> Vec<String> {
    if sentence.len() <= max_len {
        return vec![sentence.to_string()];
    }

    let mut result = Vec::new();
    let mut current = sentence;

    while current.len() > max_len {
        let slice = &current[..max_len];
        let split_pos = slice
            .rfind(|c| c == ';' || c == ':' || c == ',' || c == '—' || c == '-')
            .filter(|&pos| pos > 25)
            .or_else(|| slice.rfind(char::is_whitespace).filter(|&pos| pos > 25));

        if let Some(pos) = split_pos {
            let part = current[..=pos].trim();
            if !part.is_empty() {
                result.push(part.to_string());
            }
            current = current[pos + 1..].trim();
        } else {
            let part = current[..max_len].trim();
            if !part.is_empty() {
                result.push(part.to_string());
            }
            current = current[max_len..].trim();
        }
    }

    if !current.is_empty() {
        result.push(current.to_string());
    }

    result
}
