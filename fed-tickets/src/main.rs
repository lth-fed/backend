use fed_tickets::{activities, groups};
use poem::{Route, Server, listener::TcpListener};
use poem_openapi::OpenApiService;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    let api_service = OpenApiService::new(
        (activities::Router, groups::Router),
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    .server("http://localhost:8000/v0");
    let ui = api_service.swagger_ui();

    Server::new(TcpListener::bind("0.0.0.0:8000"))
        .run(Route::new().nest("/v0", api_service).nest("/v0/docs", ui))
        .await?;

    Ok(())
}
