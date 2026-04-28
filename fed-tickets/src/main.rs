pub mod activities;
pub mod context;
pub mod groups;

use color_eyre::eyre::Context as _;
use poem::{Route, Server, listener::TcpListener};
use poem_openapi::OpenApiService;
use sqlx::migrate::MigrateDatabase as _;
use sqlx::Postgres;
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    drop(dotenvy::dotenv());

    tracing_subscriber::fmt().init();

    let db_url = std::env::var("DATABASE_URL").wrap_err("DATABASE_URL not set")?;
    if !Postgres::database_exists(&db_url).await? {
        Postgres::create_database(&db_url).await?;
    }

    let db = PgPoolOptions::new()
        .max_connections(50)
        .connect(&db_url)
        .await
        .wrap_err("failed to connect to database")?;

    sqlx::migrate!("./migrations").run(&db).await?;

    let context = context::Context { db };
    let api_service = OpenApiService::new(
        (
            activities::Router {
                context: context.clone(),
            },
            groups::Router { context },
        ),
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server("http://localhost:8000/v0");
    let ui = api_service.swagger_ui();

    Server::new(TcpListener::bind("0.0.0.0:8000"))
        .run(Route::new().nest("/v0", api_service).nest("/v0/docs", ui))
        .await?;

    Ok(())
}
