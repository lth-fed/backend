use fed_auth_verifier::AuthContext;
use minilith::get_endpoint;
use poem::{Server, listener::TcpListener};

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let context = minilith::Context::new(None).await?;
    Server::new(TcpListener::bind("[::]:8000"))
        .run(get_endpoint(
            context,
            AuthContext::new("teknologappen").await?,
        )?)
        .await?;

    Ok(())
}
