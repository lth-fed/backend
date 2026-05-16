create extension if not exists "ltree" with schema "public" version '1.3';

CREATE OR REPLACE FUNCTION public.array_sum_money(money[])
 RETURNS money
 LANGUAGE sql
 IMMUTABLE STRICT
AS $function$SELECT sum(e) FROM unnest($1) AS a(e)$function$
;

create type "public"."notification_level" as enum ('none', 'personalized', 'all');

create type "public"."location" as (
    "name" jsonb,
    "directions" jsonb,
    "coordinate_wgs84" point,
    "url" text
);

create table "public"."activities" (
    "id" uuid not null,
    "responsible_id" text not null,
    "creator_id" ltree not null,
    "title" jsonb not null,
    "description" jsonb not null,
    "location" location not null,
    "time_start" timestamp with time zone not null,
    "time_end" timestamp with time zone not null,
    "image_id" uuid not null,
    "is_hidden" boolean not null,
    "max_tickets" integer not null
);


create table "public"."activity_host_invites" (
    "activity_id" uuid not null,
    "group_id" ltree not null
);


create table "public"."activity_hosts" (
    "activity_id" uuid not null,
    "group_id" ltree not null
);


create table "public"."group_member_requests" (
    "member_id" text not null,
    "group_id" ltree not null
);


create table "public"."group_members" (
    "member_id" text not null,
    "group_id" ltree not null,
    "is_admin" boolean not null
);


create table "public"."groups" (
    "admin_path" ltree not null,
    "limit_membership_visibility" boolean not null,
    "name" jsonb not null,
    "description" jsonb not null,
    "logo_id" uuid not null,
    "deleted" boolean not null
);


create table "public"."groups_ask_to_join" (
    "target_id" ltree not null,
    "joiner_id" ltree not null
);


create table "public"."images" (
    "id" uuid not null,
    "created" timestamp with time zone not null,
    "size" bigint not null,
    "url" text not null
);


create table "public"."purchased_ticket_addons" (
    "id" uuid not null,
    "addon_id" uuid not null,
    "ticket_id" uuid not null,
    "selected_options" integer[] not null,
    "selected_text" text not null
);


create table "public"."purchased_ticket_validations" (
    "id" uuid not null,
    "purchased_ticket_id" uuid not null,
    "timestamptz" timestamp with time zone not null
);


create table "public"."purchased_tickets" (
    "id" uuid not null,
    "ticket_kind_id" uuid not null,
    "purchaser_id" text not null,
    "owner_id" text not null
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
    "multiple_alternatives" boolean not null,
    "has_text_field" boolean not null,
    "required" boolean not null
);


create table "public"."ticket_kind_allowed_groups" (
    "ticket_kind_id" uuid not null,
    "group_id" ltree not null
);


create table "public"."ticket_kinds" (
    "id" uuid not null,
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
    "has_been_purchased" boolean not null
);


create table "public"."ticket_queue" (
    "id" uuid not null,
    "ticket_id" uuid not null,
    "user_id" text not null,
    "placement" integer not null
);


create table "public"."ticket_queuers" (
    "id" uuid not null,
    "ticket_id" uuid not null,
    "user_id" text not null,
    "started_queueing" timestamp with time zone not null
);


create table "public"."ticket_reservations" (
    "id" uuid not null,
    "ticket_id" uuid not null,
    "user_id" text not null,
    "timeout" timestamp with time zone not null
);


create table "public"."user_group_settings" (
    "user_id" text not null,
    "group_id" ltree not null,
    "notification_level" notification_level not null,
    "visible" boolean not null
);


create table "public"."users" (
    "id" text not null,
    "name" bytea not null,
    "language" bytea not null,
    "nonce" bytea not null,
    "latest_refresh" timestamp with time zone not null,
    "creation" timestamp with time zone not null,
    "inactive_since" timestamp with time zone
);


CREATE UNIQUE INDEX activities_pkey ON public.activities USING btree (id);

CREATE UNIQUE INDEX activity_host_invites_pkey ON public.activity_host_invites USING btree (activity_id, group_id);

CREATE UNIQUE INDEX activity_hosts_pkey ON public.activity_hosts USING btree (activity_id, group_id);

CREATE UNIQUE INDEX group_member_requests_pkey ON public.group_member_requests USING btree (group_id, member_id);

CREATE UNIQUE INDEX group_members_pkey ON public.group_members USING btree (group_id, member_id, is_admin);

CREATE UNIQUE INDEX groups_ask_to_join_pkey ON public.groups_ask_to_join USING btree (target_id, joiner_id);

CREATE UNIQUE INDEX groups_pkey ON public.groups USING btree (admin_path);

CREATE UNIQUE INDEX images_pkey ON public.images USING btree (id);

CREATE UNIQUE INDEX max_one_ticket_per_person_per_activity ON public.purchased_tickets USING btree (ticket_kind_id, owner_id);

CREATE UNIQUE INDEX purchased_ticket_addons_pkey ON public.purchased_ticket_addons USING btree (id);

CREATE UNIQUE INDEX purchased_ticket_validations_pkey ON public.purchased_ticket_validations USING btree (id);

CREATE UNIQUE INDEX purchased_tickets_pkey ON public.purchased_tickets USING btree (id);

CREATE UNIQUE INDEX ticket_addon_options_pkey ON public.ticket_addon_options USING btree (id);

CREATE UNIQUE INDEX ticket_addons_pkey ON public.ticket_addons USING btree (id);

CREATE UNIQUE INDEX ticket_kind_allowed_groups_pkey ON public.ticket_kind_allowed_groups USING btree (group_id, ticket_kind_id);

CREATE UNIQUE INDEX ticket_kinds_pkey ON public.ticket_kinds USING btree (id);

CREATE UNIQUE INDEX ticket_queue_pkey ON public.ticket_queue USING btree (id);

CREATE UNIQUE INDEX ticket_queuers_pkey ON public.ticket_queuers USING btree (id);

CREATE UNIQUE INDEX ticket_reservations_pkey ON public.ticket_reservations USING btree (id);

CREATE UNIQUE INDEX user_group_settings_pkey ON public.user_group_settings USING btree (group_id, user_id);

CREATE UNIQUE INDEX users_pkey ON public.users USING btree (id);

alter table "public"."activities" add constraint "activities_pkey" PRIMARY KEY using index "activities_pkey";

alter table "public"."activity_host_invites" add constraint "activity_host_invites_pkey" PRIMARY KEY using index "activity_host_invites_pkey";

alter table "public"."activity_hosts" add constraint "activity_hosts_pkey" PRIMARY KEY using index "activity_hosts_pkey";

alter table "public"."group_member_requests" add constraint "group_member_requests_pkey" PRIMARY KEY using index "group_member_requests_pkey";

alter table "public"."group_members" add constraint "group_members_pkey" PRIMARY KEY using index "group_members_pkey";

alter table "public"."groups" add constraint "groups_pkey" PRIMARY KEY using index "groups_pkey";

alter table "public"."groups_ask_to_join" add constraint "groups_ask_to_join_pkey" PRIMARY KEY using index "groups_ask_to_join_pkey";

alter table "public"."images" add constraint "images_pkey" PRIMARY KEY using index "images_pkey";

alter table "public"."purchased_ticket_addons" add constraint "purchased_ticket_addons_pkey" PRIMARY KEY using index "purchased_ticket_addons_pkey";

alter table "public"."purchased_ticket_validations" add constraint "purchased_ticket_validations_pkey" PRIMARY KEY using index "purchased_ticket_validations_pkey";

alter table "public"."purchased_tickets" add constraint "purchased_tickets_pkey" PRIMARY KEY using index "purchased_tickets_pkey";

alter table "public"."ticket_addon_options" add constraint "ticket_addon_options_pkey" PRIMARY KEY using index "ticket_addon_options_pkey";

alter table "public"."ticket_addons" add constraint "ticket_addons_pkey" PRIMARY KEY using index "ticket_addons_pkey";

alter table "public"."ticket_kind_allowed_groups" add constraint "ticket_kind_allowed_groups_pkey" PRIMARY KEY using index "ticket_kind_allowed_groups_pkey";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_pkey" PRIMARY KEY using index "ticket_kinds_pkey";

alter table "public"."ticket_queue" add constraint "ticket_queue_pkey" PRIMARY KEY using index "ticket_queue_pkey";

alter table "public"."ticket_queuers" add constraint "ticket_queuers_pkey" PRIMARY KEY using index "ticket_queuers_pkey";

alter table "public"."ticket_reservations" add constraint "ticket_reservations_pkey" PRIMARY KEY using index "ticket_reservations_pkey";

alter table "public"."user_group_settings" add constraint "user_group_settings_pkey" PRIMARY KEY using index "user_group_settings_pkey";

alter table "public"."users" add constraint "users_pkey" PRIMARY KEY using index "users_pkey";

alter table "public"."activities" add constraint "activities_creator_id_fkey" FOREIGN KEY ("creator_id") REFERENCES "public"."groups"("admin_path") NOT VALID;

alter table "public"."activities" validate constraint "activities_creator_id_fkey";

alter table "public"."activities" add constraint "activities_image_id_fkey" FOREIGN KEY ("image_id") REFERENCES "public"."images"("id") NOT VALID;

alter table "public"."activities" validate constraint "activities_image_id_fkey";

alter table "public"."activities" add constraint "activities_max_tickets_check" CHECK ((max_tickets > 0)) not valid;

alter table "public"."activities" validate constraint "activities_max_tickets_check";

alter table "public"."activities" add constraint "activities_responsible_id_fkey" FOREIGN KEY ("responsible_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."activities" validate constraint "activities_responsible_id_fkey";

alter table "public"."activity_host_invites" add constraint "activity_host_invites_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."activity_host_invites" validate constraint "activity_host_invites_activity_id_fkey";

alter table "public"."activity_host_invites" add constraint "activity_host_invites_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("admin_path") NOT VALID;

alter table "public"."activity_host_invites" validate constraint "activity_host_invites_group_id_fkey";

alter table "public"."activity_hosts" add constraint "activity_hosts_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."activity_hosts" validate constraint "activity_hosts_activity_id_fkey";

alter table "public"."activity_hosts" add constraint "activity_hosts_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("admin_path") NOT VALID;

alter table "public"."activity_hosts" validate constraint "activity_hosts_group_id_fkey";

alter table "public"."group_member_requests" add constraint "group_member_requests_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("admin_path") NOT VALID;

alter table "public"."group_member_requests" validate constraint "group_member_requests_group_id_fkey";

alter table "public"."group_member_requests" add constraint "group_member_requests_member_id_fkey" FOREIGN KEY ("member_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."group_member_requests" validate constraint "group_member_requests_member_id_fkey";

alter table "public"."group_members" add constraint "group_members_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("admin_path") NOT VALID;

alter table "public"."group_members" validate constraint "group_members_group_id_fkey";

alter table "public"."group_members" add constraint "group_members_member_id_fkey" FOREIGN KEY ("member_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."group_members" validate constraint "group_members_member_id_fkey";

alter table "public"."groups" add constraint "groups_logo_id_fkey" FOREIGN KEY ("logo_id") REFERENCES "public"."images"("id") NOT VALID;

alter table "public"."groups" validate constraint "groups_logo_id_fkey";

alter table "public"."groups_ask_to_join" add constraint "groups_ask_to_join_joiner_id_fkey" FOREIGN KEY ("joiner_id") REFERENCES "public"."groups"("admin_path") NOT VALID;

alter table "public"."groups_ask_to_join" validate constraint "groups_ask_to_join_joiner_id_fkey";

alter table "public"."groups_ask_to_join" add constraint "groups_ask_to_join_target_id_fkey" FOREIGN KEY ("target_id") REFERENCES "public"."groups"("admin_path") NOT VALID;

alter table "public"."groups_ask_to_join" validate constraint "groups_ask_to_join_target_id_fkey";

alter table "public"."purchased_ticket_addons" add constraint "purchased_ticket_addons_addon_id_fkey" FOREIGN KEY ("addon_id") REFERENCES "public"."ticket_addons"("id") NOT VALID;

alter table "public"."purchased_ticket_addons" validate constraint "purchased_ticket_addons_addon_id_fkey";

alter table "public"."purchased_ticket_addons" add constraint "purchased_ticket_addons_ticket_id_fkey" FOREIGN KEY ("ticket_id") REFERENCES "public"."purchased_tickets"("id") NOT VALID;

alter table "public"."purchased_ticket_addons" validate constraint "purchased_ticket_addons_ticket_id_fkey";

alter table "public"."purchased_ticket_validations" add constraint "purchased_ticket_validations_purchased_ticket_id_fkey" FOREIGN KEY ("purchased_ticket_id") REFERENCES "public"."purchased_tickets"("id") NOT VALID;

alter table "public"."purchased_ticket_validations" validate constraint "purchased_ticket_validations_purchased_ticket_id_fkey";

alter table "public"."purchased_tickets" add constraint "max_one_ticket_per_person_per_activity" UNIQUE using index "max_one_ticket_per_person_per_activity";

alter table "public"."purchased_tickets" add constraint "purchased_tickets_owner_id_fkey" FOREIGN KEY ("owner_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."purchased_tickets" validate constraint "purchased_tickets_owner_id_fkey";

alter table "public"."purchased_tickets" add constraint "purchased_tickets_purchaser_id_fkey" FOREIGN KEY ("purchaser_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."purchased_tickets" validate constraint "purchased_tickets_purchaser_id_fkey";

alter table "public"."purchased_tickets" add constraint "purchased_tickets_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."ticket_kinds"("id") NOT VALID;

alter table "public"."purchased_tickets" validate constraint "purchased_tickets_ticket_kind_id_fkey";

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

alter table "public"."ticket_kind_allowed_groups" add constraint "ticket_kind_allowed_groups_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("admin_path") NOT VALID;

alter table "public"."ticket_kind_allowed_groups" validate constraint "ticket_kind_allowed_groups_group_id_fkey";

alter table "public"."ticket_kind_allowed_groups" add constraint "ticket_kind_allowed_groups_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."ticket_kinds"("id") NOT VALID;

alter table "public"."ticket_kind_allowed_groups" validate constraint "ticket_kind_allowed_groups_ticket_kind_id_fkey";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_activity_id_fkey";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_check" CHECK ((min_tickets < max_tickets)) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_check";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_check1" CHECK (((reserved_or_purchased_tickets >= 0) AND (reserved_or_purchased_tickets <= max_tickets))) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_check1";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_max_tickets_check" CHECK ((max_tickets > 0)) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_max_tickets_check";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_min_tickets_check" CHECK ((min_tickets > 0)) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_min_tickets_check";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_price_check" CHECK ((price >= (0)::money)) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_price_check";

alter table "public"."ticket_queue" add constraint "ticket_queue_placement_check" CHECK ((placement >= 0)) not valid;

alter table "public"."ticket_queue" validate constraint "ticket_queue_placement_check";

alter table "public"."ticket_queue" add constraint "ticket_queue_ticket_id_fkey" FOREIGN KEY ("ticket_id") REFERENCES "public"."ticket_kinds"("id") NOT VALID;

alter table "public"."ticket_queue" validate constraint "ticket_queue_ticket_id_fkey";

alter table "public"."ticket_queue" add constraint "ticket_queue_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."ticket_queue" validate constraint "ticket_queue_user_id_fkey";

alter table "public"."ticket_queuers" add constraint "ticket_queuers_ticket_id_fkey" FOREIGN KEY ("ticket_id") REFERENCES "public"."ticket_kinds"("id") NOT VALID;

alter table "public"."ticket_queuers" validate constraint "ticket_queuers_ticket_id_fkey";

alter table "public"."ticket_queuers" add constraint "ticket_queuers_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."ticket_queuers" validate constraint "ticket_queuers_user_id_fkey";

alter table "public"."ticket_reservations" add constraint "ticket_reservations_ticket_id_fkey" FOREIGN KEY ("ticket_id") REFERENCES "public"."ticket_kinds"("id") NOT VALID;

alter table "public"."ticket_reservations" validate constraint "ticket_reservations_ticket_id_fkey";

alter table "public"."ticket_reservations" add constraint "ticket_reservations_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."ticket_reservations" validate constraint "ticket_reservations_user_id_fkey";

alter table "public"."user_group_settings" add constraint "user_group_settings_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("admin_path") NOT VALID;

alter table "public"."user_group_settings" validate constraint "user_group_settings_group_id_fkey";

alter table "public"."user_group_settings" add constraint "user_group_settings_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."user_group_settings" validate constraint "user_group_settings_user_id_fkey";

set check_function_bodies = off;



