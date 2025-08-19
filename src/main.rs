use axum::{
    extract::Extension,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use rumqttc::{AsyncClient, MqttOptions, QoS};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

use axum::http::Method;
use tower_http::cors::{Any, CorsLayer};
#[derive(Debug, Deserialize, Serialize)]
enum Direction {
    Up,
    Down,
}

impl std::fmt::Display for Direction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Direction::Up => write!(f, "Up"),
            Direction::Down => write!(f, "Down"),
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub enum SvenCommand {
    UpDuration,     // value: ms
    DownDuration,   // value: ms
    UpRelative,     // value: mm
    DownRelative,   // value: mm
    AbsoluteHeight, // value: mm
    Position,       // value: SvenPosition
}

// Just for printing purposes
impl std::fmt::Display for SvenCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SvenCommand::UpDuration => write!(f, "Up Duration"),
            SvenCommand::DownDuration => write!(f, "Down Duration"),
            SvenCommand::UpRelative => write!(f, "Up Relative"),
            SvenCommand::DownRelative => write!(f, "Down Relative"),
            SvenCommand::AbsoluteHeight => write!(f, "Absolute Height"),
            SvenCommand::Position => write!(f, "Position"),
        }
    }
}
#[derive(Debug, Deserialize, Serialize)]
pub struct DeskCommand {
    pub command: SvenCommand,
    pub value: u32,
}

// Shared state for MQTT client
struct AppState {
    mqtt_client: Arc<Mutex<AsyncClient>>,
    sven_state: Arc<Mutex<SvenState>>,
}

async fn handle_command(
    Json(command): Json<DeskCommand>,
    state: Extension<Arc<AppState>>,
) -> impl IntoResponse {
    println!("Moving Sven {} for {} ms", command.command, command.value);

    // Serialize the command as JSON for MQTT payload
    let payload = serde_json::to_string(&command).unwrap();

    // Publish to MQTT broker
    let client = state.mqtt_client.clone();
    let topic = "sven/command";
    let _ = client
        .lock()
        .await
        .publish(topic, QoS::AtLeastOnce, false, payload)
        .await;

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "Command sent successfully"})),
    )
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
pub enum SvenPosition {
    Bottom,
    Top,
    Armrest,
    AboveArmrest,
    Standing,
    Custom,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
pub struct SvenState {
    height_mm: u32,
    position: SvenPosition,
}

async fn get_sven_state(Extension(app_state): Extension<Arc<AppState>>) -> impl IntoResponse {
    let sven_state = app_state.sven_state.lock().await;
    (StatusCode::OK, Json(*sven_state))
}

#[tokio::main]
async fn main() {
    // MQTT client setup
    let mut mqttoptions = MqttOptions::new("sven-client", "localhost", 1883);
    mqttoptions.set_keep_alive(std::time::Duration::from_secs(5));

    let (mqtt_client, mut eventloop) = AsyncClient::new(mqttoptions, 10);
    mqtt_client
        .subscribe("sven/state", QoS::AtLeastOnce)
        .await
        .unwrap();
    let app_state = Arc::new(AppState {
        mqtt_client: Arc::new(Mutex::new(mqtt_client)),
        sven_state: Arc::new(Mutex::new(SvenState {
            height_mm: 0,
            position: SvenPosition::Custom,
        })),
    });

    let mqtt_app_state = app_state.clone();

    // Spawn a task to poll the MQTT event loop
    let eventloop_handle = tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(publish))) => {
                    println!(
                        "Received MQTT packet: {}: {:?}",
                        publish.topic, publish.payload
                    );
                    match publish.topic.as_str() {
                        "sven/state" => {
                            // Deserialize the payload into SvenState
                            if let Ok(state) = serde_json::from_slice::<SvenState>(&publish.payload) {
                                let mut sven_state = mqtt_app_state.sven_state.lock().await;
                                *sven_state = state;
                                println!("Updated Sven state: {:?}", *sven_state);
                            } else {
                                eprintln!("Failed to deserialize Sven state");
                            }
                        }
                        _ => {
                            eprintln!("Unknown topic: {}", publish.topic);
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("MQTT error: {:?}", e);
                }
            }
        }
    });

    // Set up CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    let app = Router::new()
        .route(
            "/api/sven/command",
            post({
                let shared_state = app_state.clone();
                move |body| {
                    println!("Received command: {:?}", body);
                    handle_command(body, Extension(shared_state))
                }
            }),
        )
        .route("/api/sven/state", get(get_sven_state))
        .layer(Extension(app_state))
        .layer(cors);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3001").await.unwrap();
    axum::serve(listener, app).await.unwrap();

    let _ = eventloop_handle.await;
}
