/// Ensure that the user may purchase a ticket of the specified `ticket_kind`
/// with regard to their group memberships.
///
/// If no allowed groups are configured for the ticket kind, no one may
/// purchase. Otherwise the user must be a (transitive) member of at least one
/// allowed group — membership in a parent group covers all descendant groups.
///
/// # Errors
///
/// Returns 403 if the user is not allowed to purchase, or an internal error if
/// the database query fails.
async fn ensure_user_may_purchase_ticket(
    db: impl PgExecutor<'_>,
    user_id: &str,
    ticket_kind: Uuid,
) -> MinilithResult<()> {
    let may_purchase = sqlx::query_scalar!(
        r#"select (
            exists (
                select 1
                from group_memberships
                inner join groups member_group on member_group.id = group_memberships.group_id
                inner join ticket_kind_allowed_groups tk_ag on tk_ag.ticket_kind_id = $1
                inner join groups allowed_group on allowed_group.id = tk_ag.group_id
                    and allowed_group.path @> member_group.path

                where group_memberships.user_id = $2
                and (
                    member_group.limit_membership_visibility = false
                    or tk_ag.group_id = group_memberships.group_id
                )
            )
        ) as "may_purchase!""#,
        ticket_kind,
        user_id,
    )
    .fetch_one(db)
    .await?;

    if !may_purchase {
        return Err(MinilithEndpointError::bad_user_input(
            "purchase",
            "",
            "not allowed to purchase this ticket kind OR \
            you have already purchased one ticket for this activity",
            "ticket_kind",
        ));
    }

    Ok(())
}

/// Transfer groups include all descendant groups in the path tree. An empty
/// transfer-group set therefore permits no recipient.
async fn ensure_user_may_receive_transferred_ticket(
    db: impl PgExecutor<'_>,
    user_id: &str,
    ticket_kind: Uuid,
) -> MinilithResult<()> {
    let may_receive = sqlx::query_scalar!(
        r#"select exists (
            select 1
            from group_memberships
            inner join groups member_group on member_group.id = group_memberships.group_id
            inner join ticket_kind_transfer_groups transfer
                on transfer.ticket_kind_id = $1
            inner join groups transfer_group on transfer_group.id = transfer.group_id
                and transfer_group.path @> member_group.path
            where group_memberships.user_id = $2
        ) as "may_receive!""#,
        ticket_kind,
        user_id,
    )
    .fetch_one(db)
    .await?;

    if !may_receive {
        return Err(MinilithEndpointError::bad_user_input(
            "transfer",
            "",
            "recipient is not a member of an allowed transfer group or descendant",
            "to_user",
        ));
    }

    Ok(())
}
