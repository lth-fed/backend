use bin_common::PgPool;
use uuid::Uuid;

use super::{
    access::{ensure_user_may_purchase_ticket, ensure_user_may_receive_transferred_ticket},
    allocation::{give_reservations, reserve_ticket_capacity},
    flow::{
        reserve_user_purchase_flow, set_user_purchase_flow_release_queue,
        set_user_purchase_flow_reservation_queue,
    },
    release::{release, remove_expired_release_queuers},
};
use crate::MinilithResult;

async fn start_release_flow(db: &PgPool, user_id: &str, ticket_kind: Uuid) -> MinilithResult<()> {
    let mut txn = db.begin().await?;
    reserve_user_purchase_flow(&mut txn, &[user_id.to_owned()], ticket_kind, &[false]).await?;
    sqlx::query!(
        "insert into ticket_release_queuers
            (user_id, ticket_kind_id, started_queueing)
            values ($1, $2, now())",
        user_id,
        ticket_kind,
    )
    .execute(&mut txn.executor())
    .await?;
    set_user_purchase_flow_release_queue(&mut txn, user_id).await?;
    txn.commit().await?;
    Ok(())
}

async fn start_reservation_queue_flow(
    db: &PgPool,
    user_id: &str,
    ticket_kind: Uuid,
    placement: i32,
) -> MinilithResult<()> {
    let mut txn = db.begin().await?;
    reserve_user_purchase_flow(&mut txn, &[user_id.to_owned()], ticket_kind, &[false]).await?;
    sqlx::query!(
        "insert into ticket_reservation_queuers
            (user_id, ticket_kind_id, placement)
            values ($1, $2, $3)",
        user_id,
        ticket_kind,
        placement,
    )
    .execute(&mut txn.executor())
    .await?;
    set_user_purchase_flow_reservation_queue(&mut txn, user_id).await?;
    txn.commit().await?;
    Ok(())
}

#[sqlx::test(fixtures("../fixtures/ticket_capacity.sql"))]
async fn reservation_capacity_is_shared_by_ticket_kinds(db: sqlx::PgPool) {
    let db = sqlx_tracing::Pool::from(db);
    let first = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
    let second = Uuid::parse_str("00000000-0000-0000-0000-000000000005").unwrap();

    let mut txn = db.begin().await.unwrap();
    assert_eq!(
        reserve_ticket_capacity(&mut txn, first, 2).await.unwrap(),
        2
    );
    assert_eq!(
        reserve_ticket_capacity(&mut txn, second, 2).await.unwrap(),
        1
    );
    assert_eq!(
        reserve_ticket_capacity(&mut txn, first, 1).await.unwrap(),
        0
    );
    txn.commit().await.unwrap();

    let total = sqlx::query_scalar!(
        r#"select sum(reserved_or_purchased_tickets)::int
            from ticket_kinds where activity_id =
                '00000000-0000-0000-0000-000000000003'"#
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(total, Some(3));
}

#[sqlx::test(fixtures("../fixtures/ticket_capacity.sql"))]
async fn concurrent_ticket_kinds_cannot_exceed_activity_capacity(db: sqlx::PgPool) {
    let db = sqlx_tracing::Pool::from(db);
    let first = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
    let second = Uuid::parse_str("00000000-0000-0000-0000-000000000005").unwrap();

    let reserve = |ticket_kind| {
        let db = db.clone();
        async move {
            let mut txn = db.begin().await.unwrap();
            let granted = reserve_ticket_capacity(&mut txn, ticket_kind, 2)
                .await
                .unwrap();
            txn.commit().await.unwrap();
            granted
        }
    };
    let (first_granted, second_granted) = tokio::join!(reserve(first), reserve(second));
    assert_eq!(first_granted + second_granted, 3);

    let total = sqlx::query_scalar!(
        r#"select sum(reserved_or_purchased_tickets)::int
            from ticket_kinds where activity_id =
                '00000000-0000-0000-0000-000000000003'"#
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(total, Some(3));
}

#[sqlx::test(fixtures("../fixtures/ticket_capacity.sql"))]
async fn only_one_concurrent_purchase_flow_can_commit(db: sqlx::PgPool) {
    let db = sqlx_tracing::Pool::from(db);
    let first = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
    let second = Uuid::parse_str("00000000-0000-0000-0000-000000000005").unwrap();

    let (first_result, second_result) = tokio::join!(
        start_release_flow(&db, "test:purchase-flow-1", first),
        start_release_flow(&db, "test:purchase-flow-1", second),
    );
    assert_ne!(
        first_result.is_ok(),
        second_result.is_ok(),
        "exactly one concurrent flow should commit"
    );

    let flow_count = sqlx::query_scalar!(
        "select count(*) from users_in_purchase_flow where user_id = $1",
        "test:purchase-flow-1"
    )
    .fetch_one(&db)
    .await
    .unwrap();
    let child_count = sqlx::query_scalar!(
        "select count(*) from ticket_release_queuers where user_id = $1",
        "test:purchase-flow-1"
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(flow_count, Some(1));
    assert_eq!(child_count, Some(1));
}

#[sqlx::test(fixtures("../fixtures/ticket_capacity.sql"))]
async fn release_moves_every_flow_and_removes_release_rows(db: sqlx::PgPool) {
    let db = sqlx_tracing::Pool::from(db);
    let ticket_kind = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
    start_release_flow(&db, "test:purchase-flow-1", ticket_kind)
        .await
        .unwrap();
    start_release_flow(&db, "test:purchase-flow-2", ticket_kind)
        .await
        .unwrap();

    let txn = db.begin().await.unwrap();
    release(None, txn, ticket_kind).await.unwrap();

    let release_count = sqlx::query_scalar!(
        "select count(*) from ticket_release_queuers where ticket_kind_id = $1",
        ticket_kind
    )
    .fetch_one(&db)
    .await
    .unwrap();
    let reservations = sqlx::query_scalar!(
        "select count(*) from ticket_reservations where ticket_kind_id = $1",
        ticket_kind
    )
    .fetch_one(&db)
    .await
    .unwrap();
    let reservation_flows = sqlx::query_scalar!(
        "select count(*) from users_in_purchase_flow
            where ticket_kind_id = $1 and reservation = user_id",
        ticket_kind
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(release_count, Some(0));
    assert_eq!(reservations, Some(2));
    assert_eq!(reservation_flows, Some(2));
}

#[sqlx::test(fixtures("../fixtures/ticket_capacity.sql"))]
async fn expired_release_queue_removes_child_and_flow(db: sqlx::PgPool) {
    let db = sqlx_tracing::Pool::from(db);
    let ticket_kind = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
    start_release_flow(&db, "test:purchase-flow-1", ticket_kind)
        .await
        .unwrap();
    sqlx::query!(
        "update ticket_release_queuers
            set started_queueing = now() - '21 minutes'::interval
            where user_id = $1",
        "test:purchase-flow-1"
    )
    .execute(&db)
    .await
    .unwrap();

    let mut txn = db.begin().await.unwrap();
    remove_expired_release_queuers(&mut txn).await.unwrap();
    txn.commit().await.unwrap();

    let has_flow = sqlx::query_scalar!(
        "select exists (
                select 1 from users_in_purchase_flow where user_id = $1
            ) as \"exists!\"",
        "test:purchase-flow-1"
    )
    .fetch_one(&db)
    .await
    .unwrap();
    let has_child = sqlx::query_scalar!(
        "select exists (
                select 1 from ticket_release_queuers where user_id = $1
            ) as \"exists!\"",
        "test:purchase-flow-1"
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(!has_flow);
    assert!(!has_child);
}

#[sqlx::test(fixtures("../fixtures/ticket_capacity.sql"))]
async fn reservation_queue_promotion_moves_child_and_flow(db: sqlx::PgPool) {
    let db = sqlx_tracing::Pool::from(db);
    let ticket_kind = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
    start_reservation_queue_flow(&db, "test:purchase-flow-1", ticket_kind, 1)
        .await
        .unwrap();

    let mut txn = db.begin().await.unwrap();
    give_reservations(ticket_kind, 1, &mut txn).await.unwrap();
    txn.commit().await.unwrap();

    let state = sqlx::query!(
        "select release_queue, reservation_queue, reservation
            from users_in_purchase_flow where user_id = $1",
        "test:purchase-flow-1"
    )
    .fetch_one(&db)
    .await
    .unwrap();
    let queue_exists = sqlx::query_scalar!(
        "select exists (
                select 1 from ticket_reservation_queuers where user_id = $1
            ) as \"exists!\"",
        "test:purchase-flow-1"
    )
    .fetch_one(&db)
    .await
    .unwrap();
    let reservation_exists = sqlx::query_scalar!(
        "select exists (
                select 1 from ticket_reservations where user_id = $1
            ) as \"exists!\"",
        "test:purchase-flow-1"
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(state.release_queue.is_none());
    assert!(state.reservation_queue.is_none());
    assert_eq!(state.reservation.as_deref(), Some("test:purchase-flow-1"));
    assert!(!queue_exists);
    assert!(reservation_exists);
}

#[sqlx::test(fixtures("../fixtures/ticket_capacity.sql"))]
async fn purchase_flow_rejects_second_ticket_for_one_activity(db: sqlx::PgPool) {
    let db = sqlx_tracing::Pool::from(db);
    let first = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
    let second = Uuid::parse_str("00000000-0000-0000-0000-000000000005").unwrap();
    sqlx::query!(
        "insert into purchased_tickets
            (ticket_kind_id, purchaser_id, owner_id, transaction_id)
            values ($1, $2, $2, $3)",
        first,
        "test:purchase-flow-1",
        Uuid::new_v4(),
    )
    .execute(&db)
    .await
    .unwrap();

    let mut txn = db.begin().await.unwrap();
    assert!(
        reserve_user_purchase_flow(
            &mut txn,
            &["test:purchase-flow-1".to_owned()],
            second,
            &[false]
        )
        .await
        .is_err(),
        "the purchase flow should reject a second ticket for the activity"
    );
}

#[sqlx::test(fixtures("../fixtures/ticket_capacity.sql"))]
async fn database_rejects_committed_flow_without_state(db: sqlx::PgPool) {
    let db = sqlx_tracing::Pool::from(db);
    let ticket_kind = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
    let mut txn = db.begin().await.unwrap();
    sqlx::query!(
        "insert into users_in_purchase_flow (user_id, ticket_kind_id)
            values ($1, $2)",
        "test:purchase-flow-1",
        ticket_kind,
    )
    .execute(&mut txn.executor())
    .await
    .unwrap();
    assert!(
        txn.commit().await.is_err(),
        "the deferred state constraint should reject a state-less flow"
    );
}

#[sqlx::test(fixtures("../fixtures/ticket_capacity.sql"))]
async fn transfer_groups_include_descendant_members(db: sqlx::PgPool) {
    let ticket_kind = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
    let root = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    let child = Uuid::parse_str("00000000-0000-0000-0000-000000000006").unwrap();
    sqlx::query!(
        "insert into groups (id, path, name, description, logo_id, limit_membership_visibility)
            values ($1, 'root.child', '{}'::jsonb, '{}'::jsonb, $2, false)",
        child,
        Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
    )
    .execute(&db)
    .await
    .unwrap();
    sqlx::query!(
        "insert into group_memberships (user_id, group_id) values ($1, $2)",
        "test:purchase-flow-1",
        child,
    )
    .execute(&db)
    .await
    .unwrap();
    sqlx::query!(
        "insert into ticket_kind_transfer_groups (ticket_kind_id, group_id)
            values ($1, $2)",
        ticket_kind,
        root,
    )
    .execute(&db)
    .await
    .unwrap();

    ensure_user_may_receive_transferred_ticket(&db, "test:purchase-flow-1", ticket_kind)
        .await
        .unwrap();

    sqlx::query!(
        "delete from ticket_kind_transfer_groups where ticket_kind_id = $1",
        ticket_kind,
    )
    .execute(&db)
    .await
    .unwrap();
    assert!(
        ensure_user_may_receive_transferred_ticket(&db, "test:purchase-flow-1", ticket_kind,)
            .await
            .is_err()
    );
}

#[sqlx::test(fixtures("../fixtures/ticket_capacity.sql"))]
async fn cannot_purchase_ticket_after_activity_ends(db: sqlx::PgPool) {
    let ticket_kind = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
    let group = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    sqlx::query!(
        "insert into group_memberships (user_id, group_id) values ($1, $2)",
        "test:purchase-flow-1",
        group,
    )
    .execute(&db)
    .await
    .unwrap();
    sqlx::query!(
        "insert into ticket_kind_allowed_groups (ticket_kind_id, group_id) values ($1, $2)",
        ticket_kind,
        group,
    )
    .execute(&db)
    .await
    .unwrap();

    ensure_user_may_purchase_ticket(&db, "test:purchase-flow-1", ticket_kind)
        .await
        .unwrap();

    sqlx::query!(
        "update activities set time_end = now() - interval '1 second',
            time_start = now() - interval '1 hour'
        where id = '00000000-0000-0000-0000-000000000003'"
    )
    .execute(&db)
    .await
    .unwrap();

    assert!(
        ensure_user_may_purchase_ticket(&db, "test:purchase-flow-1", ticket_kind)
            .await
            .is_err()
    );
}
