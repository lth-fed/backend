use poem::Server;
use poem::listener::TcpListener;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    Server::new(TcpListener::bind("[::]:8001"))
        .run(fed_auth::get_endpoint(None).await.inspect_err(|error| {
            minilith_errors::alert(
                minilith_errors::AlertLevel::L2,
                format!("get_endpoint failed: {error:?}"),
            );
        })?)
        .await
        .inspect_err(|error| {
            minilith_errors::alert(
                minilith_errors::AlertLevel::L2,
                format!("server run failed: {error:?}"),
            );
        })?;

    Ok(())
}
