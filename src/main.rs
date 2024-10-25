use std::error::Error;

use askama_axum::Template;
use axum::{
    extract::DefaultBodyLimit,
    response::Redirect,
    routing::{get, post}, Router,
};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use sqlx::{sqlite::SqliteConnection, Connection, Row};
use tempfile::NamedTempFile;
use tower_http::{services::ServeDir, trace::TraceLayer};
use tower_sessions::{cookie::time::Duration, Expiry, MemoryStore, Session, SessionManagerLayer};

const DATABASE: &'static str = "posts.db";
const POST_STATUS_KEY: &'static str = "poststatus";

#[tokio::main]
async fn main() {
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(Duration::seconds(10)));

    let app = Router::new()
        .route("/home", get(serve_home))
        .route("/post", post(make_post))
        .nest_service("/assets", ServeDir::new("assets"))
        .route("/", get(return_home))
        .layer(session_layer)
        .layer(DefaultBodyLimit::max(1024 * 1024 * 5)) // 5 MiB
        .layer(TraceLayer::new_for_http());

    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    {
        let mut conn = SqliteConnection::connect(DATABASE).await.unwrap();
        sqlx::query("CREATE TABLE IF NOT EXISTS posts (
            id INTEGER PRIMARY KEY,
            content TEXT
        )").execute(&mut conn).await.unwrap();
    }

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::debug!("Listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

async fn serve_home(session: Session) -> IndexHtml {
    let new_post_status = session
        .get::<Result<(), String>>(POST_STATUS_KEY)
        .await
        .ok()
        .flatten();

    let _ = session.remove_value(POST_STATUS_KEY).await;
    IndexHtml {
        new_post_status,
        posts: get_posts().await.map_err(|err| err.to_string()),
    }
}
async fn make_post(session: Session, data: TypedMultipart<PostMultipartParam>) -> Redirect {
    let insert_post_result = insert_post(data).await.map_err(|err| err.to_string());

    let _ = session.insert(POST_STATUS_KEY, insert_post_result).await;

    Redirect::to("/home")
}
async fn return_home() -> Redirect {
    return Redirect::permanent("/home");
}

async fn get_posts() -> Result<Vec<BlogPost>, Box<dyn Error>> {
    let mut conn = SqliteConnection::connect(DATABASE).await?;
    let rows = sqlx::query("SELECT content FROM posts").fetch_all(&mut conn).await?;

    Ok(rows.iter().map(|row| {
        let text = row.try_get("content").unwrap_or("");

        BlogPost { user: "aas".into(), user_avatar_path: "".into(), post_date: "2024-11-11".into(), post_image_path: "sssss".into(), post_text: text.into() }
    }).collect())
}
async fn insert_post(data: TypedMultipart<PostMultipartParam>) -> Result<(), Box<dyn Error>> {
    if !data.avatar.is_empty() {
        let avatar_pic = reqwest::get(&data.avatar).await?;
        let avatar_type = avatar_pic
            .headers()
            .get("Content-Type")
            .ok_or("Avatar isn't a PNG".to_string())
            .and_then(|val| val.to_str().map_err(|err| err.to_string()))?;
        if avatar_type != "image/png" {
            return Err("Avatar isn't a PNG.".into());
        }
    }

    let mut conn = SqliteConnection::connect(DATABASE).await?;
    let mut tx = conn.begin().await?;

    let id = sqlx::query("INSERT INTO posts (content) VALUES ($1) RETURNING id").bind(&data.text).fetch_one(&mut *tx).await?.get::<i32, _>("id");
    println!("{}", id);

    tx.commit().await?;

    Ok(())
}

#[derive(Debug, Template)]
#[template(path = "index.html")]
struct IndexHtml {
    new_post_status: Option<Result<(), String>>,
    posts: Result<Vec<BlogPost>, String>,
}
#[derive(Debug)]
struct BlogPost {
    user: String,
    user_avatar_path: String,
    post_date: String,
    post_image_path: String,
    post_text: String,
}

#[derive(Debug, TryFromMultipart)]
struct PostMultipartParam {
    user: String,
    #[form_data(default)]
    avatar: String,
    date: String,
    #[form_data(limit = "4MiB")]
    image: Option<FieldData<NamedTempFile>>,
    text: String,
}
