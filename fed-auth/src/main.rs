use poem::Server;
use poem::listener::TcpListener;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    Server::new(TcpListener::bind("[::]:8001"))
        .run(fed_auth::get_endpoint(None).await?)
        .await?;

    Ok(())
}
