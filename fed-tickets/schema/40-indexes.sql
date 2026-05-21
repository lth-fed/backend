-- this file is written in chronological order. Please look for indexes which do the thing you want to do before adding more

-- when writing the activity details & activity list APIs
create index group_memberships_by_member on group_memberships using hash (user_id);
create index group_tree on groups using gist (path);
create index group_tree_eq on groups using hash (path);
create index user_group_settings_by_user on user_group_settings using hash (user_id);
create index user_group_settings_by_group on user_group_settings using hash (group_id);
create index activity_time_start on activities using btree (time_start);
create index activity_time_end on activities using btree (time_end);
create index activity_hosts_by_activity on activity_hosts using hash (activity_id);
create index ticket_kind_by_activity on ticket_kinds using hash (activity_id);
create index ticket_kind_allowed_groups_by_group on ticket_kind_allowed_groups using hash (group_id)
