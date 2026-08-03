use minilith::get_endpoint;
use poem::{Server, listener::TcpListener};

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    Server::new(TcpListener::bind("[::]:8000"))
        .run(get_endpoint(None, true).await.inspect_err(|error| {
            minilith_errors::alert(
                minilith_errors::AlertLevel::L2,
                format!("get_endpoint failed: {error:?}"),
            );
        })?)
        .await
        .inspect_err(|error| {
            minilith_errors::alert(
                minilith_errors::AlertLevel::L2,
                format!("server run: {error:?}"),
            );
        })?;

    Ok(())
}
