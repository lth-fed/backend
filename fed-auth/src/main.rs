use bin_common::{listeners, shutdown_signal};
use poem::Server;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    Server::new(listeners(8001))
        .run_with_graceful_shutdown(
            fed_auth::get_endpoint(None).await.inspect_err(|error| {
                minilith_errors::alert(
                    minilith_errors::AlertLevel::L2,
                    format!("get_endpoint failed: {error:?}"),
                );
            })?,
            shutdown_signal(),
            Some(std::time::Duration::from_secs(5)),
        )
        .await
        .inspect_err(|error| {
            minilith_errors::alert(
                minilith_errors::AlertLevel::L2,
                format!("server run failed: {error:?}"),
            );
        })?;

    Ok(())
}
