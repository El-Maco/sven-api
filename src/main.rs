use axum::{Json, Router, extract::Extension, routing::post};
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

#[derive(Debug, Deserialize, Serialize)]
struct Command {
    direction: Direction,
    duration: u32,
}

// Shared state for MQTT client
struct AppState {
    mqtt_client: Arc<Mutex<AsyncClient>>,
}

async fn handle_command(
    Json(command): Json<Command>,
    state: Extension<Arc<AppState>>,
) -> &'static str {
    println!(
        "Moving Sven {} for {} ms",
        command.direction, command.duration
    );

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

    "Command received"
}

#[tokio::main]
async fn main() {
    // MQTT client setup
    let mut mqttoptions = MqttOptions::new("sven-client", "localhost", 1883);
    mqttoptions.set_keep_alive(std::time::Duration::from_secs(5));

    let (mqtt_client, mut eventloop) = AsyncClient::new(mqttoptions, 10);
    let app_state = Arc::new(AppState {
        mqtt_client: Arc::new(Mutex::new(mqtt_client)),
    });

    // Spawn a task to poll the MQTT event loop
    let eventloop_handle = tokio::spawn(async move {
        loop {
            let _ = eventloop.poll().await;
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
                move |body| handle_command(body, Extension(shared_state))
            }),
        )
        .layer(Extension(app_state))
        .layer(cors);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();

    let _ = eventloop_handle.await;
}
