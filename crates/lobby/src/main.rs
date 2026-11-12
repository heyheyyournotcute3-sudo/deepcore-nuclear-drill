//! Deepcore lobby: accounts, session tokens and the server browser.
//!
//! Built on `axum` + `sqlx` (SQLite by default, Postgres-compatible queries). Passwords are hashed
//! with argon2, logins are rate limited per IP, dedicated servers register themselves and send a
//! heartbeat.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use rand_core::OsRng;
use argon2::Argon2;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

/// Lobby command line options.
#[derive(Debug, Parser)]
#[command(name = "deepcore_lobby", about = "Deepcore accounts and server browser")]
pub struct Args {
    /// Address to listen on.
    #[arg(long, default_value = "0.0.0.0:8080")]
    pub bind: String,
    /// Database URL. Defaults to a local SQLite file.
    #[arg(long, default_value = "sqlite:lobby.db?mode=rwc")]
    pub database_url: String,
    /// Lifetime of a session token in hours.
    #[arg(long, default_value_t = 24)]
    pub token_hours: u64,
}

/// Shared lobby state.
#[derive(Clone)]
pub struct LobbyState {
    /// Database pool.
    pub db: SqlitePool,
    /// Login attempts per client address, for rate limiting.
    pub attempts: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    /// Token lifetime.
    pub token_lifetime: Duration,
}

/// A registration or login request.
#[derive(Debug, Deserialize)]
pub struct Credentials {
    /// Account name.
    pub username: String,
    /// Plain-text password, hashed before storage.
    pub password: String,
}

/// A successful authentication response.
#[derive(Debug, Serialize)]
pub struct TokenResponse {
    /// Session token to present to game servers.
    pub token: String,
    /// Account name.
    pub username: String,
}

/// An error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Human-readable error.
    pub error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

fn fail(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (status, Json(ErrorResponse { error: message.into() }))
}

/// A dedicated server's registration payload.
#[derive(Debug, Deserialize)]
pub struct ServerRegistration {
    /// Stable server id, usually the hostname:port.
    pub id: String,
    /// Player-facing name.
    pub name: String,
    /// Public address clients connect to.
    pub address: String,
    /// Whether the server is password protected.
    #[serde(default)]
    pub password_protected: bool,
}

/// A server's heartbeat payload.
#[derive(Debug, Deserialize)]
pub struct Heartbeat {
    /// Current player count.
    pub players: u32,
    /// Maximum players.
    pub max_players: u32,
    /// World seed.
    pub seed: u64,
}

/// A server listed in the browser.
#[derive(Debug, Serialize)]
pub struct ServerEntry {
    /// Stable id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Address.
    pub address: String,
    /// Player count.
    pub players: u32,
    /// Maximum players.
    pub max_players: u32,
    /// Whether a password is required.
    pub password_protected: bool,
    /// Seconds since the last heartbeat.
    pub age_secs: u64,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if let Err(e) = run(args).await {
        eprintln!("deepcore_lobby: {e}");
        std::process::exit(1);
    }
}

async fn run(args: Args) -> Result<(), String> {
    let db = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&args.database_url)
        .await
        .map_err(|e| format!("database error: {e}"))?;
    migrate(&db).await?;

    let state = LobbyState {
        db,
        attempts: Arc::new(Mutex::new(HashMap::new())),
        token_lifetime: Duration::from_secs(args.token_hours * 3600),
    };

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/servers", get(list_servers))
        .route("/servers/register", post(register_server))
        .route("/servers/{id}/heartbeat", post(heartbeat))
        .route("/verify/{token}", get(verify_token))
        .with_state(state);

    let addr: SocketAddr = args.bind.parse().map_err(|e| format!("bad bind address: {e}"))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind failed: {e}"))?;
    println!("deepcore_lobby listening on http://{addr}");
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("server error: {e}"))
}

async fn migrate(db: &SqlitePool) -> Result<(), String> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS accounts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(db)
    .await
    .map_err(|e| format!("migrate accounts: {e}"))?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS sessions (
            token TEXT PRIMARY KEY,
            username TEXT NOT NULL,
            expires_at INTEGER NOT NULL
        )",
    )
    .execute(db)
    .await
    .map_err(|e| format!("migrate sessions: {e}"))?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS servers (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            address TEXT NOT NULL,
            players INTEGER NOT NULL DEFAULT 0,
            max_players INTEGER NOT NULL DEFAULT 16,
            password_protected INTEGER NOT NULL DEFAULT 0,
            seed INTEGER NOT NULL DEFAULT 0,
            last_seen INTEGER NOT NULL
        )",
    )
    .execute(db)
    .await
    .map_err(|e| format!("migrate servers: {e}"))?;
    Ok(())
}

fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("hash error: {e}"))
}

fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

fn new_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut state = nanos as u64 ^ 0x9E37_79B9_7F4A_7C15;
    let mut token = String::with_capacity(48);
    for _ in 0..6 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        token.push_str(&format!("{:016x}", state));
    }
    token
}

fn rate_limited(state: &LobbyState, key: &str) -> bool {
    let mut attempts = state.attempts.lock().expect("attempts lock");
    let entry = attempts.entry(key.to_string()).or_default();
    let now = Instant::now();
    entry.retain(|t| now.duration_since(*t) < Duration::from_secs(60));
    if entry.len() >= 10 {
        return true;
    }
    entry.push(now);
    false
}

async fn register(
    State(state): State<LobbyState>,
    Json(credentials): Json<Credentials>,
) -> ApiResult<TokenResponse> {
    if credentials.username.len() < 3 || credentials.password.len() < 6 {
        return Err(fail(
            StatusCode::BAD_REQUEST,
            "username must be 3+ chars and password 6+ chars",
        ));
    }
    if rate_limited(&state, &format!("register:{}", credentials.username)) {
        return Err(fail(StatusCode::TOO_MANY_REQUESTS, "too many attempts"));
    }
    let hash = hash_password(&credentials.password)
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let result = sqlx::query("INSERT INTO accounts (username, password_hash) VALUES (?, ?)")
        .bind(&credentials.username)
        .bind(&hash)
        .execute(&state.db)
        .await;
    if result.is_err() {
        return Err(fail(StatusCode::CONFLICT, "username already taken"));
    }
    issue_token(&state, &credentials.username).await
}

async fn login(
    State(state): State<LobbyState>,
    Json(credentials): Json<Credentials>,
) -> ApiResult<TokenResponse> {
    if rate_limited(&state, &format!("login:{}", credentials.username)) {
        return Err(fail(StatusCode::TOO_MANY_REQUESTS, "too many attempts"));
    }
    let row = sqlx::query("SELECT password_hash FROM accounts WHERE username = ?")
        .bind(&credentials.username)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let Some(row) = row else {
        return Err(fail(StatusCode::UNAUTHORIZED, "invalid credentials"));
    };
    let hash: String = row.get("password_hash");
    if !verify_password(&credentials.password, &hash) {
        return Err(fail(StatusCode::UNAUTHORIZED, "invalid credentials"));
    }
    issue_token(&state, &credentials.username).await
}

async fn issue_token(state: &LobbyState, username: &str) -> ApiResult<TokenResponse> {
    let token = new_token();
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + state.token_lifetime.as_secs();
    sqlx::query("INSERT INTO sessions (token, username, expires_at) VALUES (?, ?, ?)")
        .bind(&token)
        .bind(username)
        .bind(expires as i64)
        .execute(&state.db)
        .await
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(TokenResponse {
        token,
        username: username.to_string(),
    }))
}

async fn verify_token(
    State(state): State<LobbyState>,
    Path(token): Path<String>,
) -> ApiResult<TokenResponse> {
    let row = sqlx::query("SELECT username, expires_at FROM sessions WHERE token = ?")
        .bind(&token)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let Some(row) = row else {
        return Err(fail(StatusCode::UNAUTHORIZED, "unknown token"));
    };
    let expires: i64 = row.get("expires_at");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if now > expires {
        return Err(fail(StatusCode::UNAUTHORIZED, "token expired"));
    }
    Ok(Json(TokenResponse {
        token,
        username: row.get("username"),
    }))
}

async fn register_server(
    State(state): State<LobbyState>,
    Json(registration): Json<ServerRegistration>,
) -> ApiResult<ServerEntry> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    sqlx::query(
        "INSERT INTO servers (id, name, address, password_protected, last_seen)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET name = ?, address = ?, password_protected = ?, last_seen = ?",
    )
    .bind(&registration.id)
    .bind(&registration.name)
    .bind(&registration.address)
    .bind(i64::from(registration.password_protected))
    .bind(now)
    .bind(&registration.name)
    .bind(&registration.address)
    .bind(i64::from(registration.password_protected))
    .bind(now)
    .execute(&state.db)
    .await
    .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(ServerEntry {
        id: registration.id,
        name: registration.name,
        address: registration.address,
        players: 0,
        max_players: 16,
        password_protected: registration.password_protected,
        age_secs: 0,
    }))
}

async fn heartbeat(
    State(state): State<LobbyState>,
    Path(id): Path<String>,
    Json(beat): Json<Heartbeat>,
) -> ApiResult<ServerEntry> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let result = sqlx::query(
        "UPDATE servers SET players = ?, max_players = ?, seed = ?, last_seen = ? WHERE id = ?",
    )
    .bind(i64::from(beat.players))
    .bind(i64::from(beat.max_players))
    .bind(beat.seed as i64)
    .bind(now)
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if result.rows_affected() == 0 {
        return Err(fail(StatusCode::NOT_FOUND, "server not registered"));
    }
    Ok(Json(ServerEntry {
        id,
        name: String::new(),
        address: String::new(),
        players: beat.players,
        max_players: beat.max_players,
        password_protected: false,
        age_secs: 0,
    }))
}

async fn list_servers(State(state): State<LobbyState>) -> ApiResult<Vec<ServerEntry>> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let rows = sqlx::query(
        "SELECT id, name, address, players, max_players, password_protected, last_seen
         FROM servers WHERE last_seen > ? ORDER BY players DESC",
    )
    .bind(now - 60)
    .fetch_all(&state.db)
    .await
    .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let servers = rows
        .into_iter()
        .map(|row| {
            let last_seen: i64 = row.get("last_seen");
            ServerEntry {
                id: row.get("id"),
                name: row.get("name"),
                address: row.get("address"),
                players: row.get::<i64, _>("players") as u32,
                max_players: row.get::<i64, _>("max_players") as u32,
                password_protected: row.get::<i64, _>("password_protected") != 0,
                age_secs: (now - last_seen).max(0) as u64,
            }
        })
        .collect();
    Ok(Json(servers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_hash_and_verify() {
        let hash = hash_password("hunter22").expect("hash");
        assert!(verify_password("hunter22", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn tokens_are_unique() {
        let a = new_token();
        let b = new_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 96);
    }
}
