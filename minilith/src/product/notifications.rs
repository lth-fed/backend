use std::{collections::HashMap, sync::Arc};

use uuid::Uuid;

use crate::{
    ContextWrapper, InternationalizedString as IS, MinilithResult,
    push_notifications::{NotificationKind, NotificationRow, PushDeviceRow, send_notifications},
};

pub(super) enum TicketNotification {
    Reservation,
    Transfer,
    /// The admins refunded the purchase. `message` is what they entered, if anything.
    Refund {
        message: Option<String>,
    },
}

/// Appends the admin's message to a notification, prefixed with `label`.
fn message_suffix(message: Option<&str>, label: &str) -> String {
    match message.map(str::trim) {
        Some(message) if !message.is_empty() => format!("\n\n{label}{message}"),
        _ => String::new(),
    }
}

/// Two-language text with the product's name in the same language spliced in at `{}`. The same
/// text is used for both languages when the product has no name for it.
pub(super) fn with_product_name(product_name: &IS, sv: &str, en: &str) -> IS {
    let text = |language: &str, template: &str| {
        let name = product_name.resolve_intl(language, "");
        if name.is_empty() {
            template.to_owned()
        } else {
            template.replace("{}", name)
        }
    };
    IS(HashMap::from([
        ("sv".to_owned(), text("sv", sv)),
        ("en".to_owned(), text("en", en)),
    ]))
}

pub(super) fn reservation_notification(
    id: Uuid,
    activity_id: Uuid,
    product_name: &IS,
) -> NotificationRow {
    NotificationRow {
        id,
        kind: NotificationKind::Reservation,
        activity_id: Some(activity_id),
        sender: sqlx::types::Json(IS::empty()),
        title: IS(HashMap::from([
            ("sv".to_owned(), "Gå in och köp biljetten!".to_owned()),
            ("en".to_owned(), "Open the app to buy your ticket!".to_owned()),
        ])).into(),
        content: with_product_name(
            product_name,
            "Du fick en reservation till {}. Köp biljetten snart, annars får någon annan din reservation.",
            "You got a reservation for {}. Buy the ticket soon, else someone else will get your reservation.",
        )
        .into(),
    }
}

/// Call only after commit. These transactional pushes are never saved to history,
/// and delivery failures must not change the successful ticket operation.
pub(super) fn notify_ticket_users(
    ctx: &ContextWrapper,
    ticket_kind: Uuid,
    users: Vec<String>,
    kind: TicketNotification,
) {
    if users.is_empty() {
        return;
    }
    let ctx = Arc::clone(ctx);
    tokio::spawn(async move {
        // MinilithEndpointError creation logs and alerts failures.
        drop(send_ticket_notification(&ctx, ticket_kind, &users, kind).await);
    });
}

/// The product's activity, when it has one. Store items have none; their notifications then use
/// the product as the sender.
struct NotificationTarget {
    activity_id: Option<Uuid>,
    activity_title: Option<crate::DbInternationalizedString>,
    product_name: crate::DbInternationalizedString,
}

async fn send_ticket_notification(
    ctx: &ContextWrapper,
    ticket_kind: Uuid,
    users: &[String],
    kind: TicketNotification,
) -> MinilithResult<()> {
    let Some(target) = sqlx::query!(
        r#"select kind.name as "product_name!: crate::DbInternationalizedString",
            activity.id as "activity_id?",
            activity.title as "activity_title?: crate::DbInternationalizedString"
        from products kind
        left join product_activity_assignments assignment on assignment.product_id = kind.id
        left join activities activity on activity.id = assignment.activity_id
        where kind.id = $1
        order by activity.id
        limit 1"#,
        ticket_kind,
    )
    .fetch_optional(&ctx.db)
    .await?
    .map(|row| NotificationTarget {
        activity_id: row.activity_id,
        activity_title: row.activity_title,
        product_name: row.product_name,
    }) else {
        return Ok(());
    };
    let devices = sqlx::query_as!(
        PushDeviceRow,
        r#"select user_id, device_id, push_token, language,
        platform as "platform!: crate::push_notifications::PushPlatform"
        from users join push_devices on push_devices.user_id = users.id
        where users.id = any($1)"#,
        users,
    )
    .fetch_all(&ctx.db)
    .await?;
    let notification = match kind {
        TicketNotification::Reservation => {
            // Reservations only exist for tickets, which always belong to an activity.
            let Some(activity_id) = target.activity_id else {
                return Ok(());
            };
            reservation_notification(Uuid::new_v4(), activity_id, &target.product_name.0)
        }
        TicketNotification::Transfer => NotificationRow {
            id: Uuid::new_v4(),
            kind: NotificationKind::Transfer,
            activity_id: target.activity_id,
            sender: target.activity_title.unwrap_or(target.product_name),
            title: IS(HashMap::from([
                ("sv".to_owned(), "Du har fått en biljett!".to_owned()),
                ("en".to_owned(), "You received a ticket!".to_owned()),
            ]))
            .into(),
            content: IS(HashMap::from([
                (
                    "sv".to_owned(),
                    "En biljett har överförts till dig. Öppna appen för att se den.".to_owned(),
                ),
                (
                    "en".to_owned(),
                    "A ticket has been transferred to you. Open the app to view it.".to_owned(),
                ),
            ]))
            .into(),
        },
        TicketNotification::Refund { message } => NotificationRow {
            id: Uuid::new_v4(),
            kind: NotificationKind::Refund,
            activity_id: target.activity_id,
            sender: target.activity_title.unwrap_or(target.product_name),
            title: IS(HashMap::from([
                ("sv".to_owned(), "Ditt köp har återbetalats".to_owned()),
                ("en".to_owned(), "Your purchase was refunded".to_owned()),
            ]))
            .into(),
            content: IS(HashMap::from([
                (
                    "sv".to_owned(),
                    format!(
                        "Ditt köp har återbetalats.{}",
                        message_suffix(message.as_deref(), "Meddelande: ")
                    ),
                ),
                (
                    "en".to_owned(),
                    format!(
                        "Your purchase has been refunded.{}",
                        message_suffix(message.as_deref(), "Message: ")
                    ),
                ),
            ]))
            .into(),
        },
    };
    let removed = send_notifications(ctx, &notification, devices).await?;
    let mut txn = ctx.db.begin().await?;
    removed.clear_failed(&mut txn).await?;
    txn.commit().await?;
    Ok(())
}
