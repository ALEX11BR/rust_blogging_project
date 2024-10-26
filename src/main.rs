use std::{error::Error, path::Path};

use askama_axum::Template;
use axum::{
    body::Bytes,
    extract::DefaultBodyLimit,
    response::Redirect,
    routing::{get, post},
    Router,
};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use regex::Regex;
use sqlx::{sqlite::SqliteConnection, Connection, Row};
use tokio::fs;
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
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS posts (
            id INTEGER PRIMARY KEY,
            author STRING,
            date STRING,
            hasimage BOOLEAN,
            hasavatar BOOLEAN,
            content TEXT,
            visible BOOLEAN
        )",
        )
        .execute(&mut conn)
        .await
        .unwrap();
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
    let rows = sqlx::query("SELECT id, author, date, hasimage, hasavatar, content FROM posts WHERE visible = TRUE ORDER BY date DESC")
        .fetch_all(&mut conn)
        .await?;

    Ok(rows
        .iter()
        .map(|row| {
            let id = row.try_get("id").unwrap_or(0);
            let user = row.try_get("author").unwrap_or("");
            let image_path = if row.try_get("hasimage").unwrap_or(false) {
                format!("/assets/images/{}.png", id)
            } else {
                "".into()
            };
            let date = row.try_get("date").unwrap_or("");
            let avatar_path = if row.try_get("hasavatar").unwrap_or(false) {
                format!("/assets/avatars/{}.png", id)
            } else {
                "".into()
            };
            let text = row.try_get("content").unwrap_or("");

            BlogPost {
                user: user.into(),
                user_avatar_path: avatar_path,
                post_date: date.into(),
                post_image_path: image_path,
                post_text: text.into(),
            }
        })
        .collect())
}
async fn insert_post(data: TypedMultipart<PostMultipartParam>) -> Result<(), Box<dyn Error>> {
    let date_regex = Regex::new("^[0-9]{4}-[0-9]{2}-[0-9]{2}$")?;
    if !date_regex.is_match(&data.date) {
        return Err("The date must be in the YYYY-MM-DD format".into());
    }

    let avatar_bytes = if !data.avatar.is_empty() {
        let avatar_pic = reqwest::get(&data.avatar).await?;
        let avatar_type = avatar_pic
            .headers()
            .get("Content-Type")
            .ok_or("Could not detect the avatar file type".to_string())?
            .to_str()?;
        if avatar_type != "image/png" {
            return Err("Avatar isn't a PNG".into());
        }

        Some(avatar_pic.bytes().await?)
    } else {
        None
    };

    if !data.image.contents.is_empty()
        && data.image.metadata.content_type != Some("image/png".into())
    {
        return Err("Post image isn't a PNG".into());
    }

    let mut conn = SqliteConnection::connect(DATABASE).await?;

    let id = sqlx::query("INSERT INTO posts (author, date, content, visible) VALUES ($1, $2, $3, FALSE) RETURNING id")
        .bind(&data.user)
        .bind(&data.date)
        .bind(&data.text)
        .fetch_one(&mut conn)
        .await?
        .get::<i64, _>("id");

    if let Some(ref avatar_bytes) = avatar_bytes {
        fs::write(
            Path::new("assets")
                .join("avatars")
                .join(format!("{}.png", id)),
            avatar_bytes,
        )
        .await?;
    }
    if !data.image.contents.is_empty() {
        fs::write(
            Path::new("assets")
                .join("images")
                .join(format!("{}.png", id)),
            &data.image.contents,
        )
        .await?;
    }

    sqlx::query("UPDATE posts SET hasimage = $1, hasavatar = $2, visible = TRUE WHERE id = $3")
        .bind(!data.image.contents.is_empty())
        .bind(avatar_bytes != None)
        .bind(id)
        .execute(&mut conn)
        .await?;

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
    image: FieldData<Bytes>,
    text: String,
}
