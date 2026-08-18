mod api;
mod auth;
mod error;
mod stats;

use std::io::Write as _;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration as StdDuration;

use api::{ApiClient, BuyFreeOutcome, PurchaseStatus, QueueEntry, QueuePoll, TicketKind};
use auth::Session;
use clap::Parser;
use error::{Result, ResultContext as _, error};
use reqwest::{Client, Url};
use sqlx::postgres::PgPoolOptions;
use stats::{SampleStatus, Stats};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::task::JoinSet;
use uuid::Uuid;

const DEFAULT_CLIENTS: usize = 1_001;
const QUEUE_POLL_INTERVAL: StdDuration = StdDuration::from_secs(15);
const RETRY_INTERVAL: StdDuration = StdDuration::from_secs(1);
const PURCHASE_FLOW_RETRY_INTERVAL: StdDuration = StdDuration::from_secs(3);

#[derive(Debug, Parser)]
#[command(about = "Load-test Minilith's free-ticket queue with real test-provider accounts")]
struct Args {
    /// Free ticket kind to queue for and purchase.
    ticket_kind_id: Uuid,
    /// Existing group path granted to every generated account, for example `tlth.e`.
    group_path: String,
    /// Number of concurrent frontend clients to simulate.
    #[arg(long, default_value_t = DEFAULT_CLIENTS)]
    clients: usize,
    /// Minilith API base URL.
    #[arg(long, default_value = "http://localhost:8000/v0/")]
    api_url: Url,
    /// fed-auth base URL.
    #[arg(long, default_value = "http://localhost:8001/")]
    auth_url: Url,
    /// `PostgreSQL` connection used only to grant group memberships.
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,
    /// Prefix before each numeric test identity. Defaults to a fresh value per run.
    #[arg(long)]
    user_prefix: Option<String>,
}

#[derive(Clone, Copy, Debug)]
enum ClientOutcome {
    Purchased,
    SoldOut,
}

#[derive(Clone, Debug)]
struct RunContext {
    api: ApiClient,
    stats: Arc<Stats>,
    sample: SampleStatus,
    ticket_kind_id: Uuid,
}

#[tokio::main]
async fn main() -> ExitCode {
    drop(dotenvy::dotenv());
    match run(Args::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(source) => {
            eprintln!("load test failed: {source}");
            ExitCode::FAILURE
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "linear orchestration is easier to audit in one place"
)]
async fn run(mut args: Args) -> Result<()> {
    if args.clients == 0 {
        return Err(error("--clients must be greater than zero"));
    }
    ensure_trailing_slash(&mut args.api_url);
    ensure_trailing_slash(&mut args.auth_url);
    let callback_url = args
        .api_url
        .join("user/auth-callback/v1")
        .context("construct Minilith auth callback URL")?;
    let mut redirect_url = args.api_url.clone();
    redirect_url.set_path("/loadtesting/callback");
    redirect_url.set_query(None);
    redirect_url.set_fragment(None);
    let user_prefix = args.user_prefix.unwrap_or_else(fresh_user_prefix);

    println!("Minilith: {}", args.api_url);
    println!("fed-auth: {}", args.auth_url);
    println!("ticket kind: {}", args.ticket_kind_id);
    println!("group path: {}", args.group_path);
    println!("clients: {}", args.clients);
    println!("user prefix: {user_prefix}");
    if args.clients <= 1_000 {
        println!("warning: this run does not meet the plan's >1000-client load target");
    }

    let http = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(args.clients)
        .build()
        .context("build HTTP client")?;
    let database = PgPoolOptions::new()
        .max_connections(5)
        .connect(&args.database_url)
        .await
        .context("connect to PostgreSQL")?;
    let stats = Arc::new(Stats::default());
    let sample = SampleStatus::default();
    sample.set(None, "authenticating", None);
    let dashboard_stop = Arc::new(AtomicBool::new(false));
    let dashboard = tokio::spawn(display_dashboard(
        Arc::clone(&stats),
        sample.clone(),
        Arc::clone(&dashboard_stop),
        args.clients,
    ));
    let reported_errors = Arc::new(AtomicUsize::new(0));
    let interrupt = tokio::signal::ctrl_c();
    tokio::pin!(interrupt);

    let mut login_tasks = JoinSet::new();
    for index in 0..args.clients {
        let http = http.clone();
        let auth_url = args.auth_url.clone();
        let redirect_url = redirect_url.clone();
        let callback_url = callback_url.clone();
        let user_prefix = user_prefix.clone();
        login_tasks.spawn(async move {
            auth::login(
                &http,
                &auth_url,
                &redirect_url,
                &callback_url,
                &user_prefix,
                index,
            )
            .await
        });
        // if index % 50 == 0 {
        //     tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        // }
    }

    let mut sessions = Vec::with_capacity(args.clients);
    let mut interrupted = false;
    while !login_tasks.is_empty() {
        tokio::select! {
            signal = &mut interrupt => {
                signal.context("listen for Ctrl+C")?;
                interrupted = true;
                login_tasks.abort_all();
            }
            result = login_tasks.join_next() => {
                match result {
                    Some(Ok(Ok(session))) => {
                        stats.authenticated.fetch_add(1, Ordering::Relaxed);
                        sessions.push(session);
                    }
                    Some(Ok(Err(source))) => {
                        stats.finished.fetch_add(1, Ordering::Relaxed);
                        record_error(&stats, &reported_errors, source.as_ref());
                    }
                    Some(Err(source)) if !source.is_cancelled() => {
                        stats.finished.fetch_add(1, Ordering::Relaxed);
                        record_error(&stats, &reported_errors, &source);
                    }
                    Some(Err(_)) | None => {}
                }
            }
        }
        if interrupted {
            while login_tasks.join_next().await.is_some() {}
        }
    }
    if interrupted {
        stop_dashboard(&dashboard_stop, dashboard, &stats, &sample, args.clients).await;
        return Ok(());
    }
    if sessions.is_empty() {
        stop_dashboard(&dashboard_stop, dashboard, &stats, &sample, args.clients).await;
        return Err(error("no client completed the login flow"));
    }

    let user_ids: Vec<String> = sessions
        .iter()
        .map(|session| session.user_id.clone())
        .collect();
    grant_memberships(&database, &user_ids, &args.group_path).await?;

    let api = ApiClient::new(http, args.api_url.clone(), &args.auth_url)?;
    let ticket_kind = api
        .ticket_kind(
            sessions
                .first_mut()
                .ok_or_else(|| error("no session available for ticket-kind validation"))?,
            args.ticket_kind_id,
        )
        .await?;
    validate_ticket_kind(&ticket_kind, args.ticket_kind_id)?;
    let release_at = ticket_kind.release_at()?;
    println!("\nrelease: {}", format_time(release_at));

    let context = RunContext {
        api,
        stats: Arc::clone(&stats),
        sample: sample.clone(),
        ticket_kind_id: args.ticket_kind_id,
    };
    let mut client_tasks = JoinSet::new();
    for session in sessions {
        let context = context.clone();
        client_tasks.spawn(async move { run_client(session, context).await });
        // if index % 50 == 0 {
        //     tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        // }
    }

    while !client_tasks.is_empty() {
        tokio::select! {
            signal = &mut interrupt => {
                signal.context("listen for Ctrl+C")?;
                interrupted = true;
                client_tasks.abort_all();
            }
            result = client_tasks.join_next() => {
                match result {
                    Some(Ok(Ok(ClientOutcome::Purchased))) => {
                        stats.purchased.fetch_add(1, Ordering::Relaxed);
                        stats.finished.fetch_add(1, Ordering::Relaxed);
                    }
                    Some(Ok(Ok(ClientOutcome::SoldOut))) => {
                        stats.sold_out.fetch_add(1, Ordering::Relaxed);
                        stats.finished.fetch_add(1, Ordering::Relaxed);
                    }
                    Some(Ok(Err(source))) => {
                        stats.finished.fetch_add(1, Ordering::Relaxed);
                        record_error(&stats, &reported_errors, source.as_ref());
                    }
                    Some(Err(source)) if !source.is_cancelled() => {
                        stats.finished.fetch_add(1, Ordering::Relaxed);
                        record_error(&stats, &reported_errors, &source);
                    }
                    Some(Err(_)) | None => {}
                }
            }
        }
        if interrupted {
            while client_tasks.join_next().await.is_some() {}
        }
    }

    stop_dashboard(&dashboard_stop, dashboard, &stats, &sample, args.clients).await;
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "the queue state machine is clearer when kept together"
)]
async fn run_client(mut session: Session, context: RunContext) -> Result<ClientOutcome> {
    let is_sample = session.index == 0;
    let ticket_kind = context
        .api
        .ticket_kind(&mut session, context.ticket_kind_id)
        .await?;
    validate_ticket_kind(&ticket_kind, context.ticket_kind_id)?;
    let release_at = ticket_kind.release_at()?;
    let release = format_time(release_at);
    let enter_at = release_at - time::Duration::minutes(10);
    if is_sample {
        context.sample.set(
            Some(release.clone()),
            "waiting for the 10-minute entry window",
            None,
        );
    }
    sleep_until(enter_at).await;

    let initial = context
        .api
        .enter_queue(&mut session, context.ticket_kind_id)
        .await?;
    let QueueEntry::Status(initial) = initial else {
        if is_sample {
            context.sample.set(Some(release), "sold out", None);
        }
        return Ok(ClientOutcome::SoldOut);
    };
    let mut counted_reservation_queue = false;
    match initial {
        PurchaseStatus::ReleaseQueued => {
            context.stats.release_queued.fetch_add(1, Ordering::Relaxed);
            let jitter_millis = u64::try_from(session.index % 101).unwrap_or_default() * 100;
            let first_poll = release_at
                + time::Duration::SECOND * 5
                + time::Duration::milliseconds(i64::try_from(jitter_millis).unwrap_or_default());
            if is_sample {
                context.sample.set(
                    Some(release.clone()),
                    "release queue",
                    Some(format_time(first_poll)),
                );
            }
            sleep_until(first_poll).await;
        }
        PurchaseStatus::ReservationQueued => {
            context
                .stats
                .reservation_queued
                .fetch_add(1, Ordering::Relaxed);
            counted_reservation_queue = true;
        }
        PurchaseStatus::Reserved | PurchaseStatus::Buying | PurchaseStatus::Purchased => {}
    }

    loop {
        match context.api.queue_status(&mut session).await? {
            QueuePoll::Retry(message) => {
                let next_poll = OffsetDateTime::now_utc() + time::Duration::seconds(1);
                if is_sample {
                    context.sample.set(
                        Some(release.clone()),
                        format!("queue poll retry: {message}"),
                        Some(format_time(next_poll)),
                    );
                }
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
            QueuePoll::Missing => {
                if context
                    .api
                    .owns_ticket(&mut session, context.ticket_kind_id)
                    .await?
                {
                    if is_sample {
                        context.sample.set(Some(release), "purchased", None);
                    }
                    return Ok(ClientOutcome::Purchased);
                }
                if is_sample {
                    context.sample.set(Some(release), "sold out", None);
                }
                return Ok(ClientOutcome::SoldOut);
            }
            QueuePoll::Status(status) => {
                if status.ticket_kind != context.ticket_kind_id {
                    return Err(error(format!(
                        "{} was queued for unexpected ticket kind {}",
                        session.user_id, status.ticket_kind
                    )));
                }
                match status.placement {
                    Some(0) => {
                        if status.timeout.is_none() || status.start_transaction_before.is_none() {
                            return Err(error("reservation response omitted its deadlines"));
                        }
                        context.stats.reserved.fetch_add(1, Ordering::Relaxed);
                        if is_sample {
                            context.sample.set(
                                Some(release.clone()),
                                format!(
                                    "reserved until {}",
                                    status.timeout.as_deref().unwrap_or("unknown")
                                ),
                                None,
                            );
                        }
                        loop {
                            match context
                                .api
                                .buy_free(&mut session, context.ticket_kind_id)
                                .await?
                            {
                                BuyFreeOutcome::Started => break,
                                BuyFreeOutcome::PurchaseFlowBusy => {
                                    if is_sample {
                                        let retry_at =
                                            OffsetDateTime::now_utc() + time::Duration::seconds(3);
                                        context.sample.set(
                                            Some(release.clone()),
                                            "purchase flow busy",
                                            Some(format_time(retry_at)),
                                        );
                                    }
                                    tokio::time::sleep(PURCHASE_FLOW_RETRY_INTERVAL).await;
                                }
                            }
                        }
                        return wait_for_purchase(&mut session, &context, release, is_sample).await;
                    }
                    Some(placement) if placement > 0 => {
                        if !counted_reservation_queue {
                            context
                                .stats
                                .reservation_queued
                                .fetch_add(1, Ordering::Relaxed);
                            counted_reservation_queue = true;
                        }
                        let next_poll = OffsetDateTime::now_utc() + time::Duration::seconds(15);
                        if is_sample {
                            context.sample.set(
                                Some(release.clone()),
                                format!("reservation queue, {placement} ahead"),
                                Some(format_time(next_poll)),
                            );
                        }
                        tokio::time::sleep(QUEUE_POLL_INTERVAL).await;
                    }
                    Some(_) | None => {
                        let next_poll = OffsetDateTime::now_utc() + time::Duration::seconds(15);
                        if is_sample {
                            context.sample.set(
                                Some(release.clone()),
                                "release resolving",
                                Some(format_time(next_poll)),
                            );
                        }
                        tokio::time::sleep(QUEUE_POLL_INTERVAL).await;
                    }
                }
            }
        }
    }
}

async fn wait_for_purchase(
    session: &mut Session,
    context: &RunContext,
    release: String,
    is_sample: bool,
) -> Result<ClientOutcome> {
    loop {
        match context
            .api
            .owns_ticket(session, context.ticket_kind_id)
            .await
        {
            Ok(true) => {
                if is_sample {
                    context.sample.set(Some(release), "purchased", None);
                }
                return Ok(ClientOutcome::Purchased);
            }
            Ok(false) => {}
            Err(source) => {
                if is_sample {
                    context.sample.set(
                        Some(release.clone()),
                        format!("purchase verification retry: {source}"),
                        Some(format_time(
                            OffsetDateTime::now_utc() + time::Duration::seconds(1),
                        )),
                    );
                }
            }
        }
        if is_sample {
            context.sample.set(
                Some(release.clone()),
                "waiting for purchased ticket",
                Some(format_time(
                    OffsetDateTime::now_utc() + time::Duration::seconds(1),
                )),
            );
        }
        tokio::time::sleep(RETRY_INTERVAL).await;
    }
}

fn validate_ticket_kind(ticket_kind: &TicketKind, expected_id: Uuid) -> Result<()> {
    if ticket_kind.id != expected_id {
        return Err(error("ticket-kind endpoint returned a different ID"));
    }
    if ticket_kind.price != 0 {
        return Err(error(format!(
            "ticket kind {} costs {} öre; this load tester only purchases free kinds",
            ticket_kind.id, ticket_kind.price
        )));
    }
    if ticket_kind
        .available_addons
        .iter()
        .any(|addon| addon.required)
    {
        return Err(error(
            "ticket kind has required addons; the load tester cannot choose user-specific answers",
        ));
    }
    Ok(())
}

async fn grant_memberships(
    database: &sqlx::PgPool,
    user_ids: &[String],
    group_path: &str,
) -> Result<()> {
    let group_id = sqlx::query_scalar::<_, Uuid>(
        "select id from groups where path = $1::ltree and deleted = false",
    )
    .bind(group_path)
    .fetch_optional(database)
    .await
    .context("look up load-test group")?
    .ok_or_else(|| error(format!("group path {group_path:?} does not exist")))?;

    let created_users =
        sqlx::query_scalar::<_, i64>("select count(*) from users where id = any($1)")
            .bind(user_ids)
            .fetch_one(database)
            .await
            .context("verify load-test account creation")?;
    let expected = i64::try_from(user_ids.len()).context("client count does not fit in i64")?;
    if created_users != expected {
        return Err(error(format!(
            "fed-auth callback created {created_users} of {expected} expected users"
        )));
    }

    sqlx::query(
        "insert into group_memberships (user_id, group_id)
         select user_id, $2 from unnest($1::text[]) as generated_user(user_id)
         on conflict (user_id, group_id) do nothing",
    )
    .bind(user_ids)
    .bind(group_id)
    .execute(database)
    .await
    .context("grant load-test group memberships")?;

    let memberships = sqlx::query_scalar::<_, i64>(
        "select count(*) from group_memberships where user_id = any($1) and group_id = $2",
    )
    .bind(user_ids)
    .bind(group_id)
    .fetch_one(database)
    .await
    .context("verify load-test group memberships")?;
    if memberships != expected {
        return Err(error(format!(
            "created {memberships} of {expected} expected memberships"
        )));
    }
    Ok(())
}

async fn display_dashboard(
    stats: Arc<Stats>,
    sample: SampleStatus,
    stop: Arc<AtomicBool>,
    total: usize,
) {
    while !stop.load(Ordering::Relaxed) {
        print!("\x1b[2K\r{} | {}", stats.summary(total), sample.summary());
        drop(std::io::stdout().flush());
        tokio::time::sleep(StdDuration::from_secs(1)).await;
    }
}

async fn stop_dashboard(
    stop: &AtomicBool,
    dashboard: tokio::task::JoinHandle<()>,
    stats: &Stats,
    sample: &SampleStatus,
    total: usize,
) {
    stop.store(true, Ordering::Relaxed);
    drop(dashboard.await);
    println!("\n{}", stats.summary(total));
    println!("{}", sample.summary());
}

fn record_error(stats: &Stats, reported_errors: &AtomicUsize, source: &dyn std::error::Error) {
    stats.errors.fetch_add(1, Ordering::Relaxed);
    let reported = reported_errors.fetch_add(1, Ordering::Relaxed);
    if reported < 10 {
        eprintln!("\nclient error: {source}");
    } else if reported == 10 {
        eprintln!("\nadditional client errors are only included in the aggregate count");
    }
}

async fn sleep_until(target: OffsetDateTime) {
    let millis = (target - OffsetDateTime::now_utc()).whole_milliseconds();
    if let Ok(millis) = u64::try_from(millis) {
        tokio::time::sleep(StdDuration::from_millis(millis)).await;
    }
}

fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}

fn fresh_user_prefix() -> String {
    let run_id: String = Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect();
    format!("loadtest-{run_id}")
}

fn ensure_trailing_slash(url: &mut Url) {
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
}
