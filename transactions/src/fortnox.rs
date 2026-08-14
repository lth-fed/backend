use std::collections::BTreeMap;
use std::time::Duration;

use minilith_errors::{AlertLevel, MinilithErrorResultExt as _, MinilithResult, alert};
use reqwest::StatusCode;
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use sqlx::postgres::types::PgMoney;
use time::OffsetDateTime;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::Provider;
use crate::api::{Currency, Ware};
use crate::context::Context;
use crate::receipt;

const TOKEN_URL: &str = "https://apps.fortnox.se/oauth-v1/token";
const API_URL: &str = "https://api.fortnox.se/3";
const MAX_ATTEMPTS: i32 = 8;

#[derive(Debug, FromRow)]
struct Job {
    transaction_id: Uuid,
    attempts: i32,
    voucher_series: Option<String>,
    voucher_number: Option<i32>,
    voucher_year: Option<i32>,
    file_id: Option<String>,
}

#[derive(FromRow)]
struct TransactionData {
    id: Uuid,
    client_id: String,
    customer_id: Option<String>,
    payment_reference: String,
    transaction_date: String,
    merchant_name: String,
    merchant_email: String,
    merchant_address: String,
    merchant_org_id: String,
    merchant_svg_icon: Option<String>,
    fortnox_client_id: String,
    fortnox_client_secret: String,
    fortnox_tenant_id: String,
    fortnox_voucher_series: String,
    fortnox_bank_account: i32,
}

#[derive(Debug, FromRow)]
struct WareRow {
    name: String,
    amount: PgMoney,
    tax: f64,
}

#[derive(Debug, FromRow)]
struct TaxAccount {
    vat_basis_points: i32,
    revenue_account: i32,
    vat_account: Option<i32>,
}

#[derive(Debug)]
enum JobFailure {
    Retry(String),
    ManualReview(String),
}

impl JobFailure {
    fn retry(message: impl Into<String>) -> Self {
        Self::Retry(message.into())
    }

    fn manual(message: impl Into<String>) -> Self {
        Self::ManualReview(message.into())
    }
}

#[derive(Debug, Serialize)]
struct VoucherEnvelope {
    #[serde(rename = "Voucher")]
    voucher: Voucher,
}

#[derive(Debug, Serialize)]
struct Voucher {
    #[serde(rename = "Comments")]
    comments: String,
    #[serde(rename = "Description")]
    description: String,
    #[serde(rename = "TransactionDate")]
    transaction_date: String,
    #[serde(rename = "VoucherRows")]
    rows: VoucherRows,
    #[serde(rename = "VoucherSeries")]
    series: String,
}

#[derive(Debug, Serialize)]
struct VoucherRows {
    #[serde(rename = "VoucherRow")]
    rows: Vec<VoucherRow>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct VoucherRow {
    #[serde(rename = "Account")]
    account: i32,
    #[serde(rename = "Credit")]
    credit: String,
    #[serde(rename = "Debit")]
    debit: String,
    #[serde(rename = "TransactionInformation")]
    transaction_information: String,
}

#[derive(Debug, Deserialize)]
struct AccessToken {
    access_token: String,
}

#[derive(Debug, Serialize)]
struct FileConnectionEnvelope<'a> {
    #[serde(rename = "VoucherFileConnection")]
    connection: FileConnection<'a>,
}

#[derive(Debug, Serialize)]
struct FileConnection<'a> {
    #[serde(rename = "FileId")]
    file_id: &'a str,
    #[serde(rename = "VoucherNumber")]
    voucher_number: String,
    #[serde(rename = "VoucherSeries")]
    voucher_series: &'a str,
}

fn response_body(body: &str) -> String {
    body.chars().take(2_000).collect()
}

fn cents(amount: i64) -> String {
    format!("{}.{:02}", amount / 100, amount % 100)
}

fn vat_basis_points(tax_multiplier: f64) -> Result<i32, JobFailure> {
    if !tax_multiplier.is_finite() || tax_multiplier < 1.0 {
        return Err(JobFailure::manual(format!(
            "invalid tax multiplier {tax_multiplier}"
        )));
    }
    let basis_points = (tax_multiplier - 1.0) * 10_000.0;
    let rounded = basis_points.round();
    if (basis_points - rounded).abs() > 0.000_1 || rounded > f64::from(i32::MAX - 10_000) {
        return Err(JobFailure::manual(format!(
            "tax multiplier {tax_multiplier} cannot be represented as basis points"
        )));
    }
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the value was range checked and rounded above"
    )]
    Ok(rounded as i32)
}

fn rounded_net(gross: i64, vat_basis_points: i32) -> Result<i64, JobFailure> {
    let divisor = 10_000_i64 + i64::from(vat_basis_points);
    gross
        .checked_mul(10_000)
        .and_then(|value| value.checked_add(divisor / 2))
        .map(|value| value / divisor)
        .ok_or_else(|| JobFailure::manual("ware amount is too large for Fortnox accounting"))
}

fn voucher_rows(
    transaction_id: Uuid,
    bank_account: i32,
    wares: &[WareRow],
    accounts: &[TaxAccount],
) -> Result<Vec<VoucherRow>, JobFailure> {
    let accounts = accounts
        .iter()
        .map(|account| (account.vat_basis_points, account))
        .collect::<BTreeMap<_, _>>();
    let mut credits = BTreeMap::<i32, i64>::new();
    let mut total = 0_i64;

    for ware in wares {
        let gross = ware.amount.0;
        if gross < 0 {
            return Err(JobFailure::manual("a Swish ware has a negative amount"));
        }
        let rate = vat_basis_points(ware.tax)?;
        let account = accounts.get(&rate).ok_or_else(|| {
            JobFailure::manual(format!("no Fortnox account mapping for VAT rate {rate} bp"))
        })?;
        let net = rounded_net(gross, rate)?;
        let vat = gross - net;
        let revenue = credits.entry(account.revenue_account).or_default();
        *revenue = revenue
            .checked_add(net)
            .ok_or_else(|| JobFailure::manual("revenue total overflowed"))?;
        if vat > 0 {
            let vat_account = account.vat_account.ok_or_else(|| {
                JobFailure::manual(format!("no Fortnox VAT account for VAT rate {rate} bp"))
            })?;
            let vat_total = credits.entry(vat_account).or_default();
            *vat_total = vat_total
                .checked_add(vat)
                .ok_or_else(|| JobFailure::manual("VAT total overflowed"))?;
        }
        total = total
            .checked_add(gross)
            .ok_or_else(|| JobFailure::manual("voucher total overflowed"))?;
    }

    let information = format!("Teknologappen {transaction_id}");
    let mut rows = vec![VoucherRow {
        account: bank_account,
        credit: cents(0),
        debit: cents(total),
        transaction_information: information.clone(),
    }];
    rows.extend(credits.into_iter().map(|(account, credit)| VoucherRow {
        account,
        credit: cents(credit),
        debit: cents(0),
        transaction_information: information.clone(),
    }));
    Ok(rows)
}

async fn access_token(
    http: &reqwest::Client,
    transaction: &TransactionData,
) -> Result<String, JobFailure> {
    let response = http
        .post(TOKEN_URL)
        .basic_auth(
            &transaction.fortnox_client_id,
            Some(&transaction.fortnox_client_secret),
        )
        .header("TenantId", &transaction.fortnox_tenant_id)
        .form(&[("grant_type", "client_credentials")])
        .send()
        .await
        .map_err(|error| JobFailure::retry(format!("Fortnox OAuth request failed: {error}")))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| JobFailure::retry(format!("Fortnox OAuth response failed: {error}")))?;
    if !status.is_success() {
        let message = format!("Fortnox OAuth returned {status}: {}", response_body(&body));
        return if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            Err(JobFailure::retry(message))
        } else {
            Err(JobFailure::manual(message))
        };
    }
    serde_json::from_str::<AccessToken>(&body)
        .map(|token| token.access_token)
        .map_err(|error| JobFailure::retry(format!("invalid Fortnox OAuth response: {error}")))
}

fn value_i32(value: &Value, field: &str) -> Result<i32, JobFailure> {
    let value = value
        .get("Voucher")
        .and_then(|voucher| voucher.get(field))
        .ok_or_else(|| JobFailure::manual(format!("Fortnox omitted Voucher.{field}")))?;
    let number = value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        .ok_or_else(|| JobFailure::manual(format!("invalid Fortnox Voucher.{field}")))?;
    i32::try_from(number)
        .map_err(|_| JobFailure::manual(format!("Fortnox Voucher.{field} is out of range")))
}

fn value_string(value: &Value, field: &str) -> Result<String, JobFailure> {
    value
        .get("Voucher")
        .and_then(|voucher| voucher.get(field))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| JobFailure::manual(format!("Fortnox omitted Voucher.{field}")))
}

async fn create_voucher(
    http: &reqwest::Client,
    token: &str,
    voucher: Voucher,
) -> Result<(String, i32, i32), JobFailure> {
    let response = http
        .post(format!("{API_URL}/vouchers"))
        .bearer_auth(token)
        .json(&VoucherEnvelope { voucher })
        .send()
        .await
        .map_err(|error| {
            JobFailure::manual(format!(
                "ambiguous Fortnox voucher request failure; check Fortnox before retrying: {error}"
            ))
        })?;
    let status = response.status();
    let body = response.text().await.map_err(|error| {
        JobFailure::manual(format!(
            "Fortnox created a voucher but its response could not be read; check Fortnox: {error}"
        ))
    })?;
    if !status.is_success() {
        let message = format!(
            "Fortnox voucher creation returned {status}: {}",
            response_body(&body)
        );
        return if status == StatusCode::TOO_MANY_REQUESTS {
            Err(JobFailure::retry(message))
        } else {
            Err(JobFailure::manual(message))
        };
    }
    let response: Value = serde_json::from_str(&body).map_err(|error| {
        JobFailure::manual(format!(
            "Fortnox created a voucher but returned invalid JSON; check Fortnox: {error}"
        ))
    })?;
    Ok((
        value_string(&response, "VoucherSeries")?,
        value_i32(&response, "VoucherNumber")?,
        value_i32(&response, "Year")?,
    ))
}

async fn upload_receipt(
    http: &reqwest::Client,
    token: &str,
    transaction_id: Uuid,
    pdf: Vec<u8>,
) -> Result<String, JobFailure> {
    let part = Part::bytes(pdf)
        .file_name(format!("{transaction_id}.pdf"))
        .mime_str("application/pdf")
        .map_err(|error| JobFailure::manual(format!("invalid receipt MIME type: {error}")))?;
    let response = http
        .post(format!("{API_URL}/inbox"))
        .query(&[("path", "inbox_v")])
        .bearer_auth(token)
        .multipart(Form::new().part("file", part))
        .send()
        .await
        .map_err(|error| {
            JobFailure::manual(format!(
                "ambiguous Fortnox receipt upload failure; check the inbox before retrying: {error}"
            ))
        })?;
    let status = response.status();
    let body = response.text().await.map_err(|error| {
        JobFailure::manual(format!(
            "Fortnox uploaded a receipt but its response could not be read; check the inbox: {error}"
        ))
    })?;
    if !status.is_success() {
        let message = format!(
            "Fortnox receipt upload returned {status}: {}",
            response_body(&body)
        );
        return if status == StatusCode::TOO_MANY_REQUESTS {
            Err(JobFailure::retry(message))
        } else {
            Err(JobFailure::manual(message))
        };
    }
    let response: Value = serde_json::from_str(&body).map_err(|error| {
        JobFailure::manual(format!(
            "Fortnox uploaded a receipt but returned invalid JSON; check the inbox: {error}"
        ))
    })?;
    response
        .get("File")
        .and_then(|file| file.get("Id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| JobFailure::manual("Fortnox omitted File.Id after uploading the receipt"))
}

async fn connect_receipt(
    http: &reqwest::Client,
    token: &str,
    file_id: &str,
    voucher_series: &str,
    voucher_number: i32,
    voucher_year: i32,
) -> Result<(), JobFailure> {
    let existing = http
        .get(format!("{API_URL}/voucherfileconnections/{file_id}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| {
            JobFailure::retry(format!("Fortnox file connection check failed: {error}"))
        })?;
    if existing.status().is_success() {
        return Ok(());
    }
    if existing.status() != StatusCode::NOT_FOUND {
        let status = existing.status();
        let body = existing.text().await.unwrap_or_default();
        let message = format!(
            "Fortnox file connection check returned {status}: {}",
            response_body(&body)
        );
        return if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            Err(JobFailure::retry(message))
        } else {
            Err(JobFailure::manual(message))
        };
    }

    let body = FileConnectionEnvelope {
        connection: FileConnection {
            file_id,
            voucher_number: voucher_number.to_string(),
            voucher_series,
        },
    };
    let response = http
        .post(format!("{API_URL}/voucherfileconnections"))
        .query(&[("financialyear", voucher_year)])
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|error| {
            JobFailure::retry(format!(
                "Fortnox file connection request failed; it will be checked before retry: {error}"
            ))
        })?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let response = response.text().await.unwrap_or_default();
    let message = format!(
        "Fortnox file connection returned {status}: {}",
        response_body(&response)
    );
    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        Err(JobFailure::retry(message))
    } else {
        Err(JobFailure::manual(message))
    }
}

async fn transaction_data(ctx: &Context, id: Uuid) -> Result<TransactionData, JobFailure> {
    sqlx::query_as::<_, TransactionData>(
        "select t.id, t.client_id, t.customer_id, t.payment_reference,
            to_char(
                coalesce(t.paid_at, t.created) at time zone 'Europe/Stockholm',
                'YYYY-MM-DD'
            ) as transaction_date,
            c.name as merchant_name, c.email as merchant_email,
            c.address as merchant_address, c.organization_number as merchant_org_id,
            c.svg_icon as merchant_svg_icon,
            c.fortnox_client_id, c.fortnox_client_secret, c.fortnox_tenant_id,
            c.fortnox_voucher_series, c.fortnox_bank_account
        from transactions t
        inner join client_ids c on c.client_id = t.client_id
        where t.id = $1 and t.provider = 'swish' and t.payment_reference is not null",
    )
    .bind(id)
    .fetch_optional(&ctx.db)
    .await
    .map_err(|error| JobFailure::retry(format!("loading Fortnox transaction failed: {error}")))?
    .ok_or_else(|| JobFailure::manual("paid Swish transaction or Fortnox configuration missing"))
}

async fn wares(ctx: &Context, id: Uuid) -> Result<Vec<WareRow>, JobFailure> {
    sqlx::query_as::<_, WareRow>(
        "select name, amount, tax from transaction_wares
        where transaction_id = $1 order by idx",
    )
    .bind(id)
    .fetch_all(&ctx.db)
    .await
    .map_err(|error| JobFailure::retry(format!("loading Fortnox wares failed: {error}")))
}

async fn tax_accounts(ctx: &Context, client_id: &str) -> Result<Vec<TaxAccount>, JobFailure> {
    sqlx::query_as::<_, TaxAccount>(
        "select vat_basis_points, revenue_account, vat_account
        from fortnox_tax_accounts where client_id = $1",
    )
    .bind(client_id)
    .fetch_all(&ctx.db)
    .await
    .map_err(|error| JobFailure::retry(format!("loading Fortnox account mappings failed: {error}")))
}

async fn persist_voucher(
    ctx: &Context,
    transaction_id: Uuid,
    series: &str,
    number: i32,
    year: i32,
) -> Result<(), JobFailure> {
    sqlx::query(
        "update fortnox_voucher_jobs
        set voucher_series = $2, voucher_number = $3, voucher_year = $4
        where transaction_id = $1 and state = 'processing'",
    )
    .bind(transaction_id)
    .bind(series)
    .bind(number)
    .bind(year)
    .execute(&ctx.db)
    .await
    .map_err(|error| {
        JobFailure::manual(format!(
            "Fortnox voucher {series}{number} was created but could not be saved locally: {error}"
        ))
    })?;
    Ok(())
}

async fn persist_file(
    ctx: &Context,
    transaction_id: Uuid,
    file_id: &str,
) -> Result<(), JobFailure> {
    sqlx::query(
        "update fortnox_voucher_jobs set file_id = $2
        where transaction_id = $1 and state = 'processing'",
    )
    .bind(transaction_id)
    .bind(file_id)
    .execute(&ctx.db)
    .await
    .map_err(|error| {
        JobFailure::manual(format!(
            "Fortnox file {file_id} was uploaded but could not be saved locally: {error}"
        ))
    })?;
    Ok(())
}

async fn run_job(ctx: &Context, job: &Job) -> Result<(), JobFailure> {
    let transaction = transaction_data(ctx, job.transaction_id).await?;
    let wares = wares(ctx, job.transaction_id).await?;
    let accounts = tax_accounts(ctx, &transaction.client_id).await?;
    let rows = voucher_rows(
        job.transaction_id,
        transaction.fortnox_bank_account,
        &wares,
        &accounts,
    )?;
    let receipt_wares = wares
        .iter()
        .map(|ware| Ware {
            name: ware.name.clone(),
            amount: ware.amount.0,
            tax: ware.tax,
            currency: Currency::Sek,
        })
        .collect();
    let date = transaction.transaction_date.clone();
    let pdf = receipt::compile(
        &ctx.typst_world,
        &receipt::Data {
            language: receipt::Language::Swedish,
            transaction_id: transaction.id.to_string(),
            purchase_date: date.clone(),
            provider: Provider::Swish,
            payment_reference: transaction.payment_reference.clone(),
            refund_reference: None,
            wares: receipt_wares,
            customer_name: None,
            customer_id: transaction.customer_id.clone(),
            merchant_id: transaction.client_id.clone(),
            merchant_name: transaction.merchant_name.clone(),
            merchant_org_id: transaction.merchant_org_id.clone(),
            merchant_email: transaction.merchant_email.clone(),
            merchant_address: transaction.merchant_address.clone(),
            merchant_svg_icon: transaction.merchant_svg_icon.clone(),
        },
    )
    .map_err(|error| JobFailure::manual(format!("compiling Fortnox receipt failed: {error:?}")))?;

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| {
            JobFailure::retry(format!("building Fortnox HTTP client failed: {error}"))
        })?;
    let token = access_token(&http, &transaction).await?;

    let (series, number, year) = match (
        job.voucher_series.as_deref(),
        job.voucher_number,
        job.voucher_year,
    ) {
        (Some(series), Some(number), Some(year)) => (series.to_owned(), number, year),
        (None, None, None) => {
            let voucher = Voucher {
                comments: format!("Swish transaction {}", transaction.id),
                description: "Swish-försäljning via Teknologappen".to_owned(),
                transaction_date: date,
                rows: VoucherRows { rows },
                series: transaction.fortnox_voucher_series,
            };
            let identity = create_voucher(&http, &token, voucher).await?;
            persist_voucher(ctx, job.transaction_id, &identity.0, identity.1, identity.2).await?;
            identity
        }
        _ => {
            return Err(JobFailure::manual(
                "incomplete local Fortnox voucher identity",
            ));
        }
    };

    let file_id = if let Some(file_id) = &job.file_id {
        file_id.clone()
    } else {
        let file_id = upload_receipt(&http, &token, job.transaction_id, pdf).await?;
        persist_file(ctx, job.transaction_id, &file_id).await?;
        file_id
    };
    connect_receipt(&http, &token, &file_id, &series, number, year).await?;
    Ok(())
}

async fn claim_job(ctx: &Context) -> MinilithResult<Option<Job>> {
    sqlx::query_as::<_, Job>(
        "update fortnox_voucher_jobs
        set state = 'processing', attempts = attempts + 1, started_at = now(), last_error = null
        where transaction_id = (
            select transaction_id from fortnox_voucher_jobs
            where state = 'pending' and next_attempt_at <= now()
            order by next_attempt_at, transaction_id
            for update skip locked
            limit 1
        )
        returning transaction_id, attempts, voucher_series, voucher_number, voucher_year, file_id",
    )
    .fetch_optional(&ctx.db)
    .await
    .wrap_err_internal("l2: claiming a Fortnox voucher job failed")
}

async fn complete_job(ctx: &Context, transaction_id: Uuid) -> MinilithResult<()> {
    sqlx::query(
        "update fortnox_voucher_jobs
        set state = 'completed', completed_at = now(), started_at = null, last_error = null
        where transaction_id = $1 and state = 'processing'",
    )
    .bind(transaction_id)
    .execute(&ctx.db)
    .await
    .wrap_err_internal("l2: completing a Fortnox voucher job failed")?;
    Ok(())
}

async fn retry_job(ctx: &Context, job: &Job, message: &str) -> MinilithResult<()> {
    if job.attempts >= MAX_ATTEMPTS {
        return manual_review_job(
            ctx,
            job.transaction_id,
            &format!("Fortnox job exhausted {MAX_ATTEMPTS} attempts: {message}"),
        )
        .await;
    }
    let exponent = u32::try_from(job.attempts.min(7)).unwrap_or(7);
    let delay = i64::from(30 * 2_i32.pow(exponent));
    let next_attempt = OffsetDateTime::now_utc() + time::Duration::seconds(delay);
    sqlx::query(
        "update fortnox_voucher_jobs
        set state = 'pending', next_attempt_at = $2, started_at = null, last_error = $3
        where transaction_id = $1 and state = 'processing'",
    )
    .bind(job.transaction_id)
    .bind(next_attempt)
    .bind(message)
    .execute(&ctx.db)
    .await
    .wrap_err_internal("l2: rescheduling a Fortnox voucher job failed")?;
    warn!(transaction_id = %job.transaction_id, attempts = job.attempts, %message, "Fortnox job will retry");
    Ok(())
}

async fn manual_review_job(
    ctx: &Context,
    transaction_id: Uuid,
    message: &str,
) -> MinilithResult<()> {
    sqlx::query(
        "update fortnox_voucher_jobs
        set state = 'manual_review', started_at = null, last_error = $2
        where transaction_id = $1 and state = 'processing'",
    )
    .bind(transaction_id)
    .bind(message)
    .execute(&ctx.db)
    .await
    .wrap_err_internal("l2: marking a Fortnox voucher job for manual review failed")?;
    error!(%transaction_id, %message, "Fortnox voucher requires manual review");
    alert(
        AlertLevel::L2,
        format!(
            "Fortnox voucher for transaction {transaction_id} requires manual review: {message}"
        ),
    );
    Ok(())
}

/// Processes at most one queued voucher. Safe to run concurrently across service instances.
///
/// # Errors
///
/// Returns database errors while claiming or updating job state.
pub async fn process_next_job(ctx: &Context) -> MinilithResult<()> {
    let Some(job) = claim_job(ctx).await? else {
        return Ok(());
    };
    match run_job(ctx, &job).await {
        Ok(()) => {
            complete_job(ctx, job.transaction_id).await?;
            info!(transaction_id = %job.transaction_id, "Fortnox voucher and receipt created");
        }
        Err(JobFailure::Retry(message)) => retry_job(ctx, &job, &message).await?,
        Err(JobFailure::ManualReview(message)) => {
            manual_review_job(ctx, job.transaction_id, &message).await?;
        }
    }
    Ok(())
}

/// Moves abandoned in-flight jobs to manual review rather than risking duplicate vouchers.
///
/// # Errors
///
/// Returns database errors.
pub async fn recover_stale_jobs(ctx: &Context) -> MinilithResult<()> {
    let rows = sqlx::query_scalar::<_, Uuid>(
        "update fortnox_voucher_jobs
        set state = 'manual_review', started_at = null,
            last_error = 'worker stopped during an API operation; check Fortnox before retrying'
        where state = 'processing' and started_at < now() - interval '10 minutes'
        returning transaction_id",
    )
    .fetch_all(&ctx.db)
    .await
    .wrap_err_internal("l2: recovering stale Fortnox jobs failed")?;
    if !rows.is_empty() {
        alert(
            AlertLevel::L2,
            format!(
                "{} Fortnox voucher job(s) stopped mid-operation and require manual review: {}",
                rows.len(),
                rows.iter()
                    .map(Uuid::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sqlx::postgres::types::PgMoney;
    use uuid::Uuid;

    use super::{TaxAccount, VoucherRow, WareRow, voucher_rows};

    #[test]
    fn builds_balanced_rows_with_mixed_vat_rates() {
        let transaction_id = Uuid::nil();
        let rows = voucher_rows(
            transaction_id,
            1930,
            &[
                WareRow {
                    name: "Lunch".to_owned(),
                    amount: PgMoney(12_500),
                    tax: 1.25,
                },
                WareRow {
                    name: "Ticket".to_owned(),
                    amount: PgMoney(10_600),
                    tax: 1.06,
                },
            ],
            &[
                TaxAccount {
                    vat_basis_points: 2500,
                    revenue_account: 3001,
                    vat_account: Some(2611),
                },
                TaxAccount {
                    vat_basis_points: 600,
                    revenue_account: 3003,
                    vat_account: Some(2631),
                },
            ],
        )
        .expect("valid mappings should build voucher rows");

        assert_eq!(
            rows,
            vec![
                VoucherRow {
                    account: 1930,
                    credit: "0.00".to_owned(),
                    debit: "231.00".to_owned(),
                    transaction_information: format!("Teknologappen {transaction_id}"),
                },
                VoucherRow {
                    account: 2611,
                    credit: "25.00".to_owned(),
                    debit: "0.00".to_owned(),
                    transaction_information: format!("Teknologappen {transaction_id}"),
                },
                VoucherRow {
                    account: 2631,
                    credit: "6.00".to_owned(),
                    debit: "0.00".to_owned(),
                    transaction_information: format!("Teknologappen {transaction_id}"),
                },
                VoucherRow {
                    account: 3001,
                    credit: "100.00".to_owned(),
                    debit: "0.00".to_owned(),
                    transaction_information: format!("Teknologappen {transaction_id}"),
                },
                VoucherRow {
                    account: 3003,
                    credit: "100.00".to_owned(),
                    debit: "0.00".to_owned(),
                    transaction_information: format!("Teknologappen {transaction_id}"),
                },
            ]
        );
    }

    #[test]
    fn rejects_missing_vat_mapping() {
        let error = voucher_rows(
            Uuid::nil(),
            1930,
            &[WareRow {
                name: "Lunch".to_owned(),
                amount: PgMoney(10_000),
                tax: 1.12,
            }],
            &[],
        )
        .expect_err("missing mapping must stop bookkeeping");
        assert!(
            format!("{error:?}").contains("1200"),
            "error should identify the missing VAT rate"
        );
    }
}
