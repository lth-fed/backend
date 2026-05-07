use sqlx::postgres::PgPool;
use time::OffsetDateTime;

pub async fn run(db: &PgPool) {
    let id = "id";
    let name = "name";
    let language = "lang";
    let latest_refresh = OffsetDateTime::now_utc();
    if let Err(err) = user(db, id, name, language, latest_refresh).await {
        eprint!("{err}");
    }
}

/// # Errors
/// Test.
async fn user(
    db: &PgPool,
    id: &str,
    name: &str,
    language: &str,
    latest_refresh: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let create_user = sqlx::query!(
        "insert into users (id, name, language, latest_refresh) values ($1, $2, $3, $4)",
        id,
        name.as_bytes(),
        language.as_bytes(),
        latest_refresh
    );

    create_user.execute(db).await?;
    Ok(())
}
