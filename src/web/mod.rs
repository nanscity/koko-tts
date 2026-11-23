use crate::autostart::set_autostart;
use crate::playback::{PlaybackCoordinator, PlaybackEvent};
use crate::settings::{AppSettings, ClipboardMode};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Json};
use axum::routing::{get, post};
use axum::Router;
use futures_util::StreamExt;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;

const INDEX_HTML: &str = include_str!("index.html");

#[derive(Clone)]
pub struct WebState {
    pub coordinator: Arc<parking_lot::Mutex<Option<Arc<PlaybackCoordinator>>>>,
    pub settings: Arc<parking_lot::Mutex<AppSettings>>,
    pub hotkey_updater: Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>,
}

#[derive(Deserialize)]
pub struct PlayRequest {
    pub text: String,
    pub voice: Option<String>,
    pub speed: Option<f32>,
}

#[derive(Deserialize)]
pub struct SettingsUpdateRequest {
    pub hotkey: Option<String>,
    pub clipboard_mode: Option<ClipboardMode>,
    pub dont_restore_clipboard: Option<bool>,
    pub start_with_windows: Option<bool>,
    pub voice: Option<String>,
    pub speed: Option<f32>,
    pub online_api_key: Option<String>,
    pub online_endpoint: Option<String>,
}

pub async fn start_web_server(state: WebState, port: u16) -> Result<(), String> {
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/ws", get(ws_handler))
        .route("/api/play", post(play_handler))
        .route("/api/pause", post(pause_handler))
        .route("/api/resume", post(resume_handler))
        .route("/api/stop", post(stop_handler))
        .route("/api/next", post(next_handler))
        .route("/api/prev", post(prev_handler))
        .route("/api/forward", post(forward_handler))
        .route("/api/rewind", post(rewind_handler))
        .route("/api/jump/:idx", post(jump_handler))
        .route("/api/settings", get(get_settings).post(update_settings))
        .route("/api/voices", get(get_voices))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("Failed to bind to {}: {}", addr, e))?;

    axum::serve(listener, app)
        .await
        .map_err(|e| format!("Axum serve error: {}", e))
}

async fn index_handler() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<WebState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: WebState) {
    loop {
        let (rx_opt, initial_state_opt) = {
            let guard = state.coordinator.lock();
            if let Some(ref c) = *guard {
                (Some(c.subscribe()), Some(c.get_state()))
            } else {
                (None, None)
            }
        };

        if let (Some(mut rx), Some(initial_state)) = (rx_opt, initial_state_opt) {
            if let Ok(json) = serde_json::to_string(&PlaybackEvent::StateChanged(initial_state)) {
                let _ = socket.send(Message::Text(json)).await;
            }

            loop {
                tokio::select! {
                    event_res = rx.recv() => {
                        match event_res {
                            Ok(event) => {
                                if let Ok(json) = serde_json::to_string(&event) {
                                    if socket.send(Message::Text(json)).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                        }
                    }
                    client_msg = socket.next() => {
                        match client_msg {
                            Some(Ok(Message::Close(_))) | None => return,
                            _ => {}
                        }
                    }
                }
            }
        } else {
            tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
        }
    }
}

async fn play_handler(State(state): State<WebState>, Json(payload): Json<PlayRequest>) -> Json<serde_json::Value> {
    let coord = state.coordinator.lock().clone();
    if let Some(coord) = coord {
        let voice = payload.voice.unwrap_or_else(|| state.settings.lock().voice.clone());
        let speed = payload.speed.unwrap_or_else(|| state.settings.lock().speed);

        coord.play_text(&payload.text, &voice, speed);
        Json(serde_json::json!({ "success": true }))
    } else {
        Json(serde_json::json!({ "success": false, "error": "Model engine is still initializing..." }))
    }
}

async fn pause_handler(State(state): State<WebState>) -> Json<serde_json::Value> {
    if let Some(coord) = state.coordinator.lock().clone() {
        coord.pause();
    }
    Json(serde_json::json!({ "success": true }))
}

async fn resume_handler(State(state): State<WebState>) -> Json<serde_json::Value> {
    if let Some(coord) = state.coordinator.lock().clone() {
        coord.resume();
    }
    Json(serde_json::json!({ "success": true }))
}

async fn stop_handler(State(state): State<WebState>) -> Json<serde_json::Value> {
    if let Some(coord) = state.coordinator.lock().clone() {
        coord.stop();
    }
    Json(serde_json::json!({ "success": true }))
}

async fn next_handler(State(state): State<WebState>) -> Json<serde_json::Value> {
    if let Some(coord) = state.coordinator.lock().clone() {
        coord.next_line();
    }
    Json(serde_json::json!({ "success": true }))
}

async fn prev_handler(State(state): State<WebState>) -> Json<serde_json::Value> {
    if let Some(coord) = state.coordinator.lock().clone() {
        coord.prev_line();
    }
    Json(serde_json::json!({ "success": true }))
}

async fn forward_handler(State(state): State<WebState>) -> Json<serde_json::Value> {
    if let Some(coord) = state.coordinator.lock().clone() {
        coord.forward_10s();
    }
    Json(serde_json::json!({ "success": true }))
}

async fn rewind_handler(State(state): State<WebState>) -> Json<serde_json::Value> {
    if let Some(coord) = state.coordinator.lock().clone() {
        coord.backward_10s();
    }
    Json(serde_json::json!({ "success": true }))
}

async fn jump_handler(State(state): State<WebState>, Path(idx): Path<usize>) -> Json<serde_json::Value> {
    if let Some(coord) = state.coordinator.lock().clone() {
        coord.jump_to_line(idx);
    }
    Json(serde_json::json!({ "success": true }))
}

async fn get_settings(State(state): State<WebState>) -> Json<AppSettings> {
    let settings = state.settings.lock().clone();
    Json(settings)
}

async fn update_settings(
    State(state): State<WebState>,
    Json(req): Json<SettingsUpdateRequest>,
) -> Json<serde_json::Value> {
    let mut s = state.settings.lock();

    if let Some(new_hotkey) = req.hotkey {
        if new_hotkey != s.hotkey {
            if let Err(e) = (state.hotkey_updater)(&new_hotkey) {
                return Json(serde_json::json!({ "success": false, "error": e }));
            }
            s.hotkey = new_hotkey;
        }
    }

    if let Some(mode) = req.clipboard_mode {
        s.clipboard_mode = mode;
    }
    if let Some(dont_restore) = req.dont_restore_clipboard {
        s.dont_restore_clipboard = dont_restore;
    }
    if let Some(autostart) = req.start_with_windows {
        s.start_with_windows = autostart;
        let _ = set_autostart(autostart);
    }
    if let Some(voice) = req.voice {
        s.voice = voice;
    }
    if let Some(speed) = req.speed {
        s.speed = speed;
    }
    if let Some(api_key) = req.online_api_key {
        s.online_api_key = api_key;
    }
    if let Some(endpoint) = req.online_endpoint {
        s.online_endpoint = endpoint;
    }

    let _ = s.save();
    Json(serde_json::json!({ "success": true }))
}

async fn get_voices(State(state): State<WebState>) -> Json<Vec<String>> {
    let voices = if let Some(coord) = state.coordinator.lock().as_ref() {
        coord.available_voices()
    } else {
        vec![
            "af_bella".to_string(),
            "af_nicole".to_string(),
            "af_sarah".to_string(),
            "af_sky".to_string(),
            "am_adam".to_string(),
            "am_michael".to_string(),
            "bf_emma".to_string(),
            "bf_isabella".to_string(),
            "bm_george".to_string(),
            "bm_lewis".to_string(),
        ]
    };
    Json(voices)
}
