pub mod activities;
pub mod context;
pub mod groups;
pub mod healthcheck;

use std::path::PathBuf;

use color_eyre::{Section as _, eyre::Context as _};
use poem::{Route, Server, listener::TcpListener};
use poem_openapi::OpenApiService;
use sqlx::migrate::MigrateDatabase as _;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let _: Result<PathBuf, dotenvy::Error> = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let db = setup_db(&std::env::var("DATABASE_URL").wrap_err("`DATABASE_URL` not set")?)
        .await
        .wrap_err("Failed to set up the database")
        .suggestion("Start the database with `docker compose up -d`")?;

    let context = context::Context { db };
    let api_service = OpenApiService::new(
        (
            activities::Router {
                context: context.clone(),
            },
            groups::Router {
                context: context.clone(),
            },
            healthcheck::Router {
                context: context.clone(),
            },
        ),
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server("http://localhost:8000/v0");
    let ui = api_service.swagger_ui();

    Server::new(TcpListener::bind("[::]:8000"))
        .run(Route::new().nest("/v0", api_service).nest("/v0/docs", ui))
        .await?;

    Ok(())
}

async fn setup_db(db_url: &str) -> color_eyre::Result<PgPool> {
    if !Postgres::database_exists(db_url)
        .await
        .wrap_err("Failed to check if database exists")?
    {
        Postgres::create_database(db_url).await?;
    }

    let db = PgPoolOptions::new()
        .max_connections(50)
        .connect(db_url)
        .await
        .wrap_err("Failed to create database pool")?;

    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .wrap_err("Failed to run migrations")?;

    Ok(db)
}
