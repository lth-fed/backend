use sqlx::postgres::PgPool;
use time::OffsetDateTime;

struct User {
    id: String,
    name: String,
    language: String,
    latest_refresh: OffsetDateTime,
    creation: OffsetDateTime,
    inactive_since: Option<OffsetDateTime>,
}

impl User {
    async fn save(&self, db: &PgPool) -> Result<(), sqlx::Error> {
        let nonce: [u8; 24] = rand::random();
        let query = sqlx::query!(
            "insert into users (id, name, language, nonce, latest_refresh, creation, inactive_since) values ($1, $2, $3, $4, $5, $6, $7)",
            &self.id,
            &self.name.as_bytes(),
            &self.language.as_bytes(),
            &nonce,
            &self.latest_refresh,
            &self.creation,
            self.inactive_since,
        );
        query.execute(db).await?;
        Ok(())
    }

    pub async fn get(db: &PgPool, id: &str) -> Option<Self> {
        let query = sqlx::query!("select * from users where id = ($1)", id);
        let res = query.fetch_one(db).await;
        match res {
            Ok(val) => Some(User {
                id: val.id,
                name: String::from_utf8(val.name).unwrap_or_default(),
                language: String::from_utf8(val.language).unwrap_or_default(),
                latest_refresh: val.latest_refresh,
                creation: val.creation,
                inactive_since: val.inactive_since,
            }),
            Err(err) => {
                eprintln!("ERROR{err}");
                None
            }
        }
    }

    pub fn new(id: &str, name: &str, language: &str) -> Self {
        User {
            id: id.to_owned(),
            name: name.to_owned(),
            language: language.to_owned(),
            latest_refresh: OffsetDateTime::now_utc(),
            creation: OffsetDateTime::now_utc(),
            inactive_since: Some(OffsetDateTime::now_utc()),
        }
    }
}

pub async fn run(db: &PgPool) {
    let id = "id1";
    let user = User::new(id, "name", "swedish");
    if let Err(err) = user.save(db).await {
        eprint!("{err}");
    }
    User::get(db, id).await;
}
