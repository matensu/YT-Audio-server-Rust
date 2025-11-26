use axum::{
    body::Body,
    extract::{Path, Query},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use bytes::Bytes;
use dotenv::dotenv;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, env, net::SocketAddr};
use thiserror::Error;
use tokio::{io::AsyncReadExt, process::Command as AsyncCommand, sync::mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info};
use tracing_subscriber;

/// ======= ERROR HANDLING =======
#[derive(Debug, Error)]
pub enum AudioStreamError {
    #[error("Failed to execute yt-dlp: {0}")]
    ProcessError(#[from] std::io::Error),

    #[error("Invalid YouTube ID or video not found")]
    InvalidYouTubeId,

    #[error("Internal server error")]
    InternalError,

    #[error("Spotify API error: {0}")]
    SpotifyError(String),

    #[error("Missing environment variable: {0}")]
    EnvVarError(String),
}

impl IntoResponse for AudioStreamError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            AudioStreamError::ProcessError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AudioStreamError::InvalidYouTubeId => StatusCode::BAD_REQUEST,
            AudioStreamError::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
            AudioStreamError::SpotifyError(_) => StatusCode::BAD_GATEWAY,
            AudioStreamError::EnvVarError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        #[derive(Serialize)]
        struct ErrJson {
            error: String,
        }

        (status, Json(ErrJson { error: self.to_string() })).into_response()
    }
}

/// ======= STREAM YOUTUBE AUDIO =======
async fn stream_youtube(Path(youtube_id): Path<String>)
    -> Result<impl IntoResponse, AudioStreamError>
{
    if youtube_id.len() != 11 {
        return Err(AudioStreamError::InvalidYouTubeId);
    }

    info!("Requested YouTube audio stream: {}", youtube_id);
    let url = format!("https://www.youtube.com/watch?v={}", youtube_id);

    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(32);

    tokio::spawn(async move {
        let child_result = AsyncCommand::new("yt-dlp")
            .args(&["-x", "--audio-format", "mp3", "-o", "-", &url])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();

        match child_result {
            Ok(mut child) => {
                if let Some(stdout) = child.stdout.take() {
                    let mut reader = tokio::io::BufReader::new(stdout);
                    let mut buffer = [0u8; 8192];

                    loop {
                        match reader.read(&mut buffer).await {
                            Ok(0) => break,
                            Ok(n) => {
                                if tx.send(Ok(Bytes::copy_from_slice(&buffer[..n]))).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                let _ = tx.send(Err(e)).await;
                                break;
                            }
                        }
                    }
                }
                let _ = child.wait().await;
            }
            Err(e) => {
                let _ = tx.send(Err(e)).await;
            }
        }
    });

    let stream = ReceiverStream::new(rx);

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/mpeg"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));

    Ok((headers, Body::from_stream(stream)))
}

/// ======= YOUTUBE SEARCH =======
#[derive(Deserialize)]
struct YtQuery {
    title: String,
    artist: Option<String>,
}

#[derive(Serialize)]
struct YtResponse {
    youtubeId: String,
}

async fn yt_search(Query(params): Query<YtQuery>) -> Result<Json<YtResponse>, AudioStreamError> {
    let search_query = if let Some(artist) = &params.artist {
        format!("{} {}", params.title, artist)
    } else {
        params.title.clone()
    };

    let output = AsyncCommand::new("yt-dlp")
        .args(&["--get-id", &format!("ytsearch1:{}", search_query)])
        .output()
        .await
        .map_err(|e| AudioStreamError::ProcessError(e))?;

    if !output.status.success() {
        return Err(AudioStreamError::InvalidYouTubeId);
    }

    let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if id.is_empty() {
        return Err(AudioStreamError::InvalidYouTubeId);
    }

    info!("YouTube search for '{}' returned ID: {}", search_query, id);

    Ok(Json(YtResponse { youtubeId: id }))
}

/// ======= SPOTIFY STRUCT ========
#[derive(Debug, Serialize, Deserialize)]
struct SpotifyTrack {
    id: String,
    name: String,
    artists: Vec<String>,
    artwork: String,
}

/// ======= SPOTIFY SEARCH ========
async fn spotify_search(
    Json(body): Json<HashMap<String, String>>,
) -> Result<Json<Vec<SpotifyTrack>>, AudioStreamError> {
    dotenv().ok();

    let q = body.get("query")
        .ok_or_else(|| AudioStreamError::SpotifyError("Missing query".into()))?
        .to_string();

    let id = env::var("SPOTIFY_CLIENT_ID")
        .map_err(|_| AudioStreamError::EnvVarError("SPOTIFY_CLIENT_ID".into()))?;

    let secret = env::var("SPOTIFY_CLIENT_SECRET")
        .map_err(|_| AudioStreamError::EnvVarError("SPOTIFY_CLIENT_SECRET".into()))?;

    let token = get_spotify_token(&id, &secret)
        .await
        .map_err(|e| AudioStreamError::SpotifyError(e.to_string()))?;

    let tracks = search_spotify_tracks(&token, &q)
        .await
        .map_err(|e| AudioStreamError::SpotifyError(e.to_string()))?;

    Ok(Json(tracks))
}

/// ======= SPOTIFY TOKEN ========
async fn get_spotify_token(id: &str, secret: &str)
    -> Result<String, Box<dyn std::error::Error>>
{
    let client = reqwest::Client::new();
    let creds = Base64Engine.encode(format!("{id}:{secret}"));

    let res = client
        .post("https://accounts.spotify.com/api/token")
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(AUTHORIZATION, format!("Basic {}", creds))
        .form(&[("grant_type", "client_credentials")])
        .send()
        .await?;

    let json: Value = res.json().await?;
    Ok(json["access_token"].as_str().unwrap().to_string())
}

/// ======= SPOTIFY TRACK SEARCH ========
async fn search_spotify_tracks(
    token: &str,
    query: &str,
) -> Result<Vec<SpotifyTrack>, Box<dyn std::error::Error>> {
    let url = format!(
        "https://api.spotify.com/v1/search?q={}&type=track&limit=10",
        urlencoding::encode(query)
    );

    let res = reqwest::Client::new()
        .get(url)
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await?;

    let json: Value = res.json().await?;
 let items = json["tracks"]["items"]
    .as_array()
    .map(|arr| arr.to_owned())  // clone des données pour éviter la référence temporaire
    .unwrap_or_default();

let results = items
    .iter()
    .map(|i| SpotifyTrack {
        id: i["id"].as_str().unwrap_or("").into(),
        name: i["name"].as_str().unwrap_or("").into(),
        artists: i["artists"].as_array().unwrap_or(&vec![])
            .iter()
            .map(|a| a["name"].as_str().unwrap_or("").into())
            .collect(),
        artwork: i["album"]["images"][0]["url"]
            .as_str()
            .unwrap_or("")
            .into(),
    })
    .collect();

    Ok(results)
}

/// ======= MAIN ========
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    tracing_subscriber::fmt::init();

    info!("Server starting...");

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(vec![Method::GET, Method::POST]) // ✅ vec pour éviter l'erreur
        .allow_headers(Any);

    let app = Router::new()
        .route("/youtube/:youtube_id", get(stream_youtube))
        .route("/stream/:youtube_id", get(stream_youtube))
        .route("/yt/search", get(yt_search))       // ✅ route ajoutée
        .route("/spotify/search", post(spotify_search))
        .layer(cors.clone());

    let port = std::env::var("PORT")
    .unwrap_or_else(|_| "3000".to_string())
    .parse::<u16>()
    .unwrap_or(3000);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    info!("Listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
