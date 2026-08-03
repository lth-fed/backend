use poem::{Server, listener::TcpListener};
use transactions::get_endpoint;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    Server::new(TcpListener::bind("[::]:8002"))
        .run(get_endpoint(None).await.inspect_err(|error| {
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
