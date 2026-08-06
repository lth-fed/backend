use std::sync::Arc;

use tracing::error;

use crate::context::ContextWrapper;

pub async fn spawn(ctx: &ContextWrapper) {
    let ctx = Arc::clone(ctx);
    tokio::spawn(async move {
        loop {
            let res = sqlx::query!(
                "with saml2 as (delete from saml2_request_id_cache
                    where created < now() - '1 hour'::interval),
                email as (delete from email_token_holding
                    where created < now() - '1 hour'::interval)
                delete from sessions
                where created < now() - '1 hour'::interval"
            )
            .execute(&ctx.db)
            .await;
            if let Err(error) = res {
                error!(?error, "failed to clean up caches");
            }

            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}
