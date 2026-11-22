-- this file is written in chronological order. Please look for indexes which do the thing you want to do before adding more

-- groups & memberships
create index group_memberships_by_member on group_memberships using hash (user_id);
create index group_memberships_by_group on group_memberships using hash (group_id);
create index group_memberships_expiry on group_memberships (until) where until is not null;
create index groups_parent_path on groups using btree (parent_path);

-- activities
create index activity_time_start on activities using btree (time_start);
create index activity_time_end on activities using btree (time_end);
create index activity_hosts_by_activity on activity_hosts using hash (activity_id);
create index activity_admin_access_by_access_group
    on allow_admins_from_group_view_activities using hash (access_group_id);
create index activity_views_by_activity on activity_views using hash (activity_id);

-- products
create index products_by_group on products using hash (group_id);
create index products_sales_start on products using btree (sales_start);
create index product_activity_assignments_by_activity
    on product_activity_assignments using hash (activity_id);
create index product_allowed_groups_by_group on product_allowed_groups using hash (group_id);
create index product_transfer_groups_by_product
    on product_transfer_groups using hash (product_id);

-- purchases
create index purchases_by_owner on purchases (owner_id);
create index purchases_by_product on purchases using hash (product_id);
create index purchases_by_activity on purchases using hash (activity_id);
create index purchase_transactions_by_purchase on purchase_transactions (purchase_id);
create index purchase_uses_by_purchase on purchase_uses (purchase_id);
create index purchased_product_addons_by_purchase on purchased_product_addons (purchase_id);

-- purchase queue & reservations
create index product_queuer_placement on product_reservation_queuers using btree (placement);
create index product_queuer_timeout on product_release_queuers using btree (started_queueing);
create index product_reservation_timeout on product_reservations using btree (timeout);
create index product_reserved_transaction on product_reservations using hash (transaction_id);
create index product_release_queuers_by_ticket on product_release_queuers using hash (product_id);

-- stock
create index stock_transactions_by_group on stock_transactions using hash (group_id);
create index stock_transaction_records_by_product
    on stock_transaction_records using hash (product_id);

-- notifications
create index push_notifications_unsent_send_time on push_notifications (send_at) where sent = false;
create index ticket_release_push_notifications_by_ticket
    on ticket_release_push_notifications using hash (ticket_id);
create index activity_push_notifications_by_activity
    on activity_push_notifications (activity_id);
create index purchased_ticket_push_notifications_by_activity
    on purchased_ticket_push_notifications (activity_id);
create index news_push_notifications_by_news on news_push_notifications (news_id);
create index notification_events_by_notification on notification_events (notification_id);
create index notification_reads_by_notification on notification_reads (notification_id);

-- votes
create index vote_rounds_by_meeting on vote_rounds using hash (meeting_id);
create index vote_alternatives_by_round on vote_alternatives using hash (round_id);
