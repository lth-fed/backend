//! Stock movement report for a period: the lager ↔ activity invoicing basis and supplier purchases.

use minilith_errors::{MinilithErrorOptionExt as _, MinilithErrorResultExt as _, MinilithResult};
use serde::Serialize;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    ContextWrapper, DbInternationalizedString as DIS, InternationalizedString as IS, report,
    report::Template,
};

/// Picks the report language's title, falling back to any title present.
fn title(value: &IS, language: report::Language) -> String {
    let key = match language {
        report::Language::Swedish => "sv",
        report::Language::English => "en",
    };
    value
        .0
        .get(key)
        .or_else(|| value.0.values().next())
        .cloned()
        .unwrap_or_default()
}

const STOCK_TYPST: &str = include_str!("./stock.typ");

/// Maps a stored record kind to its typst label key; unknown kinds fall back to `balance`.
fn static_kind(kind: &str) -> &'static str {
    match kind {
        "take-out" => "take-out",
        "put-back" => "put-back",
        "purchase" => "purchase",
        "refund" => "refund",
        "sales" => "sales",
        "sales-refund" => "sales-refund",
        "inventory" => "inventory",
        _ => "balance",
    }
}

/// `(transaction, product)` pairs whose take-out equals its put-back, i.e. nothing was consumed.
fn cancelled_movements<'a>(
    rows: impl Iterator<Item = (Uuid, Uuid, &'a str, f64)>,
) -> std::collections::BTreeSet<(Uuid, Uuid)> {
    let mut returned = std::collections::BTreeMap::<(Uuid, Uuid), (f64, f64)>::default();
    for (transaction_id, product_id, kind, amount) in rows {
        let entry = returned.entry((transaction_id, product_id)).or_default();
        match kind {
            "take-out" => entry.0 += amount,
            "put-back" => entry.1 += amount,
            _ => {}
        }
    }
    returned
        .into_iter()
        .filter(|(_, (taken_out, put_back))| (taken_out - put_back).abs() < 1e-9)
        .map(|(key, _)| key)
        .collect()
}

#[derive(Serialize)]
struct Movement {
    time: String,
    kind: &'static str,
    name: String,
    activity_name: String,
    product_name: String,
    amount: String,
    /// Value in öre at the product's value per unit, when the amount is a whole number of units.
    value: Option<i64>,
}

#[derive(Serialize)]
struct ProductSummary {
    product_name: String,
    net_amount: String,
    net_value: i64,
}

#[derive(Serialize)]
struct StockData {
    language: &'static str,
    group_name: String,
    period: String,
    exported_at: Option<String>,
    movements: Vec<Movement>,
    per_product: Vec<ProductSummary>,
}

/// Renders a group's stock movements as a PDF: one `transaction_id`, or every movement in the
/// half-open period `start`/`end`.
///
/// # Errors
///
/// `NOT_FOUND` when the group does not exist, `INTERNAL` if rendering fails.
#[allow(
    clippy::too_many_lines,
    reason = "one linear pass from query rows to the report data"
)]
pub(crate) async fn generate_stock_report(
    ctx: &ContextWrapper,
    group_id: Uuid,
    start: Option<OffsetDateTime>,
    end: Option<OffsetDateTime>,
    language: report::Language,
    transaction_id: Option<Uuid>,
) -> MinilithResult<Vec<u8>> {
    let group_name = sqlx::query_scalar!(
        r#"select name as "name!: DIS" from groups where id = $1"#,
        group_id
    )
    .fetch_optional(&ctx.db)
    .await?
    .wrap_err_not_found()?;
    let group_name = title(&group_name.0, language);
    let rows = sqlx::query!(
        r#"select
            transaction.id as "transaction_id!",
            record.product_id,
            transaction.time,
            transaction.name as "transaction_name!: DIS",
            activity.title as "activity_title?: DIS",
            record.kind::text as "kind!",
            record.amount::float8 as "amount!",
            products.name as "product_name!: DIS",
            products.stock_price::float8 as "stock_price!"
        from stock_transactions transaction
        inner join stock_transaction_records record on record.transaction_id = transaction.id
        inner join products on products.id = record.product_id
        left join activities activity on activity.id = transaction.activity_id
        where transaction.group_id = $1
        and ($2::timestamptz is null or transaction.time >= $2)
        and ($3::timestamptz is null or transaction.time < $3)
        and ($4::uuid is null or transaction.id = $4)
        order by transaction.time, transaction.id"#,
        group_id,
        start,
        end,
        transaction_id,
    )
    .fetch_all(&ctx.db)
    .await?;
    let lang = match language {
        report::Language::Swedish => "sv",
        report::Language::English => "en",
    };
    // Products that left and came back in the same movement had no net effect.
    let cancelled = cancelled_movements(rows.iter().map(|row| {
        (
            row.transaction_id,
            row.product_id,
            row.kind.as_str(),
            row.amount,
        )
    }));
    let mut movements = Vec::with_capacity(rows.len());

    let mut per_product: std::collections::BTreeMap<String, (f64, i64)> =
        std::collections::BTreeMap::default();
    for row in rows {
        if matches!(row.kind.as_str(), "take-out" | "put-back")
            && cancelled.contains(&(row.transaction_id, row.product_id))
        {
            continue;
        }
        let product_name = title(&row.product_name.0, language);
        let entry = per_product.entry(product_name.clone()).or_default();
        let signed = match row.kind.as_str() {
            // Everything that removes stock from the group: handed to an activity, sold, or
            // returned to the supplier.
            "take-out" | "sales" | "refund" => -row.amount,
            _ => row.amount,
        };
        entry.0 += signed;
        // Value in öre for the record: amount × stock price (per unit).
        #[allow(
            clippy::cast_possible_truncation,
            reason = "stock values are whole öre; amounts are bounded by the stock table"
        )]
        let value = (row.amount.abs() * row.stock_price).round() as i64;
        entry.1 += if signed < 0.0 { -value } else { value };
        movements.push(Movement {
            time: row.time.to_string(),
            kind: static_kind(&row.kind),
            name: title(&row.transaction_name.0, language),
            activity_name: row
                .activity_title
                .map_or_else(String::new, |value| title(&value.0, language)),
            product_name,
            amount: format!("{:.6}", row.amount),
            value: Some(value),
        });
    }
    let per_product = per_product
        .into_iter()
        .map(|(product_name, (net_amount, net_value))| ProductSummary {
            product_name,
            net_amount: format!("{net_amount:.6}"),
            net_value,
        })
        .collect();
    let data = StockData {
        language: lang,
        group_name,
        period: format!(
            "{} – {}",
            start.map_or_else(|| "…".to_owned(), |value| value.to_string()),
            end.map_or_else(|| "…".to_owned(), |value| value.to_string()),
        ),
        exported_at: None,
        movements,
        per_product,
    };
    let template = Template {
        source: STOCK_TYPST,
        data_json: serde_json::to_string(&data)
            .wrap_err_internal("stock report: serialize data.json")?,
        image: None,
    };
    report::compile_template(ctx.report_typst(), &template)
}
