create extension if not exists "ltree" with schema "public" version '1.3';

create type "public"."location" as (
    "name" jsonb,
    "directions" jsonb,
    "coordinate_wgs84" point,
    "url" text
);

create table "public"."activities" (
    "id" uuid not null default uuidv4(),
    "responsible_id" text not null,
    "responsible_contact" text not null,
    "creator_id" uuid not null,
    "title" jsonb not null,
    "description" jsonb not null,
    "location" location not null,
    "time_start" timestamp with time zone not null,
    "time_end" timestamp with time zone not null,
    "image_id" uuid not null,
    "is_hidden" boolean not null default true,
    "is_hidden_for_other_admins" boolean not null default false,
    "max_tickets" integer not null
);


create table "public"."activity_host_invites" (
    "activity_id" uuid not null,
    "group_id" uuid not null
);


create table "public"."activity_hosts" (
    "activity_id" uuid not null,
    "group_id" uuid not null
);


create table "public"."activity_verifiers" (
    "activity_id" uuid not null,
    "user_id" text not null
);


create table "public"."allow_admins_from_group_view_activities" (
    "host_group_id" uuid not null,
    "access_group_id" uuid not null
);


create table "public"."group_adminships" (
    "user_id" text not null,
    "group_id" uuid not null
);


create table "public"."group_member_requests" (
    "member_id" text not null,
    "group_id" uuid not null
);


create table "public"."group_memberships" (
    "user_id" text not null,
    "group_id" uuid not null
);


create table "public"."groups" (
    "id" uuid not null default uuidv4(),
    "path" ltree not null,
    "parent_path" ltree generated always as (
CASE
    WHEN (nlevel(path) > 1) THEN subpath(path, 0, (nlevel(path) - 1))
    ELSE NULL::ltree
END) stored,
    "limit_membership_visibility" boolean not null,
    "name" jsonb not null,
    "description" jsonb not null,
    "logo_id" uuid not null,
    "deleted" boolean not null default false
);


create table "public"."groups_ask_to_join" (
    "target_id" uuid not null,
    "joiner_id" uuid not null
);


create table "public"."images" (
    "id" uuid not null default uuidv4(),
    "created" timestamp with time zone not null default now(),
    "size" bigint not null,
    "url" text not null
);


create table "public"."notifications" (
    "id" uuid not null,
    "title" jsonb not null,
    "content" jsonb not null,
    "send_at" timestamp with time zone not null
);


create table "public"."purchased_ticket_addons" (
    "addon_id" uuid not null,
    "ticket_id" uuid not null,
    "selected_options" integer[] not null,
    "selected_text" text not null
);


create table "public"."purchased_ticket_validations" (
    "id" uuid not null,
    "purchased_ticket_id" uuid not null,
    "timestamp" timestamp with time zone not null default now()
);


create table "public"."purchased_tickets" (
    "id" uuid not null default uuidv4(),
    "ticket_kind_id" uuid not null,
    "purchaser_id" text not null,
    "owner_id" text not null,
    "transaction_id" uuid not null
);


create table "public"."push_devices" (
    "device_id" uuid not null,
    "user_id" text not null,
    "push_token" text not null,
    "platform" push_platform not null,
    "updated_at" timestamp with time zone not null default now()
);


create table "public"."ticket_addon_options" (
    "id" uuid not null,
    "ticket_addon_id" uuid not null,
    "idx" integer not null,
    "name" jsonb not null,
    "price" money not null,
    "bookkeeping_prices" money[] not null,
    "bookkeeping_price_categories" text[] not null
);


create table "public"."ticket_addons" (
    "id" uuid not null,
    "ticket_kind_id" uuid not null,
    "idx" integer not null,
    "name" jsonb not null,
    "multiple_alternatives" boolean not null,
    "has_text_field" boolean not null,
    "required" boolean not null
);


create table "public"."ticket_kind_allowed_groups" (
    "ticket_kind_id" uuid not null,
    "group_id" uuid not null
);


create table "public"."ticket_kind_notifications" (
    "id" text not null default 'release'::text,
    "ticket_kind_id" uuid not null,
    "notification_id" uuid not null
);


create table "public"."ticket_kinds" (
    "id" uuid not null default uuidv4(),
    "activity_id" uuid not null,
    "name" jsonb not null,
    "price" money not null,
    "purchasing_available_start" timestamp with time zone not null,
    "purchasing_available_stop" timestamp with time zone not null,
    "max_tickets" integer not null,
    "min_tickets" integer not null,
    "reserved_or_purchased_tickets" integer not null,
    "allow_transfer_ticket_start" timestamp with time zone not null,
    "allow_transfer_ticket_stop" timestamp with time zone not null,
    "allow_transfer_ticket_bypass_allowed_groups" boolean not null,
    "has_been_purchased" boolean not null,
    "has_been_released" boolean not null
);


create table "public"."ticket_release_queuers" (
    "user_id" text not null,
    "ticket_kind_id" uuid not null,
    "started_queueing" timestamp with time zone not null
);


create table "public"."ticket_reservation_addons" (
    "addon_id" uuid not null,
    "ticket_id" uuid not null,
    "selected_options" integer[] not null,
    "selected_text" text not null
);


create table "public"."ticket_reservation_queuers" (
    "user_id" text not null,
    "ticket_kind_id" uuid not null,
    "placement" integer not null
);


create table "public"."ticket_reservations" (
    "id" uuid not null default uuidv4(),
    "user_id" text,
    "ticket_kind_id" uuid not null,
    "transaction_id" uuid,
    "timeout" timestamp with time zone not null
);


create table "public"."user_group_settings" (
    "user_id" text not null,
    "group_id" uuid not null,
    "visible" boolean not null,
    "notification_level" notification_level not null
);


create table "public"."users" (
    "id" text not null,
    "name" bytea not null,
    "language" bytea not null,
    "nonce" bytea not null,
    "latest_refresh" timestamp with time zone not null default now(),
    "creation" timestamp with time zone not null default now(),
    "inactive_since" timestamp with time zone
);


CREATE UNIQUE INDEX activities_pkey ON public.activities USING btree (id);

CREATE INDEX activity_admin_access_by_access_group ON public.allow_admins_from_group_view_activities USING hash (access_group_id);

CREATE UNIQUE INDEX activity_host_invites_pkey ON public.activity_host_invites USING btree (activity_id, group_id);

CREATE INDEX activity_hosts_by_activity ON public.activity_hosts USING hash (activity_id);

CREATE UNIQUE INDEX activity_hosts_pkey ON public.activity_hosts USING btree (activity_id, group_id);

CREATE INDEX activity_time_end ON public.activities USING btree (time_end);

CREATE INDEX activity_time_start ON public.activities USING btree (time_start);

CREATE UNIQUE INDEX activity_verifiers_pkey ON public.activity_verifiers USING btree (activity_id, user_id);

CREATE UNIQUE INDEX allow_admins_from_group_view_activities_pkey ON public.allow_admins_from_group_view_activities USING btree (host_group_id, access_group_id);

CREATE UNIQUE INDEX group_adminships_pkey ON public.group_adminships USING btree (user_id, group_id);

CREATE UNIQUE INDEX group_member_requests_pkey ON public.group_member_requests USING btree (group_id, member_id);

CREATE INDEX group_memberships_by_group ON public.group_memberships USING hash (group_id);

CREATE INDEX group_memberships_by_member ON public.group_memberships USING hash (user_id);

CREATE UNIQUE INDEX group_memberships_pkey ON public.group_memberships USING btree (user_id, group_id);

CREATE INDEX group_tree ON public.groups USING gist (path);

CREATE INDEX group_tree_eq ON public.groups USING hash (path);

CREATE UNIQUE INDEX groups_ask_to_join_pkey ON public.groups_ask_to_join USING btree (target_id, joiner_id);

CREATE INDEX groups_path_gist ON public.groups USING gist (path);

CREATE UNIQUE INDEX groups_path_key ON public.groups USING btree (path);

CREATE UNIQUE INDEX groups_pkey ON public.groups USING btree (id);

CREATE UNIQUE INDEX images_pkey ON public.images USING btree (id);

CREATE UNIQUE INDEX notifications_pkey ON public.notifications USING btree (id);

CREATE INDEX notifications_send_time ON public.notifications USING btree (send_at);

CREATE UNIQUE INDEX purchased_ticket_addons_pkey ON public.purchased_ticket_addons USING btree (ticket_id, addon_id);

CREATE UNIQUE INDEX purchased_ticket_validations_pkey ON public.purchased_ticket_validations USING btree (id);

CREATE INDEX purchased_tickets_by_owner_and_ticket_kind ON public.purchased_tickets USING btree (owner_id, ticket_kind_id);

CREATE INDEX purchased_tickets_by_ticket_kind ON public.purchased_tickets USING hash (ticket_kind_id);

CREATE UNIQUE INDEX purchased_tickets_pkey ON public.purchased_tickets USING btree (id);

CREATE UNIQUE INDEX push_devices_pkey ON public.push_devices USING btree (device_id);

CREATE UNIQUE INDEX push_devices_platform_push_token_key ON public.push_devices USING btree (platform, push_token);

CREATE UNIQUE INDEX ticket_addon_options_pkey ON public.ticket_addon_options USING btree (id);

CREATE UNIQUE INDEX ticket_addons_pkey ON public.ticket_addons USING btree (id);

CREATE INDEX ticket_kind_allowed_groups_by_group ON public.ticket_kind_allowed_groups USING hash (group_id);

CREATE INDEX ticket_kind_allowed_groups_by_ticket_kind ON public.ticket_kind_allowed_groups USING hash (ticket_kind_id);

CREATE UNIQUE INDEX ticket_kind_allowed_groups_pkey ON public.ticket_kind_allowed_groups USING btree (group_id, ticket_kind_id);

CREATE INDEX ticket_kind_by_activity ON public.ticket_kinds USING hash (activity_id);

CREATE INDEX ticket_kind_notifications_by_notification ON public.ticket_kind_notifications USING hash (notification_id);

CREATE INDEX ticket_kind_notifications_by_ticket_kind ON public.ticket_kind_notifications USING hash (ticket_kind_id);

CREATE UNIQUE INDEX ticket_kind_notifications_pkey ON public.ticket_kind_notifications USING btree (id, ticket_kind_id);

CREATE INDEX ticket_kind_start ON public.ticket_kinds USING btree (purchasing_available_start);

CREATE UNIQUE INDEX ticket_kinds_pkey ON public.ticket_kinds USING btree (id);

CREATE INDEX ticket_purchased_transaction ON public.purchased_tickets USING hash (transaction_id);

CREATE INDEX ticket_queuer_placement ON public.ticket_reservation_queuers USING btree (placement);

CREATE INDEX ticket_queuer_timeout ON public.ticket_release_queuers USING btree (started_queueing);

CREATE UNIQUE INDEX ticket_release_queuers_pkey ON public.ticket_release_queuers USING btree (user_id);

CREATE INDEX ticket_release_queuers_ticket_id ON public.ticket_release_queuers USING hash (ticket_kind_id);

CREATE UNIQUE INDEX ticket_reservation_addons_pkey ON public.ticket_reservation_addons USING btree (ticket_id, addon_id);

CREATE UNIQUE INDEX ticket_reservation_queuers_pkey ON public.ticket_reservation_queuers USING btree (user_id);

CREATE UNIQUE INDEX ticket_reservation_queuers_ticket_kind_id_placement_key ON public.ticket_reservation_queuers USING btree (ticket_kind_id, placement);

CREATE INDEX ticket_reservation_timeout ON public.ticket_reservations USING btree (timeout);

CREATE UNIQUE INDEX ticket_reservations_pkey ON public.ticket_reservations USING btree (id);

CREATE UNIQUE INDEX ticket_reservations_user_id_key ON public.ticket_reservations USING btree (user_id);

CREATE INDEX ticket_reserved_transaction ON public.ticket_reservations USING hash (transaction_id);

CREATE INDEX user_group_settings_by_group ON public.user_group_settings USING hash (group_id);

CREATE INDEX user_group_settings_by_user ON public.user_group_settings USING hash (user_id);

CREATE UNIQUE INDEX user_group_settings_pkey ON public.user_group_settings USING btree (group_id, user_id);

CREATE UNIQUE INDEX users_pkey ON public.users USING btree (id);

alter table "public"."activities" add constraint "activities_pkey" PRIMARY KEY using index "activities_pkey";

alter table "public"."activity_host_invites" add constraint "activity_host_invites_pkey" PRIMARY KEY using index "activity_host_invites_pkey";

alter table "public"."activity_hosts" add constraint "activity_hosts_pkey" PRIMARY KEY using index "activity_hosts_pkey";

alter table "public"."activity_verifiers" add constraint "activity_verifiers_pkey" PRIMARY KEY using index "activity_verifiers_pkey";

alter table "public"."allow_admins_from_group_view_activities" add constraint "allow_admins_from_group_view_activities_pkey" PRIMARY KEY using index "allow_admins_from_group_view_activities_pkey";

alter table "public"."group_adminships" add constraint "group_adminships_pkey" PRIMARY KEY using index "group_adminships_pkey";

alter table "public"."group_member_requests" add constraint "group_member_requests_pkey" PRIMARY KEY using index "group_member_requests_pkey";

alter table "public"."group_memberships" add constraint "group_memberships_pkey" PRIMARY KEY using index "group_memberships_pkey";

alter table "public"."groups" add constraint "groups_pkey" PRIMARY KEY using index "groups_pkey";

alter table "public"."groups_ask_to_join" add constraint "groups_ask_to_join_pkey" PRIMARY KEY using index "groups_ask_to_join_pkey";

alter table "public"."images" add constraint "images_pkey" PRIMARY KEY using index "images_pkey";

alter table "public"."notifications" add constraint "notifications_pkey" PRIMARY KEY using index "notifications_pkey";

alter table "public"."purchased_ticket_addons" add constraint "purchased_ticket_addons_pkey" PRIMARY KEY using index "purchased_ticket_addons_pkey";

alter table "public"."purchased_ticket_validations" add constraint "purchased_ticket_validations_pkey" PRIMARY KEY using index "purchased_ticket_validations_pkey";

alter table "public"."purchased_tickets" add constraint "purchased_tickets_pkey" PRIMARY KEY using index "purchased_tickets_pkey";

alter table "public"."push_devices" add constraint "push_devices_pkey" PRIMARY KEY using index "push_devices_pkey";

alter table "public"."ticket_addon_options" add constraint "ticket_addon_options_pkey" PRIMARY KEY using index "ticket_addon_options_pkey";

alter table "public"."ticket_addons" add constraint "ticket_addons_pkey" PRIMARY KEY using index "ticket_addons_pkey";

alter table "public"."ticket_kind_allowed_groups" add constraint "ticket_kind_allowed_groups_pkey" PRIMARY KEY using index "ticket_kind_allowed_groups_pkey";

alter table "public"."ticket_kind_notifications" add constraint "ticket_kind_notifications_pkey" PRIMARY KEY using index "ticket_kind_notifications_pkey";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_pkey" PRIMARY KEY using index "ticket_kinds_pkey";

alter table "public"."ticket_release_queuers" add constraint "ticket_release_queuers_pkey" PRIMARY KEY using index "ticket_release_queuers_pkey";

alter table "public"."ticket_reservation_addons" add constraint "ticket_reservation_addons_pkey" PRIMARY KEY using index "ticket_reservation_addons_pkey";

alter table "public"."ticket_reservation_queuers" add constraint "ticket_reservation_queuers_pkey" PRIMARY KEY using index "ticket_reservation_queuers_pkey";

alter table "public"."ticket_reservations" add constraint "ticket_reservations_pkey" PRIMARY KEY using index "ticket_reservations_pkey";

alter table "public"."user_group_settings" add constraint "user_group_settings_pkey" PRIMARY KEY using index "user_group_settings_pkey";

alter table "public"."users" add constraint "users_pkey" PRIMARY KEY using index "users_pkey";

alter table "public"."activities" add constraint "activities_creator_id_fkey" FOREIGN KEY ("creator_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."activities" validate constraint "activities_creator_id_fkey";

alter table "public"."activities" add constraint "activities_image_id_fkey" FOREIGN KEY ("image_id") REFERENCES "public"."images"("id") NOT VALID;

alter table "public"."activities" validate constraint "activities_image_id_fkey";

alter table "public"."activities" add constraint "activities_max_tickets_check" CHECK ((max_tickets >= 0)) not valid;

alter table "public"."activities" validate constraint "activities_max_tickets_check";

alter table "public"."activities" add constraint "activities_responsible_id_fkey" FOREIGN KEY ("responsible_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."activities" validate constraint "activities_responsible_id_fkey";

alter table "public"."activity_host_invites" add constraint "activity_host_invites_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."activity_host_invites" validate constraint "activity_host_invites_activity_id_fkey";

alter table "public"."activity_host_invites" add constraint "activity_host_invites_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."activity_host_invites" validate constraint "activity_host_invites_group_id_fkey";

alter table "public"."activity_hosts" add constraint "activity_hosts_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."activity_hosts" validate constraint "activity_hosts_activity_id_fkey";

alter table "public"."activity_hosts" add constraint "activity_hosts_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."activity_hosts" validate constraint "activity_hosts_group_id_fkey";

alter table "public"."activity_verifiers" add constraint "activity_verifiers_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."activity_verifiers" validate constraint "activity_verifiers_activity_id_fkey";

alter table "public"."activity_verifiers" add constraint "activity_verifiers_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."activity_verifiers" validate constraint "activity_verifiers_user_id_fkey";

alter table "public"."allow_admins_from_group_view_activities" add constraint "allow_admins_from_group_view_activities_access_group_id_fkey" FOREIGN KEY ("access_group_id") REFERENCES "public"."groups"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."allow_admins_from_group_view_activities" validate constraint "allow_admins_from_group_view_activities_access_group_id_fkey";

alter table "public"."allow_admins_from_group_view_activities" add constraint "allow_admins_from_group_view_activities_host_group_id_fkey" FOREIGN KEY ("host_group_id") REFERENCES "public"."groups"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."allow_admins_from_group_view_activities" validate constraint "allow_admins_from_group_view_activities_host_group_id_fkey";

alter table "public"."group_adminships" add constraint "group_adminships_email_account" CHECK ((user_id ~~ 'email:%'::text)) not valid;

alter table "public"."group_adminships" validate constraint "group_adminships_email_account";

alter table "public"."group_adminships" add constraint "group_adminships_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."group_adminships" validate constraint "group_adminships_group_id_fkey";

alter table "public"."group_adminships" add constraint "group_adminships_group_membership_fk" FOREIGN KEY ("user_id", "group_id") REFERENCES "public"."group_memberships"("user_id", "group_id") ON DELETE CASCADE NOT VALID;

alter table "public"."group_adminships" validate constraint "group_adminships_group_membership_fk";

alter table "public"."group_adminships" add constraint "group_adminships_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."group_adminships" validate constraint "group_adminships_user_id_fkey";

alter table "public"."group_member_requests" add constraint "group_member_requests_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."group_member_requests" validate constraint "group_member_requests_group_id_fkey";

alter table "public"."group_member_requests" add constraint "group_member_requests_member_id_fkey" FOREIGN KEY ("member_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."group_member_requests" validate constraint "group_member_requests_member_id_fkey";

alter table "public"."group_memberships" add constraint "group_memberships_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."group_memberships" validate constraint "group_memberships_group_id_fkey";

alter table "public"."group_memberships" add constraint "group_memberships_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."group_memberships" validate constraint "group_memberships_user_id_fkey";

alter table "public"."groups" add constraint "groups_logo_id_fkey" FOREIGN KEY ("logo_id") REFERENCES "public"."images"("id") NOT VALID;

alter table "public"."groups" validate constraint "groups_logo_id_fkey";

alter table "public"."groups" add constraint "groups_parent_path_fkey" FOREIGN KEY ("parent_path") REFERENCES "public"."groups"("path") NOT VALID;

alter table "public"."groups" validate constraint "groups_parent_path_fkey";

alter table "public"."groups" add constraint "groups_path_key" UNIQUE using index "groups_path_key";

alter table "public"."groups_ask_to_join" add constraint "groups_ask_to_join_joiner_id_fkey" FOREIGN KEY ("joiner_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."groups_ask_to_join" validate constraint "groups_ask_to_join_joiner_id_fkey";

alter table "public"."groups_ask_to_join" add constraint "groups_ask_to_join_target_id_fkey" FOREIGN KEY ("target_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."groups_ask_to_join" validate constraint "groups_ask_to_join_target_id_fkey";

alter table "public"."purchased_ticket_addons" add constraint "purchased_ticket_addons_addon_id_fkey" FOREIGN KEY ("addon_id") REFERENCES "public"."ticket_addons"("id") NOT VALID;

alter table "public"."purchased_ticket_addons" validate constraint "purchased_ticket_addons_addon_id_fkey";

alter table "public"."purchased_ticket_addons" add constraint "purchased_ticket_addons_ticket_id_fkey" FOREIGN KEY ("ticket_id") REFERENCES "public"."purchased_tickets"("id") NOT VALID;

alter table "public"."purchased_ticket_addons" validate constraint "purchased_ticket_addons_ticket_id_fkey";

alter table "public"."purchased_ticket_validations" add constraint "purchased_ticket_validations_purchased_ticket_id_fkey" FOREIGN KEY ("purchased_ticket_id") REFERENCES "public"."purchased_tickets"("id") NOT VALID;

alter table "public"."purchased_ticket_validations" validate constraint "purchased_ticket_validations_purchased_ticket_id_fkey";

alter table "public"."purchased_tickets" add constraint "purchased_tickets_owner_id_fkey" FOREIGN KEY ("owner_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."purchased_tickets" validate constraint "purchased_tickets_owner_id_fkey";

alter table "public"."purchased_tickets" add constraint "purchased_tickets_purchaser_id_fkey" FOREIGN KEY ("purchaser_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."purchased_tickets" validate constraint "purchased_tickets_purchaser_id_fkey";

alter table "public"."purchased_tickets" add constraint "purchased_tickets_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."ticket_kinds"("id") NOT VALID;

alter table "public"."purchased_tickets" validate constraint "purchased_tickets_ticket_kind_id_fkey";

alter table "public"."push_devices" add constraint "push_devices_platform_push_token_key" UNIQUE using index "push_devices_platform_push_token_key";

alter table "public"."push_devices" add constraint "push_devices_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."push_devices" validate constraint "push_devices_user_id_fkey";

alter table "public"."ticket_addon_options" add constraint "bookkeeping_lengths_consistent" CHECK ((cardinality(bookkeeping_prices) = cardinality(bookkeeping_price_categories))) not valid;

alter table "public"."ticket_addon_options" validate constraint "bookkeeping_lengths_consistent";

alter table "public"."ticket_addon_options" add constraint "bookkeeping_prices_add_up" CHECK ((array_sum_money(bookkeeping_prices) = price)) not valid;

alter table "public"."ticket_addon_options" validate constraint "bookkeeping_prices_add_up";

alter table "public"."ticket_addon_options" add constraint "ticket_addon_options_price_check" CHECK ((price >= (0)::money)) not valid;

alter table "public"."ticket_addon_options" validate constraint "ticket_addon_options_price_check";

alter table "public"."ticket_addon_options" add constraint "ticket_addon_options_ticket_addon_id_fkey" FOREIGN KEY ("ticket_addon_id") REFERENCES "public"."ticket_addons"("id") NOT VALID;

alter table "public"."ticket_addon_options" validate constraint "ticket_addon_options_ticket_addon_id_fkey";

alter table "public"."ticket_addons" add constraint "ticket_addons_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."ticket_kinds"("id") NOT VALID;

alter table "public"."ticket_addons" validate constraint "ticket_addons_ticket_kind_id_fkey";

alter table "public"."ticket_kind_allowed_groups" add constraint "ticket_kind_allowed_groups_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."ticket_kind_allowed_groups" validate constraint "ticket_kind_allowed_groups_group_id_fkey";

alter table "public"."ticket_kind_allowed_groups" add constraint "ticket_kind_allowed_groups_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."ticket_kinds"("id") NOT VALID;

alter table "public"."ticket_kind_allowed_groups" validate constraint "ticket_kind_allowed_groups_ticket_kind_id_fkey";

alter table "public"."ticket_kind_notifications" add constraint "ticket_kind_notifications_notification_id_fkey" FOREIGN KEY ("notification_id") REFERENCES "public"."notifications"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."ticket_kind_notifications" validate constraint "ticket_kind_notifications_notification_id_fkey";

alter table "public"."ticket_kind_notifications" add constraint "ticket_kind_notifications_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."ticket_kinds"("id") NOT VALID;

alter table "public"."ticket_kind_notifications" validate constraint "ticket_kind_notifications_ticket_kind_id_fkey";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_activity_id_fkey";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_check" CHECK ((((min_tickets = 0) AND (max_tickets = 0)) OR (min_tickets < max_tickets))) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_check";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_check1" CHECK (((reserved_or_purchased_tickets >= 0) AND (reserved_or_purchased_tickets <= max_tickets))) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_check1";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_max_tickets_check" CHECK ((max_tickets >= 0)) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_max_tickets_check";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_min_tickets_check" CHECK ((min_tickets >= 0)) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_min_tickets_check";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_price_check" CHECK ((price >= (0)::money)) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_price_check";

alter table "public"."ticket_release_queuers" add constraint "ticket_release_queuers_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."ticket_kinds"("id") NOT VALID;

alter table "public"."ticket_release_queuers" validate constraint "ticket_release_queuers_ticket_kind_id_fkey";

alter table "public"."ticket_release_queuers" add constraint "ticket_release_queuers_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."ticket_release_queuers" validate constraint "ticket_release_queuers_user_id_fkey";

alter table "public"."ticket_reservation_addons" add constraint "ticket_reservation_addons_addon_id_fkey" FOREIGN KEY ("addon_id") REFERENCES "public"."ticket_addons"("id") NOT VALID;

alter table "public"."ticket_reservation_addons" validate constraint "ticket_reservation_addons_addon_id_fkey";

alter table "public"."ticket_reservation_addons" add constraint "ticket_reservation_addons_ticket_id_fkey" FOREIGN KEY ("ticket_id") REFERENCES "public"."ticket_reservations"("id") NOT VALID;

alter table "public"."ticket_reservation_addons" validate constraint "ticket_reservation_addons_ticket_id_fkey";

alter table "public"."ticket_reservation_queuers" add constraint "ticket_reservation_queuers_placement_check" CHECK ((placement >= 0)) not valid;

alter table "public"."ticket_reservation_queuers" validate constraint "ticket_reservation_queuers_placement_check";

alter table "public"."ticket_reservation_queuers" add constraint "ticket_reservation_queuers_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."ticket_kinds"("id") NOT VALID;

alter table "public"."ticket_reservation_queuers" validate constraint "ticket_reservation_queuers_ticket_kind_id_fkey";

alter table "public"."ticket_reservation_queuers" add constraint "ticket_reservation_queuers_ticket_kind_id_placement_key" UNIQUE using index "ticket_reservation_queuers_ticket_kind_id_placement_key";

alter table "public"."ticket_reservation_queuers" add constraint "ticket_reservation_queuers_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."ticket_reservation_queuers" validate constraint "ticket_reservation_queuers_user_id_fkey";

alter table "public"."ticket_reservations" add constraint "ticket_reservations_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."ticket_kinds"("id") NOT VALID;

alter table "public"."ticket_reservations" validate constraint "ticket_reservations_ticket_kind_id_fkey";

alter table "public"."ticket_reservations" add constraint "ticket_reservations_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."ticket_reservations" validate constraint "ticket_reservations_user_id_fkey";

alter table "public"."ticket_reservations" add constraint "ticket_reservations_user_id_key" UNIQUE using index "ticket_reservations_user_id_key";

alter table "public"."user_group_settings" add constraint "user_group_settings_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."user_group_settings" validate constraint "user_group_settings_group_id_fkey";

alter table "public"."user_group_settings" add constraint "user_group_settings_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."user_group_settings" validate constraint "user_group_settings_user_id_fkey";


