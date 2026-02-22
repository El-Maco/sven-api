use axum::{
    Json, Router,
    extract::Extension,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{self, Timelike};
use rumqttc::{AsyncClient, Event as MqttEvent, MqttOptions, Outgoing, Packet, QoS};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

use axum::http::Method;
use tower_http::cors::{Any, CorsLayer};

pub const SVEN_COMMAND_TOPIC: &str = "sven/command";
pub const SVEN_STATE_TOPIC: &str = "sven/state";
pub const SVEN_STATUS_TOPIC: &str = "sven/status";

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
    sven_status: Arc<Mutex<String>>,
}

async fn handle_command(
    Json(command): Json<DeskCommand>,
    state: Extension<Arc<AppState>>,
) -> impl IntoResponse {
    println!(
        "Received command {} with value {}",
        command.command, command.value
    );

    // Serialize the command as JSON for MQTT payload
    let payload = serde_json::to_string(&command).unwrap();

    // Publish to MQTT broker
    let client = state.mqtt_client.clone();
    let _ = client
        .lock()
        .await
        .publish(SVEN_COMMAND_TOPIC, QoS::AtLeastOnce, false, payload)
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

async fn get_sven_status(Extension(app_state): Extension<Arc<AppState>>) -> impl IntoResponse {
    let sven_status = app_state.sven_status.lock().await;
    println!("Returning Sven status: {}", *sven_status);
    (StatusCode::OK, Json(sven_status.clone()))
}
async fn set_to_night_mode(Extension(app_state): Extension<Arc<AppState>>) {
    let client = app_state.mqtt_client.clone();
    let _ = client
        .lock()
        .await
        .publish(
            SVEN_COMMAND_TOPIC,
            QoS::AtLeastOnce,
            false,
            serde_json::to_string(&DeskCommand {
                command: SvenCommand::AbsoluteHeight,
                value: 850,
            })
            .unwrap(),
        )
        .await;
}

static HOST_IP: &str = "192.168.1.132";

async fn host_is_active() -> bool {
    match tokio::process::Command::new("ping")
        .arg("-c")
        .arg("1")
        .arg(HOST_IP)
        .output()
        .await
    {
        Ok(output) => output.status.success(),
        Err(e) => {
            eprintln!("Failed to execute ping command: {:?}", e);
            false
        }
    }
}

#[tokio::main]
async fn main() {
    // MQTT client setup
    let mut mqtt_options = MqttOptions::new("sven-client", "localhost", 1883);
    mqtt_options.set_keep_alive(std::time::Duration::from_secs(5));

    let (mqtt_client, mut eventloop) = AsyncClient::new(mqtt_options, 10);
    mqtt_client
        .subscribe("sven/#", QoS::AtLeastOnce)
        .await
        .unwrap();
    let app_state = Arc::new(AppState {
        mqtt_client: Arc::new(Mutex::new(mqtt_client)),
        sven_state: Arc::new(Mutex::new(SvenState {
            height_mm: 0,
            position: SvenPosition::Custom,
        })),
        sven_status: Arc::new(Mutex::new("offline".to_string())),
    });

    let mqtt_app_state = app_state.clone();
    let night_mode_app_state = app_state.clone();
    let _ = tokio::spawn(async move {
        loop {
            let sven_state = {
                let sven_state = night_mode_app_state.sven_state.lock().await;
                *sven_state
            };
            static NIGHT_TIME_THRESHOLD_MM: u32 = 845;

            if sven_state.height_mm >= NIGHT_TIME_THRESHOLD_MM {
                println!(
                    "Current height is {}... Already in night mode!",
                    sven_state.height_mm
                );
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                continue;
            }

            static NIGHT_TIME_START: u32 = 23;
            static NIGHT_TIME_END: u32 = 6;

            let now = chrono::Local::now();
            if now.hour() < NIGHT_TIME_START && now.hour() >= NIGHT_TIME_END {
                println!("Not night time, skipping night mode check");
                let wait_time = now
                    .with_hour(NIGHT_TIME_START)
                    .unwrap()
                    .with_minute(0)
                    .unwrap()
                    .with_second(0)
                    .unwrap()
                    - now;
                println!(
                    "Waiting until night time starts in {} seconds",
                    wait_time.num_seconds()
                );
                tokio::time::sleep(std::time::Duration::from_secs(
                    wait_time.num_seconds() as u64
                ))
                .await;
                continue;
            }

            if host_is_active().await {
                println!("Host is still active, will not set to night mode");
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                continue;
            }

            println!(
                "It's night time and current height is {}, setting desk to night mode",
                sven_state.height_mm
            );
            set_to_night_mode(Extension(night_mode_app_state.clone())).await;
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
    // Spawn a task to poll the MQTT event loop
    let eventloop_handle = tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(MqttEvent::Incoming(Packet::Publish(publish))) => {
                    println!(
                        "Received MQTT packet: {}: {:?}",
                        publish.topic, publish.payload
                    );
                    match publish.topic.as_str() {
                        "sven/state" => {
                            // Deserialize the payload into SvenState
                            if let Ok(state) = serde_json::from_slice::<SvenState>(&publish.payload)
                            {
                                let mut sven_state = mqtt_app_state.sven_state.lock().await;
                                *sven_state = state;
                                println!("Updated Sven state: {:?}", *sven_state);
                            } else {
                                eprintln!("Failed to deserialize Sven state");
                            }
                        }
                        SVEN_STATUS_TOPIC => {
                            if let Ok(status) = String::from_utf8(publish.payload.to_vec()) {
                                let mut sven_status = mqtt_app_state.sven_status.lock().await;
                                *sven_status = status;
                                println!("Updated Sven status: {}", *sven_status);
                            } else {
                                eprintln!("Failed to deserialize Sven status");
                            }
                        }
                        _ => eprintln!("Unknown topic: {}", publish.topic),
                    }
                }
                Ok(MqttEvent::Outgoing(Outgoing::Publish(publish))) => {
                    println!("MQTT Published packet: {:?}", publish);
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
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
            format!("/api/{}", SVEN_COMMAND_TOPIC).as_str(),
            post({
                let shared_state = app_state.clone();
                move |body| {
                    println!("Received command: {:?}", body);
                    handle_command(body, Extension(shared_state))
                }
            }),
        )
        .route(
            format!("/api/{}", SVEN_STATE_TOPIC).as_str(),
            get(get_sven_state),
        )
        .route(
            format!("/api/{}", SVEN_STATUS_TOPIC).as_str(),
            get(get_sven_status),
        )
        .layer(Extension(app_state))
        .layer(cors);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3001").await.unwrap();
    axum::serve(listener, app).await.unwrap();

    let _ = eventloop_handle.await;
}
