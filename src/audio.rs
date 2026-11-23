use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, Sink};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::thread::{self, sleep};
use std::time::Duration;

pub const KOKORO_SAMPLE_RATE: u32 = 24000;

pub enum AudioCommand {
    Play(Vec<f32>),
    Pause,
    Resume,
    Stop,
}

#[derive(Clone)]
pub struct AudioHandle {
    cmd_tx: Sender<AudioCommand>,
    #[allow(dead_code)]
    is_paused: Arc<AtomicBool>,
    is_empty: Arc<AtomicBool>,
}

impl AudioHandle {
    pub fn new() -> Result<Self, String> {
        let (cmd_tx, cmd_rx) = channel::<AudioCommand>();
        let is_paused = Arc::new(AtomicBool::new(false));
        let is_empty = Arc::new(AtomicBool::new(true));

        let is_paused_thread = is_paused.clone();
        let is_empty_thread = is_empty.clone();

        // Spawn dedicated audio thread to own OutputStream and Sink
        let (init_tx, init_rx) = channel::<Result<(), String>>();

        thread::Builder::new()
            .name("koko_audio".to_string())
            .spawn(move || {
                let stream_res = OutputStream::try_default()
                    .map_err(|e| format!("Default audio device error: {}", e));

                let (_stream, stream_handle) = match stream_res {
                    Ok(pair) => {
                        let _ = init_tx.send(Ok(()));
                        pair
                    }
                    Err(e) => {
                        let _ = init_tx.send(Err(e));
                        return;
                    }
                };

                let mut sink = match Sink::try_new(&stream_handle) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Failed to create sink: {}", e);
                        return;
                    }
                };

                loop {
                    // Process all pending commands
                    while let Ok(cmd) = cmd_rx.try_recv() {
                        match cmd {
                            AudioCommand::Play(samples) => {
                                if !samples.is_empty() {
                                    let buffer = SamplesBuffer::new(1, KOKORO_SAMPLE_RATE, samples);
                                    sink.append(buffer);
                                    sink.play();
                                    is_paused_thread.store(false, Ordering::SeqCst);
                                    is_empty_thread.store(false, Ordering::SeqCst);
                                }
                            }
                            AudioCommand::Pause => {
                                sink.pause();
                                is_paused_thread.store(true, Ordering::SeqCst);
                            }
                            AudioCommand::Resume => {
                                sink.play();
                                is_paused_thread.store(false, Ordering::SeqCst);
                            }
                            AudioCommand::Stop => {
                                sink.stop();
                                if let Ok(new_sink) = Sink::try_new(&stream_handle) {
                                    sink = new_sink;
                                }
                                is_paused_thread.store(false, Ordering::SeqCst);
                                is_empty_thread.store(true, Ordering::SeqCst);
                            }
                        }
                    }

                    let empty = sink.empty();
                    is_empty_thread.store(empty, Ordering::SeqCst);

                    sleep(Duration::from_millis(15));
                }
            })
            .map_err(|e| format!("Failed to spawn audio thread: {}", e))?;

        init_rx
            .recv()
            .map_err(|e| format!("Audio thread init timeout: {}", e))??;

        Ok(Self {
            cmd_tx,
            is_paused,
            is_empty,
        })
    }

    pub fn play(&self, samples: Vec<f32>) {
        let _ = self.cmd_tx.send(AudioCommand::Play(samples));
    }

    pub fn pause(&self) {
        let _ = self.cmd_tx.send(AudioCommand::Pause);
    }

    pub fn resume(&self) {
        let _ = self.cmd_tx.send(AudioCommand::Resume);
    }

    pub fn stop(&self) {
        let _ = self.cmd_tx.send(AudioCommand::Stop);
    }

    pub fn is_empty(&self) -> bool {
        self.is_empty.load(Ordering::SeqCst)
    }

    #[allow(dead_code)]
    pub fn is_paused(&self) -> bool {
        self.is_paused.load(Ordering::SeqCst)
    }
}
