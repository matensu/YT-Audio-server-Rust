use axum::{
    body::Body,
    extract::Path,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use bytes::Bytes;
use std::{
    fmt,
    net::SocketAddr,
    process::Stdio,
};
use tokio::{
    io::AsyncReadExt,
    process::Command as AsyncCommand,
    sync::mpsc,
};

#[derive(Debug)]
pub enum AudioStreamError {
    NotFound,
    FileOpenError(std::io::Error),
    InvalidRange,
    RangeNotSatisfiable,
}

impl fmt::Display for AudioStreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioStreamError::NotFound => write!(f, "File not found"),
            AudioStreamError::FileOpenError(e) => write!(f, "Failed to open file: {}", e),
            AudioStreamError::InvalidRange => write!(f, "Invalid range header"),
            AudioStreamError::RangeNotSatisfiable => write!(f, "Range not satisfiable"),
        }
    }
}

impl std::error::Error for AudioStreamError {}

impl From<std::io::Error> for AudioStreamError {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::NotFound {
            AudioStreamError::NotFound
        } else {
            AudioStreamError::FileOpenError(error)
        }
    }
}

impl IntoResponse for AudioStreamError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            AudioStreamError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            AudioStreamError::FileOpenError(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Failed to process file".to_string())
            }
            AudioStreamError::InvalidRange => (StatusCode::BAD_REQUEST, self.to_string()),
            AudioStreamError::RangeNotSatisfiable => {
                (StatusCode::RANGE_NOT_SATISFIABLE, self.to_string())
            }
        };
        (status, message).into_response()
    }
}

async fn stream_youtube(
    Path(youtube_id): Path<String>,
) -> Result<impl IntoResponse, AudioStreamError> {
    println!("[INFO] Requested YouTube video ID: {}", youtube_id);
    let url = format!("https://www.youtube.com/watch?v={}", youtube_id);

    // Create a channel to stream the audio data with explicit type annotation
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(32);

    // Spawn a task to handle the yt-dlp process
    tokio::spawn(async move {
        if let Ok(mut child) = AsyncCommand::new("yt-dlp")
            .args(&[
                "-x",
                "--audio-format", "mp3",
                "--no-playlist",
                "-o", "-",
                &url,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            let stdout = child.stdout.take().unwrap();
            let mut reader = tokio::io::BufReader::new(stdout);
            let mut buffer = [0u8; 8192]; // 8KB buffer

            loop {
                match reader.read(&mut buffer).await {
                    Ok(0) => break, // EOF
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
    });

    // Create a stream from the channel
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

    // Set up headers
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "audio/mpeg".parse().unwrap());
    headers.insert(header::TRANSFER_ENCODING, "chunked".parse().unwrap());

    Ok((headers, Body::from_stream(stream)))
}

#[tokio::main]
async fn main() {
    // Simple logging
    println!("Server starting...");

    let app = Router::new()
        .route("/youtube/:youtube_id", get(stream_youtube));

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("Listening on {}", addr);
    println!("Try accessing: http://localhost:3000/youtube/dQw4w9WgXcQ");
    
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}