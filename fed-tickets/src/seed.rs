use chacha20::ChaCha20;
use chacha20::cipher::KeyIvInit as _;
use chacha20::cipher::StreamCipher as _;
use hex::FromHex as _;
use sqlx::postgres::PgPool;
use sqlx::postgres::PgQueryResult;
use time::OffsetDateTime;

struct User {
    pub id: String,
    pub name: String,
    pub language: String,
    pub latest_refresh: OffsetDateTime,
    pub creation: OffsetDateTime,
    pub inactive_since: Option<OffsetDateTime>,
}

impl User {
    async fn save(&self, db: &PgPool, encryption_key: &[u8; 32]) -> Result<(), sqlx::Error> {
        let nonce: [u8; 12] = rand::random();
        let query = sqlx::query!(
            "insert into users (id, name, language, nonce, latest_refresh, creation, inactive_since) values ($1, $2, $3, $4, $5, $6, $7)",
            &self.id,
            User::endecrypt(&self.name.as_bytes(), &nonce, encryption_key),
            User::endecrypt(&self.language.as_bytes(), &nonce, encryption_key),
            &nonce,
            &self.latest_refresh,
            &self.creation,
            self.inactive_since,
        );
        query.execute(db).await?;
        Ok(())
    }

    pub async fn delete(&self, db: &PgPool) -> Result<PgQueryResult, sqlx::Error> {
        let query = sqlx::query!("delete from users where id = ($1)", &self.id);
        query.execute(db).await
    }

    // TODO Add proper error handling/returning to this beauty
    pub async fn get(db: &PgPool, id: &str, encryption_key: &[u8; 32]) -> Option<Self> {
        let query = sqlx::query!("select * from users where id = ($1)", id);
        let res = query.fetch_one(db).await;
        match res {
            Ok(val) => {
                // let nonce: [u8; 12] = val.nonce;
                let nonce: [u8; 12] = if let Ok(arr) = val.nonce.try_into() {
                    arr
                } else {
                    eprintln!("Error: expected 12 bytes");
                    return None;
                };
                Some(User {
                    id: val.id,
                    name: String::from_utf8(User::endecrypt(&val.name, &nonce, encryption_key))
                        .unwrap_or_default(),
                    language: String::from_utf8(User::endecrypt(
                        &val.language,
                        &nonce,
                        encryption_key,
                    ))
                    .unwrap_or_default(),
                    latest_refresh: val.latest_refresh,
                    creation: val.creation,
                    inactive_since: val.inactive_since,
                })
            }
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

    fn endecrypt(data: &[u8], nonce: &[u8; 12], key: &[u8; 32]) -> Vec<u8> {
        let mut cipher = ChaCha20::new(key.into(), nonce.into());
        let mut buffer = data.to_owned();
        cipher.apply_keystream(&mut buffer);
        buffer
    }
}

pub async fn run(db: &PgPool) {
    let key: [u8; 32] =
        <[u8; 32]>::from_hex(std::env::var("CHACHA20_KEY").unwrap_or_default()).unwrap_or_default();
    let id = "id5";
    let user = User::new(id, "name", "swedish");
    if let Err(err) = user.save(db, &key).await {
        eprint!("{err}");
    }
    println!("\nGetting the user...");
    let u2 = User::get(db, id, &key).await;
    println!("Got the user...");
    match u2 {
        Some(val) => {
            println!("{}", val.name);
            if let Err(err) = val.delete(db).await {
                eprint!("{err}");
            }
        }
        None => println!("Nope"),
    }
}
