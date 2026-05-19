alter table "public"."ticket_kinds" drop constraint "ticket_kinds_max_tickets_check";

alter table "public"."ticket_kinds" drop constraint "ticket_kinds_min_tickets_check";

CREATE INDEX activity_hosts_by_activity ON public.activity_hosts USING hash (activity_id);

CREATE INDEX activity_time_end ON public.activities USING btree (time_end);

CREATE INDEX activity_time_start ON public.activities USING btree (time_start);

CREATE INDEX group_members_by_member ON public.group_members USING hash (member_id);

CREATE INDEX group_tree ON public.groups USING gist (admin_path);

CREATE INDEX group_tree_eq ON public.groups USING hash (admin_path);

CREATE INDEX ticket_kind_by_activity ON public.ticket_kinds USING hash (activity_id);

CREATE INDEX user_group_settings_by_group ON public.user_group_settings USING hash (group_id);

CREATE INDEX user_group_settings_by_user ON public.user_group_settings USING hash (user_id);

alter table "public"."ticket_kinds" add constraint "ticket_kinds_max_tickets_check" CHECK ((max_tickets >= 0)) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_max_tickets_check";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_min_tickets_check" CHECK ((min_tickets >= 0)) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_min_tickets_check";


