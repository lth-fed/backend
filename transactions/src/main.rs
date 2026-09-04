use bin_common::{listeners, shutdown_signal};
use poem::Server;
use transactions::get_endpoint;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    Server::new(listeners(8002))
        .run_with_graceful_shutdown(
            get_endpoint(None).await.inspect_err(|error| {
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
                format!("server run: {error:?}"),
            );
        })?;

    Ok(())
}
