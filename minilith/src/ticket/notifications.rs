use std::{collections::HashMap, sync::Arc};

use uuid::Uuid;

use crate::{
    ContextWrapper, InternationalizedString as IS, MinilithResult,
    push_notifications::{NotificationRow, PushDeviceRow, send_notifications},
};

pub(super) enum TicketNotification {
    Reservation,
    Transfer,
}

pub(super) fn reservation_notification(id: Uuid, activity_id: Uuid) -> NotificationRow {
    NotificationRow {
        id,
        activity_id: Some(activity_id),
        sender: sqlx::types::Json(IS::empty()),
        title: IS(HashMap::from([
            ("sv".to_owned(), "Gå in och köp biljetten!".to_owned()),
            ("en".to_owned(), "Open the app to buy your ticket!".to_owned()),
        ])).into(),
        content: IS(HashMap::from([
            ("sv".to_owned(), "Du fick en reservation. Köp biljetten snart, annars får någon annan din reservation.".to_owned()),
            ("en".to_owned(), "You got a reservation. Buy the ticket soon, else someone else will get your reservation.".to_owned()),
        ])).into(),
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

async fn send_ticket_notification(
    ctx: &ContextWrapper,
    ticket_kind: Uuid,
    users: &[String],
    kind: TicketNotification,
) -> MinilithResult<()> {
    let activity = sqlx::query!(
        r#"select activity.id, activity.title as "title!: crate::DbInternationalizedString"
        from ticket_kinds kind join activities activity on activity.id = kind.activity_id
        where kind.id = $1"#,
        ticket_kind,
    )
    .fetch_one(&ctx.db)
    .await?;
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
        TicketNotification::Reservation => reservation_notification(Uuid::new_v4(), activity.id),
        TicketNotification::Transfer => NotificationRow {
            id: Uuid::new_v4(),
            activity_id: Some(activity.id),
            sender: activity.title,
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
    };
    let removed = send_notifications(ctx, &notification, devices).await?;
    let mut txn = ctx.db.begin().await?;
    removed.clear_failed(&mut txn).await?;
    txn.commit().await?;
    Ok(())
}
