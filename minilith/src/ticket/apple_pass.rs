//! An Apple Wallet pass for a ticket the caller holds.

use fed_auth_verifier::User;
use minilith_errors::{MinilithErrorOptionExt as _, MinilithErrorResultExt as _};
use poem_openapi::payload::{Binary, Response};
use serde_json::{Value, json};
use sqlx::types::time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use super::catalog;
use super::models::PurchasedTicket;
use crate::{ContextWrapper, MinilithEndpointError, MinilithResult};

/// How long after its activity ends a pass is marked expired, so Wallet files it away. Matches how
/// long the apps keep a ticket in their own wallet.
const EXPIRES_AFTER_END: time::Duration = time::Duration::hours(6);

/// The pass's own labels; Wallet formats the dates itself, in the device's locale.
struct Labels {
    ticket_for: &'static str,
    date: &'static str,
    location: &'static str,
    ticket: &'static str,
    holder: &'static str,
    organiser: &'static str,
    directions: &'static str,
    serial: &'static str,
}

const ENGLISH: Labels = Labels {
    ticket_for: "Ticket for",
    date: "Date",
    location: "Location",
    ticket: "Ticket",
    holder: "Name",
    organiser: "Organiser",
    directions: "Directions",
    serial: "Ticket ID",
};

const SWEDISH: Labels = Labels {
    ticket_for: "Biljett till",
    date: "Datum",
    location: "Plats",
    ticket: "Biljett",
    holder: "Namn",
    organiser: "Arrangör",
    directions: "Vägbeskrivning",
    serial: "Biljett-ID",
};

/// `/tickets/{id}/apple-pass`. Only for a ticket the caller holds: the barcode is bound to its
/// owner, so a pass for anyone else's would never scan.
pub(super) async fn apple_pass(
    ctx: &ContextWrapper,
    auth: User,
    id: Uuid,
) -> MinilithResult<Response<Binary<Vec<u8>>>> {
    let Some(signer) = ctx.apple_passes() else {
        // 404 rather than 500: the server is working as configured, it just has no certificate.
        return Err(MinilithEndpointError::not_found());
    };
    let owner_id = auth.get_id().to_owned();

    let user = sqlx::query!("select name, language from users where id = $1", owner_id)
        .fetch_one(&ctx.db)
        .await?;
    let language = ctx
        .decrypt_string(user.language)
        .wrap_err_encryption("user.language")?;
    let holder = ctx
        .decrypt_string(user.name)
        .wrap_err_encryption("user.name")?;

    let ticket = catalog::my_tickets(ctx, auth)
        .await?
        .into_iter()
        .find(|ticket| ticket.id == id && ticket.owned_by_me)
        .wrap_err_bad_frontend("you don't hold this ticket")?;

    let signature = ctx
        .pass_barcode_signature(ticket.id, &owner_id)
        .wrap_err_internal("pass barcode signature")?;
    let pass = pass_json(
        &ticket,
        &language,
        &holder,
        &signer.pass_type_identifier,
        &signer.team_identifier,
        &signature,
    )
    .wrap_err_internal("pass.json")?;
    let pkpass = signer
        .sign(pass.to_string().as_bytes())
        .wrap_err_internal("pass signing")?;

    Ok(Response::new(Binary(pkpass)).header("content-type", "application/vnd.apple.pkpass"))
}

/// The barcode's message. A JSON object like the app's own QR, so a scanner can tell the two
/// apart by their keys: `{tid, t}` is the app's timed code, `{tid, sig}` a pass.
fn barcode_message(ticket_id: Uuid, signature: &str) -> String {
    json!({ "tid": ticket_id, "sig": signature }).to_string()
}

fn rfc3339(time: OffsetDateTime) -> Result<String, time::error::Format> {
    time.format(&Rfc3339)
}

fn pass_json(
    ticket: &PurchasedTicket,
    language: &str,
    holder: &str,
    pass_type_identifier: &str,
    team_identifier: &str,
    signature: &str,
) -> Result<Value, time::error::Format> {
    let labels = if language.starts_with("sv") {
        &SWEDISH
    } else {
        &ENGLISH
    };
    let title = ticket.activity_title.resolve_intl(language, "");
    let kind = ticket.inner.ticket_kind_name.resolve_intl(language, "");
    let organiser = ticket.creator_name.resolve_intl(language, "");
    let location = &ticket.activity_location;
    let place = location
        .name
        .as_ref()
        .map(|name| name.resolve_intl(language, ""))
        .unwrap_or_default();
    let start = rfc3339(ticket.time_start)?;
    let end = rfc3339(ticket.time_end)?;
    let serial = format!("#{}", &ticket.id.simple().to_string()[..8]).to_uppercase();

    let back = back_fields(ticket, language, labels, holder, &serial);

    let mut pass = json!({
        "formatVersion": 1,
        "passTypeIdentifier": pass_type_identifier,
        "teamIdentifier": team_identifier,
        "serialNumber": ticket.id,
        "organizationName": "Teknologappen",
        "description": format!("{} {title}", labels.ticket_for),
        "logoText": organiser,
        "foregroundColor": "rgb(0, 0, 0)",
        "labelColor": "rgb(110, 110, 115)",
        "backgroundColor": "rgb(255, 255, 255)",
        // `relevantDates` replaces `relevantDate` from iOS 18; both are set for older devices.
        "relevantDate": start,
        "relevantDates": [{ "startDate": start, "endDate": end }],
        "expirationDate": rfc3339(ticket.time_end + EXPIRES_AFTER_END)?,
        "barcodes": [{
            "format": "PKBarcodeFormatQR",
            "message": barcode_message(ticket.id, signature),
            "messageEncoding": "iso-8859-1",
            "altText": serial,
        }],
        "eventTicket": {
            "primaryFields": [{ "key": "event", "label": labels.ticket, "value": title }],
            "secondaryFields": [{
                "key": "date",
                "label": labels.date,
                "value": start,
                "dateStyle": "PKDateStyleMedium",
                "timeStyle": "PKDateStyleShort",
            }],
            "auxiliaryFields": [
                { "key": "location", "label": labels.location, "value": place },
                { "key": "kind", "label": labels.ticket, "value": kind },
            ],
            "backFields": back,
        },
        "semantics": {
            "eventName": title,
            "eventStartDate": start,
            "eventEndDate": end,
            "attendeeName": holder,
            "venueName": place,
        },
    });
    if let Some(point) = location.coordinate_wgs84 {
        let coordinates = json!({ "latitude": point.north, "longitude": point.east });
        if let Some(semantics) = pass.get_mut("semantics").and_then(Value::as_object_mut) {
            semantics.insert("venueLocation".to_owned(), coordinates.clone());
        }
        if let Some(object) = pass.as_object_mut() {
            object.insert("locations".to_owned(), json!([coordinates]));
        }
    }
    Ok(pass)
}

/// The back of the pass: who holds it, what they chose, and where to go.
fn back_fields(
    ticket: &PurchasedTicket,
    language: &str,
    labels: &Labels,
    holder: &str,
    serial: &str,
) -> Vec<Value> {
    let location = &ticket.activity_location;
    let organiser = ticket.creator_name.resolve_intl(language, "");
    let mut back = vec![json!({ "key": "holder", "label": labels.holder, "value": holder })];
    for addon in &ticket.purchased_addons {
        let chosen: Vec<&str> = addon
            .selected_options
            .iter()
            .filter_map(|idx| addon.options.iter().find(|option| option.idx == *idx))
            .map(|option| option.name.resolve_intl(language, ""))
            .collect();
        let value = if chosen.is_empty() {
            addon.selected_text.trim().to_owned()
        } else {
            chosen.join(", ")
        };
        if !value.is_empty() {
            back.push(json!({
                "key": format!("addon-{}", addon.inner.id),
                "label": addon.inner.name.resolve_intl(language, ""),
                "value": value,
            }));
        }
    }
    back.push(json!({ "key": "organiser", "label": labels.organiser, "value": organiser }));
    if let Some(directions) = &location.directions {
        back.push(json!({
            "key": "directions",
            "label": labels.directions,
            "value": directions.resolve_intl(language, ""),
        }));
    }
    if let Some(url) = &location.url {
        back.push(json!({ "key": "map", "label": labels.location, "value": url }));
    }
    back.push(json!({ "key": "serial", "label": labels.serial, "value": serial }));
    back
}

/// Whether `signature` is the barcode signature for `ticket_id` held by `owner_id`. Compared in
/// constant time, so the comparison doesn't leak how much of a forged signature was right.
pub(super) fn verify_barcode_signature(
    ctx: &ContextWrapper,
    ticket_id: Uuid,
    owner_id: &str,
    signature: &str,
) -> MinilithResult<bool> {
    let expected = ctx
        .pass_barcode_signature(ticket_id, owner_id)
        .wrap_err_internal("pass barcode signature")?;
    Ok(expected.len() == signature.len()
        && openssl::memcmp::eq(expected.as_bytes(), signature.as_bytes()))
}
