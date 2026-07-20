use poem::{Server, listener::TcpListener};
use transactions::get_endpoint;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    Server::new(TcpListener::bind("[::]:8002"))
        .run(get_endpoint(None).await?)
        .await?;

    Ok(())
}
