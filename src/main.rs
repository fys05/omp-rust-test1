use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqlitePool, FromRow};
use std::net::SocketAddr;
use tower_http::{cors::CorsLayer, services::ServeDir};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    pool: SqlitePool,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
struct Book {
    id: i64,
    title: String,
    author: String,
    isbn: Option<String>,
    category: Option<String>,
    published_year: Option<i32>,
    total_copies: i32,
    available_copies: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct CreateBook {
    title: String,
    author: String,
    isbn: Option<String>,
    category: Option<String>,
    published_year: Option<i32>,
    total_copies: i32,
}

#[derive(Debug, Deserialize)]
struct UpdateBook {
    title: Option<String>,
    author: Option<String>,
    isbn: Option<String>,
    category: Option<String>,
    published_year: Option<i32>,
    total_copies: Option<i32>,
    available_copies: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
struct Member {
    id: i64,
    name: String,
    email: String,
    phone: Option<String>,
    membership_date: DateTime<Utc>,
    is_active: bool,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct CreateMember {
    name: String,
    email: String,
    phone: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
struct BorrowRecord {
    id: i64,
    book_id: i64,
    member_id: i64,
    borrow_date: DateTime<Utc>,
    due_date: DateTime<Utc>,
    return_date: Option<DateTime<Utc>>,
    status: String, // borrowed, returned, overdue
    created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct BorrowBook {
    book_id: i64,
    member_id: i64,
    days: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct ReturnBook {
    record_id: i64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let pool = SqlitePool::connect("sqlite:library.db?mode=rwc").await?;
    init_db(&pool).await?;

    let state = AppState { pool };

    let app = Router::new()
        .route("/api/books", get(list_books).post(create_book))
        .route("/api/books/{id}", get(get_book).put(update_book).delete(delete_book))
        .route("/api/members", get(list_members).post(create_member))
        .route("/api/members/{id}", get(get_member).delete(delete_member))
        .route("/api/borrow", post(borrow_book))
        .route("/api/return", post(return_book))
        .route("/api/records", get(list_records))
        .route("/api/stats", get(get_stats))
        .fallback_service(ServeDir::new("static"))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3001));
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn init_db(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS books (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            author TEXT NOT NULL,
            isbn TEXT,
            category TEXT,
            published_year INTEGER,
            total_copies INTEGER NOT NULL DEFAULT 1,
            available_copies INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS members (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            email TEXT NOT NULL UNIQUE,
            phone TEXT,
            membership_date TEXT NOT NULL DEFAULT (datetime('now')),
            is_active INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS borrow_records (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            book_id INTEGER NOT NULL,
            member_id INTEGER NOT NULL,
            borrow_date TEXT NOT NULL DEFAULT (datetime('now')),
            due_date TEXT NOT NULL,
            return_date TEXT,
            status TEXT NOT NULL DEFAULT 'borrowed',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (book_id) REFERENCES books(id),
            FOREIGN KEY (member_id) REFERENCES members(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

// Book handlers
async fn list_books(State(state): State<AppState>) -> Result<Json<Vec<Book>>, StatusCode> {
    let books = sqlx::query_as::<_, Book>("SELECT * FROM books ORDER BY id DESC")
        .fetch_all(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(books))
}

async fn get_book(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Book>, StatusCode> {
    let book = sqlx::query_as::<_, Book>("SELECT * FROM books WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(book))
}

async fn create_book(
    State(state): State<AppState>,
    Json(payload): Json<CreateBook>,
) -> Result<(StatusCode, Json<Book>), StatusCode> {
    let available = payload.total_copies;
    let result = sqlx::query(
        "INSERT INTO books (title, author, isbn, category, published_year, total_copies, available_copies) VALUES (?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&payload.title)
    .bind(&payload.author)
    .bind(&payload.isbn)
    .bind(&payload.category)
    .bind(payload.published_year)
    .bind(payload.total_copies)
    .bind(available)
    .execute(&state.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let id = result.last_insert_rowid();
    let book = sqlx::query_as::<_, Book>("SELECT * FROM books WHERE id = ?")
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((StatusCode::CREATED, Json(book)))
}

async fn update_book(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateBook>,
) -> Result<Json<Book>, StatusCode> {
    let existing = sqlx::query_as::<_, Book>("SELECT * FROM books WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let title = payload.title.unwrap_or(existing.title);
    let author = payload.author.unwrap_or(existing.author);
    let isbn = payload.isbn.or(existing.isbn);
    let category = payload.category.or(existing.category);
    let year = payload.published_year.or(existing.published_year);
    let total = payload.total_copies.unwrap_or(existing.total_copies);
    let avail = payload.available_copies.unwrap_or(existing.available_copies);

    sqlx::query(
        "UPDATE books SET title=?, author=?, isbn=?, category=?, published_year=?, total_copies=?, available_copies=?, updated_at=datetime('now') WHERE id=?"
    )
    .bind(&title)
    .bind(&author)
    .bind(&isbn)
    .bind(&category)
    .bind(year)
    .bind(total)
    .bind(avail)
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let book = sqlx::query_as::<_, Book>("SELECT * FROM books WHERE id = ?")
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(book))
}

async fn delete_book(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    sqlx::query("DELETE FROM books WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

// Member handlers
async fn list_members(State(state): State<AppState>) -> Result<Json<Vec<Member>>, StatusCode> {
    let members = sqlx::query_as::<_, Member>("SELECT * FROM members ORDER BY id DESC")
        .fetch_all(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(members))
}

async fn get_member(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Member>, StatusCode> {
    let member = sqlx::query_as::<_, Member>("SELECT * FROM members WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(member))
}

async fn create_member(
    State(state): State<AppState>,
    Json(payload): Json<CreateMember>,
) -> Result<(StatusCode, Json<Member>), StatusCode> {
    let result = sqlx::query("INSERT INTO members (name, email, phone) VALUES (?, ?, ?)")
        .bind(&payload.name)
        .bind(&payload.email)
        .bind(&payload.phone)
        .execute(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let id = result.last_insert_rowid();
    let member = sqlx::query_as::<_, Member>("SELECT * FROM members WHERE id = ?")
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((StatusCode::CREATED, Json(member)))
}

async fn delete_member(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    sqlx::query("DELETE FROM members WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

// Borrow/Return handlers
async fn borrow_book(
    State(state): State<AppState>,
    Json(payload): Json<BorrowBook>,
) -> Result<(StatusCode, Json<BorrowRecord>), StatusCode> {
    let mut tx = state.pool.begin().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let book = sqlx::query_as::<_, Book>("SELECT * FROM books WHERE id = ?")
        .bind(payload.book_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if book.available_copies < 1 {
        return Err(StatusCode::BAD_REQUEST);
    }

    let days = payload.days.unwrap_or(14);
    let due = Utc::now() + chrono::Duration::days(days as i64);

    sqlx::query("UPDATE books SET available_copies = available_copies - 1, updated_at = datetime('now') WHERE id = ?")
        .bind(payload.book_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let result = sqlx::query(
        "INSERT INTO borrow_records (book_id, member_id, due_date, status) VALUES (?, ?, ?, 'borrowed')"
    )
    .bind(payload.book_id)
    .bind(payload.member_id)
    .bind(due)
    .execute(&mut *tx)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let id = result.last_insert_rowid();
    let record = sqlx::query_as::<_, BorrowRecord>("SELECT * FROM borrow_records WHERE id = ?")
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    tx.commit().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((StatusCode::CREATED, Json(record)))
}

async fn return_book(
    State(state): State<AppState>,
    Json(payload): Json<ReturnBook>,
) -> Result<Json<BorrowRecord>, StatusCode> {
    let mut tx = state.pool.begin().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let record = sqlx::query_as::<_, BorrowRecord>("SELECT * FROM borrow_records WHERE id = ?")
        .bind(payload.record_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if record.status == "returned" {
        return Err(StatusCode::BAD_REQUEST);
    }

    sqlx::query("UPDATE borrow_records SET return_date = datetime('now'), status = 'returned' WHERE id = ?")
        .bind(payload.record_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    sqlx::query("UPDATE books SET available_copies = available_copies + 1, updated_at = datetime('now') WHERE id = ?")
        .bind(record.book_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let updated = sqlx::query_as::<_, BorrowRecord>("SELECT * FROM borrow_records WHERE id = ?")
        .bind(payload.record_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    tx.commit().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(updated))
}

async fn list_records(State(state): State<AppState>) -> Result<Json<Vec<BorrowRecord>>, StatusCode> {
    let records = sqlx::query_as::<_, BorrowRecord>("SELECT * FROM borrow_records ORDER BY id DESC")
        .fetch_all(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(records))
}

#[derive(Serialize)]
struct Stats {
    total_books: i64,
    total_members: i64,
    active_borrows: i64,
    overdue: i64,
}

async fn get_stats(State(state): State<AppState>) -> Result<Json<Stats>, StatusCode> {
    let total_books: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM books")
        .fetch_one(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let total_members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members WHERE is_active = 1")
        .fetch_one(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let active_borrows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM borrow_records WHERE status = 'borrowed'")
        .fetch_one(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let overdue: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM borrow_records WHERE status = 'borrowed' AND due_date < datetime('now')")
        .fetch_one(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(Stats {
        total_books,
        total_members,
        active_borrows,
        overdue,
    }))
}
