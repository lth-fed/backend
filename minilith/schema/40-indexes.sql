-- this file is written in chronological order. Please look for indexes which do the thing you want to do before adding more

-- when writing the activity details & activity list APIs
create index group_memberships_by_member on group_memberships using hash (user_id);
create index group_memberships_by_group on group_memberships using hash (group_id);
create index group_tree on groups using gist (path);
create index group_tree_eq on groups using hash (path);
create index user_group_settings_by_user on user_group_settings using hash (user_id);
create index user_group_settings_by_group on user_group_settings using hash (group_id);
create index activity_time_start on activities using btree (time_start);
create index activity_time_end on activities using btree (time_end);
create index activity_hosts_by_activity on activity_hosts using hash (activity_id);
create index ticket_kind_by_activity on ticket_kinds using hash (activity_id);
create index ticket_kind_allowed_groups_by_group on ticket_kind_allowed_groups using hash (group_id);
create index ticket_kind_allowed_groups_by_ticket_kind
    on ticket_kind_allowed_groups using hash (ticket_kind_id);
create index ticket_kind_notifications_by_notification
    on ticket_kind_notifications using hash (notification_id);
create index ticket_kind_notifications_by_ticket_kind
    on ticket_kind_notifications using hash (ticket_kind_id);
create index activity_admin_access_by_access_group
    on allow_admins_from_group_view_activities using hash (access_group_id);

-- tickets
create index ticket_kind_start on ticket_kinds using btree (purchasing_available_start);
create index ticket_queuer_placement on ticket_reservation_queuers using btree (placement);
create index ticket_queuer_timeout on ticket_release_queuers using btree (started_queueing);
create index ticket_reservation_timeout on ticket_reservations using btree (timeout);
create index ticket_release_queuers_ticket_id on ticket_release_queuers using hash (ticket_kind_id);
create index ticket_reserved_transaction on ticket_reservations using hash (transaction_id);
create index ticket_purchased_transaction on purchased_tickets using hash (transaction_id);
create index purchased_tickets_by_owner_and_ticket_kind
    on purchased_tickets (owner_id, ticket_kind_id);
create index purchased_tickets_by_ticket_kind
    on purchased_tickets using hash (ticket_kind_id);
