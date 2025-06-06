use axum::{Json, Router, routing::post};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
struct Command {
    direction: Direction,
    duration: u32,
}
async fn handle_command(Json(command): Json<Command>) -> &'static str {
    println!(
        "Moving Sven {} for {} ms",
        command.direction, command.duration
    );
    "Command received"
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/api/sven/command", post(handle_command));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
