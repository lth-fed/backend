use sqlx::postgres::PgPool;
use time::OffsetDateTime;

struct User {
    id: String,
    name: String,
    language: String,
    latest_refresh: OffsetDateTime,
    inactive_since: OffsetDateTime,
}

impl User {
    async fn save(&self, db: &PgPool) -> Result<(), sqlx::Error> {
        let query = sqlx::query!(
            "insert into users (id, name, language, latest_refresh, inactive_since) values ($1, $2, $3, $4, $5)",
            self.id,
            self.name.as_bytes(),
            self.language.as_bytes(),
            self.latest_refresh,
            self.inactive_since,
        );
        query.execute(db).await?;
        Ok(())
    }

    pub fn new(id: &str, name: &str, language: &str) -> Self {
        User {
            id: id.to_owned(),
            name: name.to_owned(),
            language: language.to_owned(),
            latest_refresh: OffsetDateTime::now_utc(),
            inactive_since: OffsetDateTime::now_utc(),
        }
    }
}

pub async fn run(db: &PgPool) {
    let user = User::new("id4", "name", "swedish");
    if let Err(err) = user.save(db).await {
        eprint!("{err}");
    }
}
