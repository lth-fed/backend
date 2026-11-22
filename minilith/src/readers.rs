//! `SumUp` card readers ("Solo") attached to groups.
//!
//! `transactions` owns the `SumUp` credentials and the provider calls; this module keeps the local
//! group association in `physical_payment_terminals` and mirrors the provider's reader state.

use minilith_errors::{MinilithErrorOptionExt as _, MinilithErrorResultExt as _, MinilithResult};
use poem_openapi::Object;
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::context::ContextWrapper;
use crate::{DbInternationalizedString as DIS, InternationalizedString};

/// A reader as returned by the `transactions` service.
#[derive(Debug, Clone, Serialize, Deserialize, Object)]
pub struct Reader {
    pub id: String,
    pub name: String,
    pub status: String,
    pub device: Option<ReaderDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Object)]
pub struct ReaderDevice {
    pub identifier: String,
    pub model: String,
}

#[derive(Debug, Clone, Object)]
pub struct GroupReader {
    pub id: String,
    pub name: String,
    pub group_id: Uuid,
    pub group_path: String,
    pub group_name: InternationalizedString,
    /// Provider status; `unknown` when the reader is not visible to our merchant.
    pub status: String,
    pub device: Option<ReaderDevice>,
}

#[derive(Debug, Clone, Serialize, Object)]
pub struct CreateReader {
    /// Shown on the device when pairing.
    pub pairing_code: String,
    pub name: String,
}

#[derive(Debug, Clone, Object)]
pub struct UpdateReader {
    pub name: Option<String>,
    /// Moves the reader to another group (hand-over).
    pub group_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct ReaderRow {
    pub id: String,
    pub name: String,
    pub group_id: Uuid,
    pub group_path: String,
    pub group_name: InternationalizedString,
}

/// Fetches the readers of the merchant used by this service.
///
/// # Errors
///
/// Returns an error if the `transactions` service cannot be reached or rejects the request.
pub async fn fetch_remote(ctx: &ContextWrapper) -> MinilithResult<Vec<Reader>> {
    let response = ctx
        .transactions_get("/v0/readers")
        .send()
        .await
        .wrap_err_internal("failed to list readers")?;
    if !response.status().is_success() {
        return Err(minilith_errors::MinilithEndpointError::internal_error(
            "transactions rejected the reader list",
            response.text().await.unwrap_or_default(),
        ));
    }
    response
        .json()
        .await
        .wrap_err_internal("invalid reader list response")
}

/// Pairs a new reader.
///
/// # Errors
///
/// Returns an error if pairing fails (e.g. the pairing code expired).
pub async fn pair_remote(
    ctx: &ContextWrapper,
    pairing_code: &str,
    name: &str,
) -> MinilithResult<Reader> {
    let response = ctx
        .transactions_post("/v0/readers")
        .json(&CreateReader {
            pairing_code: pairing_code.to_owned(),
            name: name.to_owned(),
        })
        .send()
        .await
        .wrap_err_internal("failed to pair reader")?;
    if !response.status().is_success() {
        return Err(minilith_errors::MinilithEndpointError::bad_frontend_code(
            "SumUp rejected the pairing",
            response.text().await.unwrap_or_default(),
        ));
    }
    response
        .json()
        .await
        .wrap_err_internal("invalid reader response")
}

/// Renames a reader at the provider.
///
/// # Errors
///
/// Returns an error if the provider rejects the request.
pub async fn rename_remote(
    ctx: &ContextWrapper,
    id: &str,
    name: Option<&str>,
) -> MinilithResult<Reader> {
    let response = ctx
        .transactions_patch(format!("/v0/readers/{id}"))
        .json(&UpdateReaderBody {
            name: name.map(ToOwned::to_owned),
        })
        .send()
        .await
        .wrap_err_internal("failed to update reader")?;
    if !response.status().is_success() {
        return Err(minilith_errors::MinilithEndpointError::bad_frontend_code(
            "SumUp rejected the reader update",
            response.text().await.unwrap_or_default(),
        ));
    }
    response
        .json()
        .await
        .wrap_err_internal("invalid reader response")
}

#[derive(Serialize)]
struct UpdateReaderBody {
    name: Option<String>,
}

/// # Errors
///
/// DB errors.
pub async fn for_group(
    db: impl PgExecutor<'_>,
    ctx: &ContextWrapper,
    group_id: Uuid,
) -> MinilithResult<Vec<GroupReader>> {
    let rows = load_reader_rows(db, &[group_id]).await?;
    Ok(attach_status(ctx, rows).await)
}

/// # Errors
///
/// DB errors.
pub async fn for_activity(
    db: impl PgExecutor<'_>,
    ctx: &ContextWrapper,
    group_ids: &[Uuid],
) -> MinilithResult<Vec<GroupReader>> {
    let rows = load_reader_rows(db, group_ids).await?;
    Ok(attach_status(ctx, rows).await)
}

async fn load_reader_rows(
    db: impl PgExecutor<'_>,
    group_ids: &[Uuid],
) -> MinilithResult<Vec<ReaderRow>> {
    let rows = sqlx::query!(
        r#"select
            terminal.id,
            terminal.name,
            terminal.group_id,
            grp.path::text as "group_path!",
            grp.name as "group_name!: DIS"
        from physical_payment_terminals terminal
        inner join groups grp on grp.id = terminal.group_id
        where terminal.group_id = any($1)
        order by grp.path, terminal.name"#,
        group_ids,
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ReaderRow {
            id: row.id,
            name: row.name,
            group_id: row.group_id,
            group_path: row.group_path,
            group_name: row.group_name.0,
        })
        .collect())
}

async fn attach_status(ctx: &ContextWrapper, rows: Vec<ReaderRow>) -> Vec<GroupReader> {
    let remote = fetch_remote(ctx).await.unwrap_or_default();
    rows.into_iter()
        .map(|row| {
            let reader = remote.iter().find(|reader| reader.id == row.id);
            GroupReader {
                id: row.id,
                name: row.name,
                group_id: row.group_id,
                group_path: row.group_path,
                group_name: row.group_name,
                status: reader.map_or_else(|| "unknown".to_owned(), |reader| reader.status.clone()),
                device: reader.and_then(|reader| reader.device.clone()),
            }
        })
        .collect()
}

/// # Errors
///
/// DB errors.
pub async fn attach_local(
    db: impl PgExecutor<'_>,
    id: &str,
    name: &str,
    group_id: Uuid,
) -> MinilithResult<()> {
    sqlx::query!(
        r#"insert into physical_payment_terminals (id, name, group_id)
        values ($1, $2, $3)
        on conflict (id) do update set name = excluded.name, group_id = excluded.group_id"#,
        id,
        name,
        group_id,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// # Errors
///
/// DB errors.
pub async fn detach_local(db: impl PgExecutor<'_>, id: &str) -> MinilithResult<()> {
    sqlx::query!("delete from physical_payment_terminals where id = $1", id)
        .execute(db)
        .await?;
    Ok(())
}

/// # Errors
///
/// NOT FOUND, DB.
pub async fn reader_group(db: impl PgExecutor<'_>, id: &str) -> MinilithResult<Uuid> {
    sqlx::query_scalar!(
        "select group_id from physical_payment_terminals where id = $1",
        id
    )
    .fetch_optional(db)
    .await?
    .wrap_err_not_found()
}
