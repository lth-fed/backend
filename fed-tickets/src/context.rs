use sqlx::PgPool;

#[derive(Debug, Clone)]
pub struct Context {
    pub db: PgPool,
}
