use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use clap::{Parser, ValueEnum};
use color_eyre::eyre::{Context as _, Result, bail, ensure};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::Value;
use sqlx::FromRow;
use sqlx::postgres::types::PgMoney;
use sqlx::postgres::{PgPool, PgPoolOptions};
use time::{OffsetDateTime, format_description::well_known::Iso8601};
use uuid::Uuid;

const SWISH_FEE_ORE: i64 = 300;
const DEFAULT_SWISH_API: &str = "https://cpc.getswish.net/swish-cpcapi/api";
const DEFAULT_STRIPE_API: &str = "https://api.stripe.com/v1";

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ProviderSelection {
    All,
    Both,
    Swish,
    Stripe,
    Free,
}

impl ProviderSelection {
    const fn includes_swish(self) -> bool {
        matches!(self, Self::All | Self::Both | Self::Swish)
    }

    const fn includes_stripe(self) -> bool {
        matches!(self, Self::All | Self::Both | Self::Stripe)
    }

    const fn includes_free(self) -> bool {
        matches!(self, Self::All | Self::Free)
    }

    const fn requires_all_paid_matches(self) -> bool {
        matches!(self, Self::All | Self::Both)
    }
}

#[derive(Debug, Parser)]
#[command(about = "Recover ticket transactions deleted from the transactions database")]
struct Args {
    #[arg(long, env = "FED_DATABASE_URL")]
    fed_database_url: String,
    #[arg(long, env = "TRANSACTIONS_DATABASE_URL")]
    transactions_database_url: String,
    #[arg(long)]
    client_id: Option<String>,
    #[arg(long)]
    callback_url: Option<String>,
    #[arg(long, value_enum, default_value = "all")]
    provider: ProviderSelection,
    /// Limit recovery to these transaction IDs. May be supplied more than once.
    #[arg(long = "transaction-id")]
    transaction_ids: Vec<Uuid>,
    /// Force a language when the original ware names cannot be inferred uniquely.
    #[arg(long)]
    language: Option<String>,
    /// Write the recovered rows. Without this flag, the command is a dry run.
    #[arg(long)]
    apply: bool,
    #[arg(long, default_value = DEFAULT_SWISH_API, hide = true)]
    swish_api: String,
    #[arg(long, default_value = DEFAULT_STRIPE_API, hide = true)]
    stripe_api: String,
}

#[derive(Debug, FromRow)]
struct ClientConfig {
    client_id: String,
    swish_cert: String,
    swish_key: String,
    swish_number: String,
    stripe_secret: Option<String>,
}

#[derive(Debug, FromRow)]
struct TicketRow {
    transaction_id: Uuid,
    purchased_ticket_id: Uuid,
    purchaser_id: String,
    activity_names: String,
    ticket_kind_names: String,
    price: PgMoney,
}

#[derive(Debug, FromRow)]
struct OptionRow {
    transaction_id: Uuid,
    purchased_ticket_id: Uuid,
    addon_names: String,
    option_names: String,
    price: PgMoney,
}

type Names = BTreeMap<String, String>;

#[derive(Clone, Debug)]
struct WareTemplate {
    names: Vec<Names>,
    indented: bool,
    amount: i64,
    tax: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct Ware {
    name: String,
    amount: i64,
    tax: f64,
}

#[derive(Debug)]
struct TicketTransaction {
    customer_id: String,
    wares: Vec<WareTemplate>,
}

impl TicketTransaction {
    fn total(&self) -> i64 {
        self.wares.iter().map(|ware| ware.amount).sum()
    }

    fn render(&self, language: &str) -> Vec<Ware> {
        self.wares
            .iter()
            .map(|ware| {
                let name = ware
                    .names
                    .iter()
                    .map(|names| resolve_name(names, language))
                    .collect::<Vec<_>>()
                    .join(" - ");
                Ware {
                    name: if ware.indented {
                        format!("    {name}")
                    } else {
                        name
                    },
                    amount: ware.amount,
                    tax: ware.tax,
                }
            })
            .collect()
    }

    fn candidate_wares(&self, forced_language: Option<&str>) -> Vec<Vec<Ware>> {
        let languages = forced_language.map_or_else(
            || {
                let mut languages = BTreeSet::from(["en".to_owned(), "sv".to_owned()]);
                for ware in &self.wares {
                    for names in &ware.names {
                        languages.extend(names.keys().cloned());
                    }
                }
                languages
            },
            |language| BTreeSet::from([language.to_owned()]),
        );

        let mut candidates = Vec::new();
        for language in languages {
            let wares = self.render(&language);
            if !candidates.contains(&wares) {
                candidates.push(wares);
            }
        }
        candidates
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwishPayment {
    id: Uuid,
    payment_reference: Option<String>,
    callback_identifier: Option<String>,
    amount: Value,
    currency: String,
    message: String,
    status: String,
    date_created: String,
    date_paid: Option<String>,
    payee_alias: String,
}

#[derive(Debug, Deserialize)]
struct StripeList<T> {
    data: Vec<T>,
    has_more: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct StripeSessionSummary {
    id: String,
    customer: Option<String>,
    client_reference_id: Option<String>,
    amount_total: Option<i64>,
    currency: Option<String>,
    payment_status: String,
}

#[derive(Debug, Deserialize)]
struct StripeSession {
    id: String,
    customer: Option<String>,
    client_reference_id: Option<String>,
    amount_total: Option<i64>,
    created: i64,
    currency: Option<String>,
    payment_status: String,
    line_items: StripeList<StripeLineItem>,
    payment_intent: StripePaymentIntent,
}

#[derive(Debug, Deserialize)]
struct StripeLineItem {
    description: Option<String>,
    amount_total: i64,
    currency: String,
    quantity: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct StripePaymentIntent {
    latest_charge: StripeCharge,
}

#[derive(Debug, Deserialize)]
struct StripeCharge {
    created: i64,
    refunded: bool,
    amount_refunded: i64,
    balance_transaction: StripeBalanceTransaction,
}

#[derive(Debug, Deserialize)]
struct StripeBalanceTransaction {
    fee: i64,
    currency: String,
}

#[derive(Debug)]
enum RecoveredProvider {
    Swish,
    Stripe { checkout_id: String },
    Free,
}

#[derive(Debug)]
struct RecoveredTransaction {
    id: Uuid,
    customer_id: String,
    provider: RecoveredProvider,
    payment_reference: String,
    callback_identifier: Uuid,
    created: OffsetDateTime,
    paid_at: Option<OffsetDateTime>,
    timeout: OffsetDateTime,
    fee: i64,
    wares: Vec<Ware>,
}

type StripeMatches = BTreeMap<Uuid, Vec<(String, Vec<Ware>)>>;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let _dotenv = dotenvy::dotenv();
    let args = Args::parse();

    let fed_db = connect(&args.fed_database_url).await?;
    let transactions_db = connect(&args.transactions_database_url).await?;
    let client = load_client(&transactions_db, args.client_id.as_deref()).await?;
    let callback_url = load_callback_url(
        &transactions_db,
        &client.client_id,
        args.callback_url.as_deref(),
    )
    .await?;
    let tickets = load_tickets(&fed_db, &args.transaction_ids).await?;
    let existing = load_existing_ids(&transactions_db, tickets.keys().copied()).await?;
    let missing = tickets
        .into_iter()
        .filter(|(id, _)| !existing.contains(id))
        .collect::<BTreeMap<_, _>>();

    println!(
        "found {} purchased transaction(s): {} already exist, {} need recovery",
        existing.len() + missing.len(),
        existing.len(),
        missing.len()
    );
    if missing.is_empty() {
        return Ok(());
    }

    let mut recovered = Vec::new();
    let mut paid_missing = BTreeMap::new();
    for (id, ticket) in missing {
        if ticket.total() == 0 {
            if args.provider.includes_free() {
                recovered.push(recover_free(id, ticket, args.language.as_deref())?);
            }
        } else {
            paid_missing.insert(id, ticket);
        }
    }

    let mut not_swish = BTreeMap::new();
    if args.provider.includes_swish() && !paid_missing.is_empty() {
        let swish_client = build_swish_client(&client)?;
        for (id, ticket) in paid_missing {
            match fetch_swish(&swish_client, &args.swish_api, id).await? {
                Some(payment) => recovered.push(recover_swish(
                    id,
                    ticket,
                    payment,
                    &client.swish_number,
                    args.language.as_deref(),
                )?),
                None => {
                    not_swish.insert(id, ticket);
                }
            }
        }
    } else {
        not_swish = paid_missing;
    }

    let mut unresolved = Vec::new();
    if args.provider.includes_stripe() && !not_swish.is_empty() {
        let (stripe_recovered, stripe_unresolved) = recover_stripe(
            &transactions_db,
            &client,
            &args.stripe_api,
            not_swish,
            args.language.as_deref(),
        )
        .await?;
        recovered.extend(stripe_recovered);
        if args.provider.requires_all_paid_matches() {
            unresolved.extend(stripe_unresolved);
        }
    } else if args.provider.requires_all_paid_matches() {
        unresolved.extend(not_swish.into_keys());
    }

    recovered.sort_unstable_by_key(|transaction| transaction.id);
    report_recovery(&recovered, &unresolved);

    if !unresolved.is_empty() {
        bail!(
            "{} transaction(s) could not be matched safely; no rows were written",
            unresolved.len()
        );
    }
    if !args.apply {
        println!("dry run complete; rerun with --apply to write these rows");
        return Ok(());
    }

    write_recovered(
        &transactions_db,
        &client.client_id,
        &callback_url,
        &recovered,
    )
    .await?;
    println!("recovered {} transaction(s)", recovered.len());
    Ok(())
}

fn report_recovery(recovered: &[RecoveredTransaction], unresolved: &[Uuid]) {
    for transaction in recovered {
        let provider = match transaction.provider {
            RecoveredProvider::Swish => "swish",
            RecoveredProvider::Stripe { .. } => "stripe",
            RecoveredProvider::Free => "free",
        };
        let amount = transaction.wares.iter().map(|ware| ware.amount).sum();
        println!(
            "recoverable: {} ({provider}, {} SEK, fee {} SEK, reference {})",
            transaction.id,
            format_ore(amount),
            format_ore(transaction.fee),
            transaction.payment_reference
        );
    }
    for id in unresolved {
        eprintln!("unresolved: {id}");
    }
}

fn format_ore(amount: i64) -> String {
    format!("{}.{:02}", amount / 100, (amount % 100).abs())
}

async fn connect(url: &str) -> Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(2)
        .connect(url)
        .await
        .wrap_err("failed to connect to PostgreSQL")
}

async fn load_client(db: &PgPool, requested: Option<&str>) -> Result<ClientConfig> {
    let clients = if let Some(client_id) = requested {
        sqlx::query_as::<_, ClientConfig>(
            "select client_id, swish_cert, swish_key, swish_number, stripe_secret
             from client_ids where client_id = $1",
        )
        .bind(client_id)
        .fetch_all(db)
        .await?
    } else {
        sqlx::query_as::<_, ClientConfig>(
            "select client_id, swish_cert, swish_key, swish_number, stripe_secret
             from client_ids order by client_id",
        )
        .fetch_all(db)
        .await?
    };
    ensure!(
        !clients.is_empty(),
        "no matching transactions client was found"
    );
    ensure!(
        clients.len() == 1,
        "multiple transactions clients exist; choose one with --client-id"
    );
    clients
        .into_iter()
        .next()
        .ok_or_else(|| color_eyre::eyre::eyre!("client disappeared"))
}

async fn load_callback_url(
    db: &PgPool,
    client_id: &str,
    requested: Option<&str>,
) -> Result<String> {
    if let Some(callback_url) = requested {
        return Ok(callback_url.to_owned());
    }
    let urls = sqlx::query_scalar::<_, String>(
        "select distinct callback_url_v1 from api_tokens
         where client_id = $1 order by callback_url_v1",
    )
    .bind(client_id)
    .fetch_all(db)
    .await?;
    ensure!(
        urls.len() == 1,
        "expected one callback URL for {client_id}; specify --callback-url"
    );
    urls.into_iter()
        .next()
        .ok_or_else(|| color_eyre::eyre::eyre!("callback URL disappeared"))
}

async fn load_tickets(
    db: &PgPool,
    selected_ids: &[Uuid],
) -> Result<BTreeMap<Uuid, TicketTransaction>> {
    let ticket_rows = sqlx::query_as::<_, TicketRow>(
        "select purchased.transaction_id, purchased.id as purchased_ticket_id,
                purchased.purchaser_id, activity.title::text as activity_names,
                kind.name::text as ticket_kind_names, kind.price
         from purchased_tickets purchased
         inner join ticket_kinds kind on kind.id = purchased.ticket_kind_id
         inner join activities activity on activity.id = kind.activity_id
         where cardinality($1::uuid[]) = 0 or purchased.transaction_id = any($1)
         order by purchased.transaction_id, purchased.id",
    )
    .bind(selected_ids)
    .fetch_all(db)
    .await?;
    let option_rows = sqlx::query_as::<_, OptionRow>(
        "select purchased.transaction_id, purchased.id as purchased_ticket_id,
                addon.name::text as addon_names, option.name::text as option_names,
                option.price
         from purchased_tickets purchased
         inner join purchased_ticket_addons chosen on chosen.ticket_id = purchased.id
         inner join ticket_addons addon on addon.id = chosen.addon_id
         cross join lateral unnest(chosen.selected_options)
             with ordinality as selected(option_idx, selection_order)
         inner join ticket_addon_options option
             on option.ticket_addon_id = addon.id and option.idx = selected.option_idx
         where cardinality($1::uuid[]) = 0 or purchased.transaction_id = any($1)
         order by purchased.transaction_id, purchased.id, addon.idx, selected.selection_order",
    )
    .bind(selected_ids)
    .fetch_all(db)
    .await?;

    let mut options_by_ticket: HashMap<Uuid, Vec<OptionRow>> = HashMap::new();
    for option in option_rows {
        options_by_ticket
            .entry(option.purchased_ticket_id)
            .or_default()
            .push(option);
    }

    let mut transactions = BTreeMap::new();
    for ticket in ticket_rows {
        let entry = transactions
            .entry(ticket.transaction_id)
            .or_insert_with(|| TicketTransaction {
                customer_id: ticket.purchaser_id.clone(),
                wares: Vec::new(),
            });
        ensure!(
            entry.customer_id == ticket.purchaser_id,
            "transaction {} has multiple purchasers",
            ticket.transaction_id
        );
        entry.wares.push(WareTemplate {
            names: vec![
                parse_names(&ticket.activity_names)?,
                parse_names(&ticket.ticket_kind_names)?,
            ],
            indented: false,
            amount: ticket.price.0,
            tax: 1.0,
        });
        for option in options_by_ticket
            .remove(&ticket.purchased_ticket_id)
            .unwrap_or_default()
        {
            ensure!(
                option.transaction_id == ticket.transaction_id,
                "addon transaction ID did not match its ticket"
            );
            entry.wares.push(WareTemplate {
                names: vec![
                    parse_names(&option.addon_names)?,
                    parse_names(&option.option_names)?,
                ],
                indented: true,
                amount: option.price.0,
                tax: 1.25,
            });
        }
    }
    ensure!(
        options_by_ticket.is_empty(),
        "found selected addon options without a purchased ticket"
    );
    ensure!(
        selected_ids.is_empty() || transactions.len() == selected_ids.len(),
        "one or more --transaction-id values did not identify a purchased ticket"
    );
    Ok(transactions)
}

async fn load_existing_ids(db: &PgPool, ids: impl Iterator<Item = Uuid>) -> Result<HashSet<Uuid>> {
    let ids = ids.collect::<Vec<_>>();
    Ok(
        sqlx::query_scalar::<_, Uuid>("select id from transactions where id = any($1)")
            .bind(ids)
            .fetch_all(db)
            .await?
            .into_iter()
            .collect(),
    )
}

fn parse_names(json: &str) -> Result<Names> {
    serde_json::from_str(json).wrap_err("internationalized name was not a string map")
}

fn resolve_name<'a>(names: &'a Names, language: &str) -> &'a str {
    names
        .get(language)
        .or_else(|| language.split('-').next().and_then(|base| names.get(base)))
        .or_else(|| names.get("en"))
        .or_else(|| names.get("sv"))
        .or_else(|| names.values().next())
        .map_or("", String::as_str)
}

fn build_swish_client(config: &ClientConfig) -> Result<reqwest::Client> {
    let identity = format!("{}\n{}", config.swish_key, config.swish_cert);
    reqwest::Client::builder()
        .identity(reqwest::Identity::from_pem(identity.as_bytes())?)
        .build()
        .wrap_err("failed to build the Swish client")
}

async fn fetch_swish(
    client: &reqwest::Client,
    api: &str,
    id: Uuid,
) -> Result<Option<SwishPayment>> {
    let response = client
        .get(format!(
            "{}/v1/paymentrequests/{}",
            api.trim_end_matches('/'),
            id.simple().encode_upper(&mut Uuid::encode_buffer())
        ))
        .send()
        .await
        .wrap_err_with(|| format!("failed to retrieve Swish payment {id}"))?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let status = response.status();
    let body = response.text().await?;
    ensure!(
        status.is_success(),
        "Swish returned {status} while retrieving {id}: {body}"
    );
    let payment = serde_json::from_str(&body)
        .wrap_err_with(|| format!("invalid Swish response while retrieving {id}"))?;
    Ok(Some(payment))
}

fn recover_swish(
    id: Uuid,
    ticket: TicketTransaction,
    payment: SwishPayment,
    expected_payee: &str,
    forced_language: Option<&str>,
) -> Result<RecoveredTransaction> {
    ensure!(
        payment.id == id,
        "Swish returned the wrong payment ID for {id}"
    );
    ensure!(payment.status == "PAID", "Swish payment {id} is not paid");
    ensure!(
        payment.currency.eq_ignore_ascii_case("SEK"),
        "Swish payment {id} is not in SEK"
    );
    ensure!(
        payment.payee_alias == expected_payee,
        "Swish payment {id} has the wrong payee"
    );
    ensure!(
        parse_ore(&payment.amount)? == ticket.total(),
        "Swish payment {id} amount differs from the ticket"
    );

    let candidate_wares = ticket.candidate_wares(forced_language);
    let wares = if forced_language.is_some() {
        let wares = candidate_wares
            .into_iter()
            .next()
            .ok_or_else(|| color_eyre::eyre::eyre!("forced ware language disappeared"))?;
        if swish_message(&wares).as_deref() != Some(payment.message.as_str()) {
            eprintln!(
                "warning: Swish message order/text for {id} differs from the forced ware language"
            );
        }
        wares
    } else {
        let matching_wares = candidate_wares
            .into_iter()
            .filter(|wares| swish_message(wares).as_deref() == Some(payment.message.as_str()))
            .collect::<Vec<_>>();
        ensure!(
            matching_wares.len() == 1,
            "Swish payment {id} matched {} ware-language variants; use --language if needed",
            matching_wares.len()
        );
        matching_wares
            .into_iter()
            .next()
            .ok_or_else(|| color_eyre::eyre::eyre!("ware match disappeared"))?
    };
    let created = OffsetDateTime::parse(&payment.date_created, &Iso8601::DEFAULT)?;
    let paid_at = OffsetDateTime::parse(
        payment
            .date_paid
            .as_deref()
            .ok_or_else(|| color_eyre::eyre::eyre!("paid Swish payment {id} had no datePaid"))?,
        &Iso8601::DEFAULT,
    )?;
    let payment_reference = payment.payment_reference.ok_or_else(|| {
        color_eyre::eyre::eyre!("paid Swish payment {id} had no paymentReference")
    })?;
    let callback_identifier = payment
        .callback_identifier
        .as_deref()
        .map(Uuid::parse_str)
        .transpose()?
        .unwrap_or_else(Uuid::nil);

    Ok(RecoveredTransaction {
        id,
        customer_id: ticket.customer_id,
        provider: RecoveredProvider::Swish,
        payment_reference,
        callback_identifier,
        created,
        paid_at: Some(paid_at),
        timeout: paid_at,
        fee: SWISH_FEE_ORE,
        wares,
    })
}

fn recover_free(
    id: Uuid,
    ticket: TicketTransaction,
    forced_language: Option<&str>,
) -> Result<RecoveredTransaction> {
    ensure!(
        ticket.total() == 0,
        "refusing to recover nonzero transaction {id} as free"
    );
    let candidate_wares = ticket.candidate_wares(forced_language);
    ensure!(
        candidate_wares.len() == 1,
        "free transaction {id} has {} possible ware-language variants; use --language",
        candidate_wares.len()
    );
    let wares = candidate_wares
        .into_iter()
        .next()
        .ok_or_else(|| color_eyre::eyre::eyre!("free ware language disappeared"))?;
    let recovered_at = OffsetDateTime::now_utc();

    Ok(RecoveredTransaction {
        id,
        customer_id: ticket.customer_id,
        provider: RecoveredProvider::Free,
        payment_reference: "free".to_owned(),
        callback_identifier: Uuid::nil(),
        created: recovered_at,
        paid_at: None,
        timeout: recovered_at,
        fee: 0,
        wares,
    })
}

fn parse_ore(value: &Value) -> Result<i64> {
    let value = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => bail!("payment amount was neither a string nor a number"),
    };
    let (kronor, ore) = value.split_once('.').unwrap_or((&value, ""));
    ensure!(
        ore.len() <= 2,
        "payment amount had fractions smaller than one ore"
    );
    let mut ore = ore.to_owned();
    while ore.len() < 2 {
        ore.push('0');
    }
    Ok(kronor.parse::<i64>()? * 100 + ore.parse::<i64>().unwrap_or(0))
}

fn swish_message(wares: &[Ware]) -> Option<String> {
    let mut message = wares
        .iter()
        .map(|ware| ware.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    message.retain(|character| {
        "!?(),.-:; åäöÅÄÖ".contains(character) || character.is_ascii_alphanumeric()
    });
    if message.len() > 50 {
        if !message.is_char_boundary(50) {
            return None;
        }
        message.truncate(50);
    }
    Some(message)
}

async fn recover_stripe(
    db: &PgPool,
    client_config: &ClientConfig,
    stripe_api: &str,
    tickets: BTreeMap<Uuid, TicketTransaction>,
    forced_language: Option<&str>,
) -> Result<(Vec<RecoveredTransaction>, Vec<Uuid>)> {
    let Some(secret) = client_config.stripe_secret.as_deref() else {
        return Ok((Vec::new(), tickets.into_keys().collect()));
    };
    let customer_ids = sqlx::query_as::<_, (String, String)>(
        "select customer_id, stripe_id from stripe_customers",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .collect::<HashMap<_, _>>();
    let used_references = sqlx::query_scalar::<_, String>(
        "select payment_reference from transactions
         where provider = 'stripe' and payment_reference is not null",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .collect::<HashSet<_>>();
    let client = reqwest::Client::new();
    let summaries = list_stripe_sessions(&client, stripe_api, secret).await?;
    let (mut matches, mut details) = match_stripe_sessions(
        &client,
        stripe_api,
        secret,
        &tickets,
        &customer_ids,
        &summaries,
        &used_references,
        forced_language,
    )
    .await?;

    let session_counts =
        matches
            .values()
            .flatten()
            .fold(HashMap::<String, usize>::new(), |mut counts, (id, _)| {
                *counts.entry(id.clone()).or_default() += 1;
                counts
            });
    let mut recovered = Vec::new();
    let mut unresolved = Vec::new();
    for (id, ticket) in tickets {
        let Some(candidates) = matches.remove(&id) else {
            unresolved.push(id);
            continue;
        };
        let uniquely_owned = candidates
            .first()
            .is_some_and(|(checkout_id, _)| session_counts.get(checkout_id).copied() == Some(1));
        if candidates.len() != 1 || !uniquely_owned {
            unresolved.push(id);
            continue;
        }
        let (checkout_id, wares) = candidates
            .into_iter()
            .next()
            .ok_or_else(|| color_eyre::eyre::eyre!("Stripe match disappeared"))?;
        let detail = details
            .remove(&checkout_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Stripe detail disappeared"))?;
        validate_stripe_detail(
            id,
            &checkout_id,
            customer_ids.get(&ticket.customer_id).map(String::as_str),
            &detail,
            ticket.total(),
        )?;
        let paid_at =
            OffsetDateTime::from_unix_timestamp(detail.payment_intent.latest_charge.created)?;
        let created = OffsetDateTime::from_unix_timestamp(detail.created)?;
        recovered.push(RecoveredTransaction {
            id,
            customer_id: ticket.customer_id,
            provider: RecoveredProvider::Stripe {
                checkout_id: checkout_id.clone(),
            },
            payment_reference: checkout_id,
            callback_identifier: id,
            created,
            paid_at: Some(paid_at),
            timeout: paid_at,
            fee: detail.payment_intent.latest_charge.balance_transaction.fee,
            wares,
        });
    }
    Ok((recovered, unresolved))
}

#[allow(
    clippy::too_many_arguments,
    reason = "the arguments are the explicit recovery inputs and provider snapshots"
)]
async fn match_stripe_sessions(
    client: &reqwest::Client,
    stripe_api: &str,
    secret: &str,
    tickets: &BTreeMap<Uuid, TicketTransaction>,
    customer_ids: &HashMap<String, String>,
    summaries: &[StripeSessionSummary],
    used_references: &HashSet<String>,
    forced_language: Option<&str>,
) -> Result<(StripeMatches, HashMap<String, StripeSession>)> {
    let mut details = HashMap::new();
    let mut matches = StripeMatches::new();
    for (id, ticket) in tickets {
        let stripe_customer = customer_ids.get(&ticket.customer_id);
        let candidates = ticket.candidate_wares(forced_language);
        let transaction_id = id.to_string();
        for summary in summaries.iter().filter(|summary| {
            !used_references.contains(&summary.id)
                && summary.payment_status == "paid"
                && summary.currency.as_deref() == Some("sek")
                && summary.amount_total == Some(ticket.total())
                && (summary.client_reference_id.as_deref() == Some(transaction_id.as_str())
                    || stripe_customer.is_some_and(|customer| {
                        summary.customer.as_deref() == Some(customer.as_str())
                    }))
        }) {
            if !details.contains_key(&summary.id) {
                let detail =
                    retrieve_stripe_session(client, stripe_api, secret, &summary.id).await?;
                details.insert(summary.id.clone(), detail);
            }
            let detail = details
                .get(&summary.id)
                .ok_or_else(|| color_eyre::eyre::eyre!("Stripe detail disappeared"))?;
            if stripe_charge_has_refund(&detail.payment_intent.latest_charge) {
                continue;
            }
            if let Some(wares) = candidates
                .iter()
                .find(|wares| stripe_line_items_match(detail, wares))
            {
                matches
                    .entry(*id)
                    .or_default()
                    .push((summary.id.clone(), wares.clone()));
            }
        }
    }
    Ok((matches, details))
}

const fn stripe_charge_has_refund(charge: &StripeCharge) -> bool {
    charge.refunded || charge.amount_refunded != 0
}

async fn list_stripe_sessions(
    client: &reqwest::Client,
    api: &str,
    secret: &str,
) -> Result<Vec<StripeSessionSummary>> {
    let mut sessions = Vec::new();
    let mut starting_after = None;
    loop {
        let mut request = client
            .get(format!("{}/checkout/sessions", api.trim_end_matches('/')))
            .bearer_auth(secret)
            .query(&[("limit", "100")]);
        if let Some(cursor) = starting_after.as_deref() {
            request = request.query(&[("starting_after", cursor)]);
        }
        let response = request.send().await?.error_for_status()?;
        let page = response.json::<StripeList<StripeSessionSummary>>().await?;
        let last_id = page.data.last().map(|session| session.id.clone());
        sessions.extend(page.data);
        if !page.has_more {
            break;
        }
        starting_after = Some(last_id.ok_or_else(|| {
            color_eyre::eyre::eyre!("Stripe returned has_more without any sessions")
        })?);
    }
    Ok(sessions)
}

async fn retrieve_stripe_session(
    client: &reqwest::Client,
    api: &str,
    secret: &str,
    id: &str,
) -> Result<StripeSession> {
    client
        .get(format!(
            "{}/checkout/sessions/{id}",
            api.trim_end_matches('/')
        ))
        .bearer_auth(secret)
        .query(&[
            ("expand[]", "line_items"),
            (
                "expand[]",
                "payment_intent.latest_charge.balance_transaction",
            ),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .wrap_err_with(|| format!("failed to retrieve Stripe Checkout Session {id}"))
}

fn stripe_line_items_match(session: &StripeSession, wares: &[Ware]) -> bool {
    if session.line_items.data.len() != wares.len()
        || session
            .line_items
            .data
            .iter()
            .any(|item| item.quantity != Some(1) || item.currency != "sek")
    {
        return false;
    }
    let mut actual = session
        .line_items
        .data
        .iter()
        .filter_map(|item| {
            item.description
                .as_ref()
                .map(|name| (name.as_str(), item.amount_total))
        })
        .collect::<Vec<_>>();
    let mut expected = wares
        .iter()
        .map(|ware| (ware.name.as_str(), ware.amount))
        .collect::<Vec<_>>();
    actual.sort_unstable();
    expected.sort_unstable();
    actual == expected
}

fn validate_stripe_detail(
    id: Uuid,
    checkout_id: &str,
    expected_customer: Option<&str>,
    session: &StripeSession,
    expected_total: i64,
) -> Result<()> {
    ensure!(
        session.id == checkout_id,
        "Stripe returned the wrong Checkout Session for {id}"
    );
    ensure!(
        session.payment_status == "paid",
        "Stripe session for {id} is not paid"
    );
    ensure!(
        session.currency.as_deref() == Some("sek"),
        "Stripe session for {id} is not in SEK"
    );
    ensure!(
        session.amount_total == Some(expected_total),
        "Stripe session amount differs for {id}"
    );
    ensure!(
        !session.line_items.has_more,
        "Stripe session for {id} has more than the expanded line items"
    );
    ensure!(
        session
            .payment_intent
            .latest_charge
            .balance_transaction
            .currency
            == "sek",
        "Stripe fee for {id} is not in SEK"
    );
    ensure!(
        !stripe_charge_has_refund(&session.payment_intent.latest_charge),
        "Stripe charge for {id} has been fully or partially refunded"
    );
    let transaction_id = id.to_string();
    ensure!(
        session.client_reference_id.as_deref() == Some(transaction_id.as_str())
            || expected_customer
                .is_some_and(|customer| { session.customer.as_deref() == Some(customer) }),
        "Stripe session detail no longer matches {id}"
    );
    Ok(())
}

async fn write_recovered(
    db: &PgPool,
    client_id: &str,
    callback_url: &str,
    recovered: &[RecoveredTransaction],
) -> Result<()> {
    let mut transaction = db.begin().await?;
    for recovered in recovered {
        let provider = match recovered.provider {
            RecoveredProvider::Swish => "swish",
            RecoveredProvider::Stripe { .. } => "stripe",
            RecoveredProvider::Free => "free",
        };
        let inserted = sqlx::query(
            "insert into transactions
             (id, customer_id, client_id, callback_url_v1, created, payment_reference,
              paid_at, timeout, provider, total_transaction_fee, callback_identifier)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9::provider, $10, $11)
             on conflict (id) do nothing",
        )
        .bind(recovered.id)
        .bind(&recovered.customer_id)
        .bind(client_id)
        .bind(callback_url)
        .bind(recovered.created)
        .bind(&recovered.payment_reference)
        .bind(recovered.paid_at)
        .bind(recovered.timeout)
        .bind(provider)
        .bind(PgMoney(recovered.fee))
        .bind(recovered.callback_identifier)
        .execute(&mut *transaction)
        .await?;
        ensure!(
            inserted.rows_affected() == 1,
            "transaction {} was inserted concurrently; rolled back all recovery writes",
            recovered.id
        );
        for (index, ware) in recovered.wares.iter().enumerate() {
            let index = i32::try_from(index)?;
            sqlx::query(
                "insert into transaction_wares
                 (idx, transaction_id, name, amount, currency, tax)
                 values ($1, $2, $3, $4, 'SEK', $5)",
            )
            .bind(index)
            .bind(recovered.id)
            .bind(&ware.name)
            .bind(PgMoney(ware.amount))
            .bind(ware.tax)
            .execute(&mut *transaction)
            .await?;
        }
        match &recovered.provider {
            RecoveredProvider::Swish => {
                sqlx::query(
                    "insert into fortnox_voucher_jobs (transaction_id)
                     select transactions.id from transactions
                     inner join client_ids using (client_id)
                     where transactions.id = $1 and client_ids.fortnox_client_id is not null
                     on conflict (transaction_id) do nothing",
                )
                .bind(recovered.id)
                .execute(&mut *transaction)
                .await?;
            }
            RecoveredProvider::Stripe { checkout_id } => {
                sqlx::query(
                    "insert into stripe_checkouts (transaction_id, stripe_id) values ($1, $2)",
                )
                .bind(recovered.id)
                .bind(checkout_id)
                .execute(&mut *transaction)
                .await?;
            }
            RecoveredProvider::Free => {}
        }
    }
    transaction.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Names, RecoveredProvider, StripeBalanceTransaction, StripeCharge, TicketTransaction, Ware,
        WareTemplate, parse_ore, recover_free, stripe_charge_has_refund, swish_message,
    };
    use uuid::Uuid;

    fn names(en: &str, sv: &str) -> Names {
        [
            ("en".to_owned(), en.to_owned()),
            ("sv".to_owned(), sv.to_owned()),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn renders_candidate_wares_without_duplicates() {
        let transaction = TicketTransaction {
            customer_id: "aa0000aa-s".to_owned(),
            wares: vec![WareTemplate {
                names: vec![names("Event", "Aktivitet"), names("Ticket", "Biljett")],
                indented: false,
                amount: 10_00,
                tax: 1.0,
            }],
        };
        let candidates = transaction.candidate_wares(None);
        assert_eq!(candidates.len(), 2, "English and Swedish names differ");
        assert!(
            candidates.contains(&vec![Ware {
                name: "Event - Ticket".to_owned(),
                amount: 10_00,
                tax: 1.0,
            }]),
            "English candidate should be rendered"
        );
    }

    #[test]
    fn recreates_swish_message_sanitizing_and_truncation() {
        let message = swish_message(&[Ware {
            name: "A really long activity with # characters - Biljett".to_owned(),
            amount: 10_00,
            tax: 1.0,
        }]);
        assert_eq!(
            message.as_deref(),
            Some("A really long activity with  characters - Biljett")
        );
    }

    #[test]
    fn parses_swish_amount_exactly_as_ore() {
        assert_eq!(parse_ore(&serde_json::json!("3.00")).ok(), Some(300));
        assert_eq!(parse_ore(&serde_json::json!(125.5)).ok(), Some(12_550));
    }

    #[test]
    fn recovers_zero_total_as_free() {
        let transaction = TicketTransaction {
            customer_id: "aa0000aa-s".to_owned(),
            wares: vec![WareTemplate {
                names: vec![names("Event", "Aktivitet"), names("Ticket", "Biljett")],
                indented: false,
                amount: 0,
                tax: 1.0,
            }],
        };

        let Ok(recovered) = recover_free(Uuid::nil(), transaction, Some("sv")) else {
            panic!("zero-total transaction should be recoverable as free");
        };
        assert!(matches!(recovered.provider, RecoveredProvider::Free));
        assert_eq!(recovered.payment_reference, "free");
        assert_eq!(recovered.fee, 0);
        assert_eq!(recovered.paid_at, None);
        assert_eq!(recovered.timeout, recovered.created);
        assert_eq!(recovered.wares[0].name, "Aktivitet - Biljett");
    }

    #[test]
    fn free_recovery_refuses_nonzero_total() {
        let transaction = TicketTransaction {
            customer_id: "aa0000aa-s".to_owned(),
            wares: vec![WareTemplate {
                names: vec![names("Event", "Aktivitet")],
                indented: false,
                amount: 1,
                tax: 1.0,
            }],
        };

        assert!(recover_free(Uuid::nil(), transaction, Some("sv")).is_err());
    }

    #[test]
    fn free_recovery_requires_ambiguous_language_to_be_selected() {
        let transaction = TicketTransaction {
            customer_id: "aa0000aa-s".to_owned(),
            wares: vec![WareTemplate {
                names: vec![names("Event", "Aktivitet")],
                indented: false,
                amount: 0,
                tax: 1.0,
            }],
        };

        assert!(recover_free(Uuid::nil(), transaction, None).is_err());
    }

    #[test]
    fn excludes_fully_and_partially_refunded_stripe_charges() {
        let charge = |refunded, amount_refunded| StripeCharge {
            created: 0,
            refunded,
            amount_refunded,
            balance_transaction: StripeBalanceTransaction {
                fee: 0,
                currency: "sek".to_owned(),
            },
        };

        assert!(!stripe_charge_has_refund(&charge(false, 0)));
        assert!(stripe_charge_has_refund(&charge(true, 10_00)));
        assert!(stripe_charge_has_refund(&charge(false, 1)));
    }
}
