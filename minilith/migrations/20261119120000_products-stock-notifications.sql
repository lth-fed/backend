
create type "public"."notification_event_kind" as enum ('sent', 'opened', 'dismissed');

create type "public"."product_kind" as enum ('ticket', 'normal', 'questionnaire', 'activity_visibility');

create type "public"."stock_record_kind" as enum ('purchase', 'refund', 'take-out', 'put-back', 'balance');

drop view if exists "public"."notification_recipients";

alter table "public"."activities" drop constraint "activities_creator_id_fkey";

alter table "public"."activity_buyers_notifications" drop constraint "activity_buyers_notifications_activity_id_fkey";

alter table "public"."activity_buyers_notifications" drop constraint "activity_buyers_notifications_notification_id_fkey";

alter table "public"."activity_notification_overrides" drop constraint "activity_notification_overrides_activity_id_fkey";

alter table "public"."activity_notification_overrides" drop constraint "activity_notification_overrides_user_id_fkey";

alter table "public"."activity_notifications" drop constraint "activity_notifications_activity_id_fkey";

alter table "public"."activity_notifications" drop constraint "activity_notifications_notification_id_fkey";

alter table "public"."group_notifications" drop constraint "group_notifications_group_id_fkey";

alter table "public"."group_notifications" drop constraint "group_notifications_notification_id_fkey";

alter table "public"."purchased_ticket_addons" drop constraint "purchased_ticket_addons_addon_id_fkey";

alter table "public"."purchased_ticket_addons" drop constraint "purchased_ticket_addons_ticket_id_fkey";

alter table "public"."purchased_ticket_notifications" drop constraint "purchased_ticket_notifications_activity_id_fkey";

alter table "public"."purchased_ticket_notifications" drop constraint "purchased_ticket_notifications_notification_id_fkey";

alter table "public"."purchased_ticket_validations" drop constraint "purchased_ticket_validations_purchased_ticket_id_fkey";

alter table "public"."purchased_tickets" drop constraint "purchased_tickets_owner_id_fkey";

alter table "public"."purchased_tickets" drop constraint "purchased_tickets_purchaser_id_fkey";

alter table "public"."purchased_tickets" drop constraint "purchased_tickets_ticket_kind_id_fkey";

alter table "public"."ticket_addon_options" drop constraint "ticket_addon_options_ticket_addon_id_fkey";

alter table "public"."ticket_addons" drop constraint "ticket_addons_activity_id_fkey";

alter table "public"."ticket_kind_allowed_groups" drop constraint "ticket_kind_allowed_groups_group_id_fkey";

alter table "public"."ticket_kind_allowed_groups" drop constraint "ticket_kind_allowed_groups_ticket_kind_id_fkey";

alter table "public"."ticket_kinds" drop constraint "ticket_kinds_activity_id_fkey";

alter table "public"."ticket_release_queuers" drop constraint "ticket_release_queuers_ticket_kind_id_fkey";

alter table "public"."ticket_release_queuers" drop constraint "ticket_release_queuers_user_id_fkey";

alter table "public"."ticket_release_queuers" drop constraint "ticket_release_queuers_user_id_ticket_kind_id_fkey";

alter table "public"."ticket_reservation_addons" drop constraint "ticket_reservation_addons_addon_id_fkey";

alter table "public"."ticket_reservation_addons" drop constraint "ticket_reservation_addons_ticket_id_fkey";

alter table "public"."ticket_reservation_placement_tails" drop constraint "ticket_reservation_placement_tails_ticket_kind_id_fkey";

alter table "public"."ticket_reservation_queuers" drop constraint "ticket_reservation_queuers_ticket_kind_id_fkey";

alter table "public"."ticket_reservation_queuers" drop constraint "ticket_reservation_queuers_user_id_fkey";

alter table "public"."ticket_reservation_queuers" drop constraint "ticket_reservation_queuers_user_id_ticket_kind_id_fkey";

alter table "public"."ticket_reservations" drop constraint "ticket_reservations_ticket_kind_id_fkey";

alter table "public"."ticket_reservations" drop constraint "ticket_reservations_user_id_fkey";

alter table "public"."ticket_reservations" drop constraint "ticket_reservations_user_id_ticket_kind_id_fkey";

alter table "public"."user_group_settings" drop constraint "user_group_settings_group_id_fkey";

alter table "public"."user_group_settings" drop constraint "user_group_settings_user_id_fkey";

alter table "public"."users_in_purchase_flow" drop constraint "users_in_purchase_flow_ticket_kind_id_fkey";

alter table "public"."ticket_kind_addons" drop constraint "ticket_kind_addons_addon_id_fkey";

alter table "public"."ticket_kind_addons" drop constraint "ticket_kind_addons_ticket_kind_id_fkey";

alter table "public"."ticket_kind_transfer_groups" drop constraint "ticket_kind_transfer_groups_ticket_kind_id_fkey";

alter table "public"."activities" drop constraint "activities_image_id_fkey";

alter table "public"."activity_host_invites" drop constraint "activity_host_invites_activity_id_fkey";

alter table "public"."activity_host_invites" drop constraint "activity_host_invites_group_id_fkey";

alter table "public"."activity_hosts" drop constraint "activity_hosts_activity_id_fkey";

alter table "public"."activity_hosts" drop constraint "activity_hosts_group_id_fkey";

alter table "public"."activity_verifiers" drop constraint "activity_verifiers_activity_id_fkey";

alter table "public"."allow_admins_from_group_view_activities" drop constraint "allow_admins_from_group_view_activities_access_group_id_fkey";

alter table "public"."allow_admins_from_group_view_activities" drop constraint "allow_admins_from_group_view_activities_host_group_id_fkey";

alter table "public"."group_adminships" drop constraint "group_adminships_group_id_fkey";

alter table "public"."group_adminships" drop constraint "group_adminships_group_membership_fk";

alter table "public"."group_member_requests" drop constraint "group_member_requests_group_id_fkey";

alter table "public"."group_memberships" drop constraint "group_memberships_group_id_fkey";

alter table "public"."groups" drop constraint "groups_logo_id_fkey";

alter table "public"."groups" drop constraint "groups_parent_path_fkey";

alter table "public"."groups_ask_to_join" drop constraint "groups_ask_to_join_joiner_id_fkey";

alter table "public"."groups_ask_to_join" drop constraint "groups_ask_to_join_target_id_fkey";

alter table "public"."ticket_kind_transfer_groups" drop constraint "ticket_kind_transfer_groups_group_id_fkey";

alter table "public"."users_in_purchase_flow" drop constraint "users_in_purchase_flow_user_id_fkey";

drop trigger if exists "release_queuer_has_matching_flow" on "public"."ticket_release_queuers";

drop trigger if exists "reservation_queuer_has_matching_flow" on "public"."ticket_reservation_queuers";

drop trigger if exists "reservation_has_matching_flow" on "public"."ticket_reservations";

alter table "public"."activity_buyers_notifications" drop constraint "activity_buyers_notifications_pkey";

alter table "public"."activity_notification_overrides" drop constraint "activity_notification_overrides_pkey";

alter table "public"."activity_notifications" drop constraint "activity_notifications_pkey";

alter table "public"."group_notifications" drop constraint "group_notifications_pkey";

alter table "public"."notifications" drop constraint "notifications_pkey";

alter table "public"."purchased_ticket_addons" drop constraint "purchased_ticket_addons_pkey";

alter table "public"."purchased_ticket_notifications" drop constraint "purchased_ticket_notifications_pkey";

alter table "public"."purchased_ticket_validations" drop constraint "purchased_ticket_validations_pkey";

alter table "public"."purchased_tickets" drop constraint "purchased_tickets_pkey";

alter table "public"."ticket_addon_options" drop constraint "ticket_addon_options_pkey";

alter table "public"."ticket_addons" drop constraint "ticket_addons_pkey";

alter table "public"."ticket_kind_allowed_groups" drop constraint "ticket_kind_allowed_groups_pkey";

alter table "public"."ticket_kinds" drop constraint "ticket_kinds_pkey";

alter table "public"."ticket_release_queuers" drop constraint "ticket_release_queuers_pkey";

alter table "public"."ticket_reservation_addons" drop constraint "ticket_reservation_addons_pkey";

alter table "public"."ticket_reservation_placement_tails" drop constraint "ticket_reservation_placement_tails_pkey";

alter table "public"."ticket_reservation_queuers" drop constraint "ticket_reservation_queuers_pkey";

alter table "public"."ticket_reservations" drop constraint "ticket_reservations_pkey";

alter table "public"."user_group_settings" drop constraint "user_group_settings_pkey";

alter table "public"."activities" drop constraint "activities_max_tickets_check";

alter table "public"."ticket_addon_options" drop constraint "bookkeeping_lengths_consistent";

alter table "public"."ticket_addon_options" drop constraint "bookkeeping_prices_add_up";

alter table "public"."ticket_addon_options" drop constraint "ticket_addon_options_price_check";

alter table "public"."ticket_kinds" drop constraint "ticket_kinds_check";

alter table "public"."ticket_kinds" drop constraint "ticket_kinds_check1";

alter table "public"."ticket_kinds" drop constraint "ticket_kinds_check2";

alter table "public"."ticket_kinds" drop constraint "ticket_kinds_max_tickets_check";

alter table "public"."ticket_kinds" drop constraint "ticket_kinds_min_tickets_check";

alter table "public"."ticket_kinds" drop constraint "ticket_kinds_price_check";

alter table "public"."ticket_reservation_queuers" drop constraint "ticket_reservation_queuers_placement_check";

alter table "public"."ticket_reservation_queuers" drop constraint "ticket_reservation_queuers_ticket_kind_id_placement_key";

alter table "public"."ticket_reservations" drop constraint "ticket_reservations_transaction_id_key";

alter table "public"."ticket_reservations" drop constraint "ticket_reservations_user_id_key";

alter table "public"."users_in_purchase_flow" drop constraint "users_in_purchase_flow_user_id_ticket_kind_id_key";

drop index if exists "public"."activity_buyers_notifications_by_activity";

drop index if exists "public"."activity_buyers_notifications_by_notification";

drop index if exists "public"."activity_buyers_notifications_pkey";

drop index if exists "public"."activity_notification_overrides_pkey";

drop index if exists "public"."activity_notifications_by_activity";

drop index if exists "public"."activity_notifications_by_notification";

drop index if exists "public"."activity_notifications_pkey";

drop index if exists "public"."group_notifications_by_group";

drop index if exists "public"."group_notifications_by_notification";

drop index if exists "public"."group_notifications_pkey";

drop index if exists "public"."group_tree";

drop index if exists "public"."group_tree_eq";

drop index if exists "public"."notifications_pkey";

drop index if exists "public"."notifications_unsent_send_time";

drop index if exists "public"."purchased_ticket_addons_pkey";

drop index if exists "public"."purchased_ticket_notifications_by_activity";

drop index if exists "public"."purchased_ticket_notifications_by_notification";

drop index if exists "public"."purchased_ticket_notifications_pkey";

drop index if exists "public"."purchased_ticket_validations_pkey";

drop index if exists "public"."purchased_tickets_by_owner_and_ticket_kind";

drop index if exists "public"."purchased_tickets_by_ticket_kind";

drop index if exists "public"."purchased_tickets_pkey";

drop index if exists "public"."ticket_addon_options_pkey";

drop index if exists "public"."ticket_addons_pkey";

drop index if exists "public"."ticket_kind_allowed_groups_by_group";

drop index if exists "public"."ticket_kind_allowed_groups_by_ticket_kind";

drop index if exists "public"."ticket_kind_allowed_groups_pkey";

drop index if exists "public"."ticket_kind_by_activity";

drop index if exists "public"."ticket_kind_start";

drop index if exists "public"."ticket_kinds_pkey";

drop index if exists "public"."ticket_purchased_transaction";

drop index if exists "public"."ticket_queuer_placement";

drop index if exists "public"."ticket_queuer_timeout";

drop index if exists "public"."ticket_release_queuers_pkey";

drop index if exists "public"."ticket_release_queuers_ticket_id";

drop index if exists "public"."ticket_reservation_addons_pkey";

drop index if exists "public"."ticket_reservation_placement_tails_pkey";

drop index if exists "public"."ticket_reservation_queuers_pkey";

drop index if exists "public"."ticket_reservation_queuers_ticket_kind_id_placement_key";

drop index if exists "public"."ticket_reservation_timeout";

drop index if exists "public"."ticket_reservations_pkey";

drop index if exists "public"."ticket_reservations_transaction_id_key";

drop index if exists "public"."ticket_reservations_user_id_key";

drop index if exists "public"."ticket_reserved_transaction";

drop index if exists "public"."user_group_settings_by_group";

drop index if exists "public"."user_group_settings_by_user";

drop index if exists "public"."user_group_settings_pkey";

drop index if exists "public"."users_in_purchase_flow_user_id_ticket_kind_id_key";

set check_function_bodies = off;

CREATE OR REPLACE FUNCTION public.ensure_purchase_flow_has_state()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
declare
    checked_user text;
begin
    if tg_op = 'DELETE' then
        checked_user := old.user_id;
    else
        checked_user := new.user_id;
    end if;
    if exists (
        select 1 from users_in_purchase_flow flow
        where flow.user_id = checked_user
        and (
            num_nonnulls(release_queue, reservation_queue, reservation) != 1
            or (release_queue is not null) != exists (
                select 1 from product_release_queuers
                where user_id = flow.user_id and product_id = flow.product_id
            )
            or (reservation_queue is not null) != exists (
                select 1 from product_reservation_queuers
                where user_id = flow.user_id and product_id = flow.product_id
            )
            or (reservation is not null) != exists (
                select 1 from product_reservations
                where user_id = flow.user_id and product_id = flow.product_id
            )
        )
    ) then
        raise exception 'purchase flow for user % has inconsistent state', checked_user;
    end if;
    return null;
end;
$function$
;




















create table "public"."activity_assigned_tags" (
    "activity_id" uuid not null,
    "tag_id" uuid not null
);


create table "public"."activity_followers" (
    "user_id" text not null,
    "activity_id" uuid not null,
    "follow" boolean not null
);


create table "public"."activity_opt_out_tags" (
    "user_id" text not null,
    "tag_id" uuid not null
);


create table "public"."activity_push_notifications" (
    "id" uuid not null,
    "activity_id" uuid not null,
    "notification_id" uuid not null
);


create table "public"."activity_push_random_selected" (
    "activity_id" uuid not null,
    "user_id" text not null
);


create table "public"."activity_tags" (
    "id" uuid not null,
    "creator_id" uuid not null,
    "title" jsonb not null
);


create table "public"."activity_views" (
    "id" uuid not null,
    "activity_id" uuid not null,
    "user_id" text not null,
    "count" integer not null
);


create table "public"."news" (
    "id" uuid not null,
    "target_group_id" uuid not null,
    "title" jsonb not null,
    "content" jsonb not null,
    "questionnaire_product" uuid,
    "published_at" timestamp with time zone not null
);


create table "public"."news_assigned_tags" (
    "news_id" uuid not null,
    "tag_id" uuid not null
);


create table "public"."news_push_notifications" (
    "id" uuid not null,
    "news_id" uuid not null,
    "notification_id" uuid not null
);


create table "public"."news_push_opt_out_tags" (
    "user_id" text not null,
    "tag_id" uuid not null
);


create table "public"."news_tags" (
    "id" uuid not null,
    "creator_id" uuid not null,
    "title" jsonb not null
);


create table "public"."notification_events" (
    "id" uuid not null default uuidv4(),
    "notification_id" uuid not null,
    "user_id" text not null,
    "kind" notification_event_kind not null,
    "target_id" uuid,
    "time" timestamp with time zone not null default now()
);


create table "public"."notification_stats" (
    "notification_id" uuid not null,
    "n_opened" integer not null,
    "n_dismissed" integer not null,
    "n_sent" integer not null
);


create table "public"."physical_payment_terminals" (
    "id" text not null,
    "group_id" uuid not null,
    "name" text not null
);


create table "public"."physical_sales_clients" (
    "id" uuid not null,
    "name" text not null,
    "group_id" uuid,
    "activity_id" uuid,
    "until" timestamp with time zone not null
);


create table "public"."product_activity_assignments" (
    "activity_id" uuid not null,
    "product_id" uuid not null
);


create table "public"."product_addon_option_bookkeepings" (
    "id" uuid not null,
    "option_id" uuid not null,
    "price" money not null,
    "category" uuid not null
);


create table "public"."product_addon_options" (
    "id" uuid not null,
    "product_addon_id" uuid not null,
    "idx" integer not null,
    "name" jsonb not null,
    "price" money not null
);


create table "public"."product_addons" (
    "id" uuid not null,
    "activity_id" uuid not null,
    "idx" integer not null,
    "name" jsonb not null,
    "multiple_alternatives" boolean not null,
    "has_text_field" boolean not null,
    "required" boolean not null
);


create table "public"."product_allowed_groups" (
    "product_id" uuid not null,
    "group_id" uuid not null
);


create table "public"."product_assigned_tags" (
    "tag_id" uuid not null,
    "product_id" uuid not null
);


create table "public"."product_buyers" (
    "product_id" uuid not null,
    "activity_id" uuid not null,
    "user_id" text not null,
    "guaranteed" boolean not null
);


create table "public"."product_release_queuers" (
    "user_id" text not null,
    "product_id" uuid not null,
    "started_queueing" timestamp with time zone not null
);


create table "public"."product_reservation_addons" (
    "addon_id" uuid not null,
    "reservation_id" uuid not null,
    "selected_options" integer[] not null,
    "selected_options_prices" money[] not null,
    "selected_text" text not null
);


create table "public"."product_reservation_placement_tails" (
    "product_id" uuid not null,
    "placement_tail" integer not null
);


create table "public"."product_reservation_queuers" (
    "user_id" text not null,
    "product_id" uuid not null,
    "placement" integer not null
);


create table "public"."product_reservations" (
    "id" uuid not null default uuidv4(),
    "user_id" text not null,
    "product_id" uuid not null,
    "product_price" money not null,
    "transaction_id" uuid,
    "timeout" timestamp with time zone not null
);


create table "public"."product_tags" (
    "id" uuid not null,
    "group_id" uuid not null,
    "hidden" boolean not null default false,
    "name" jsonb not null
);


create table "public"."products" (
    "id" uuid not null default uuidv4(),
    "group_id" uuid not null,
    "image_id" uuid,
    "kind" product_kind not null,
    "name" jsonb not null,
    "description" jsonb not null,
    "sales_price" money not null,
    "sales_organization" uuid not null,
    "sales_stock" integer,
    "stock_real" integer not null,
    "stock_unit" text not null,
    "stock_price" numeric(18,6) not null,
    "bookkeeping_tag" uuid not null,
    "max_uses" integer not null default 1,
    "sales_start" timestamp with time zone not null,
    "sales_stop" timestamp with time zone not null,
    "resale_start" timestamp with time zone not null,
    "resale_stop" timestamp with time zone not null,
    "been_purchased" boolean not null default false,
    "store_general_sales" boolean not null default false,
    "store_online_self_serve" boolean not null default false,
    "store_internal_id" text,
    "store_alcohol_by_volume" integer,
    "tickets_been_released" boolean not null default false,
    "tickets_guaranteed" boolean not null default false,
    "tickets_require_other" boolean not null default false
);


create table "public"."purchase_transactions" (
    "purchase_id" uuid not null,
    "seller_id" text,
    "buyer_id" text not null,
    "completed_at" timestamp with time zone not null,
    "transaction_id" uuid
);


create table "public"."purchase_use_validations" (
    "id" uuid not null,
    "use_id" uuid not null,
    "timestamp" timestamp with time zone not null default now()
);


create table "public"."purchase_uses" (
    "id" uuid not null,
    "purchase_id" uuid not null,
    "timestamp" timestamp with time zone not null default now()
);


create table "public"."purchased_product_addons" (
    "addon_id" uuid not null,
    "purchase_id" uuid not null,
    "selected_options" integer[] not null,
    "selected_options_prices" money[] not null,
    "selected_text" text not null
);


create table "public"."purchased_ticket_push_notifications" (
    "id" uuid not null,
    "activity_id" uuid not null,
    "notification_id" uuid not null
);


create table "public"."purchases" (
    "id" uuid not null default uuidv4(),
    "product_id" uuid not null,
    "product_price" money not null,
    "activity_id" uuid,
    "purchaser_id" text not null,
    "owner_id" text not null,
    "purchased_at" timestamp with time zone not null,
    "refunded_id" uuid,
    "for_resale" money,
    "no_show_hint" boolean not null default false,
    "seller_payout_completed" boolean not null default false,
    "seller_payout_id" uuid
);


create table "public"."push_notifications" (
    "id" uuid not null,
    "sender" jsonb not null,
    "title" jsonb not null,
    "content" jsonb not null,
    "send_at" timestamp with time zone not null,
    "sent" boolean not null default false
);


create table "public"."sales_targets" (
    "id" uuid not null,
    "group_id" uuid not null,
    "email" text not null,
    "iban" text not null
);


create table "public"."stock_transaction_records" (
    "id" uuid not null,
    "transaction_id" uuid not null,
    "product_id" uuid not null,
    "amount" numeric not null,
    "kind" stock_record_kind not null
);


create table "public"."stock_transactions" (
    "id" uuid not null,
    "name" jsonb not null,
    "group_id" uuid not null,
    "activity_id" uuid,
    "time" timestamp with time zone not null default now()
);


create table "public"."ticket_release_groups" (
    "push_id" uuid not null,
    "push_ticket_id" uuid not null,
    "to_group_id" uuid not null
);


create table "public"."ticket_release_push_notifications" (
    "id" uuid not null,
    "ticket_id" uuid not null,
    "notification_id" uuid not null
);


create table "public"."vote_alternatives" (
    "id" uuid not null,
    "round_id" uuid not null,
    "title" jsonb not null,
    "voted_for" integer not null
);


create table "public"."vote_cast_votes" (
    "client_id" uuid not null,
    "round_id" uuid not null
);


create table "public"."vote_meeting_admins" (
    "user_id" text not null,
    "meeting_id" uuid not null,
    "readonly" boolean not null
);


create table "public"."vote_meetings" (
    "id" uuid not null,
    "group_id" uuid not null,
    "activity_id" uuid,
    "secure_voting" boolean not null,
    "title" jsonb not null
);


create table "public"."vote_registered_clients" (
    "client_id" uuid not null,
    "meeting_id" uuid not null
);


create table "public"."vote_registered_users" (
    "user_id" text not null,
    "meeting_id" uuid not null
);


create table "public"."vote_rounds" (
    "id" uuid not null,
    "meeting_id" uuid not null,
    "title" jsonb not null,
    "secret" text not null,
    "max_choises" integer not null,
    "open" boolean not null
);


alter table "public"."activities" add column "base_ticket" uuid;

alter table "public"."activities" add column "max_tickets_per_user" integer not null default 2147483647;

alter table "public"."activities" add column "sales_target_id" uuid;

alter table "public"."activities" add column "stock_group" uuid;

-- ================== data migration from the ticket model ==================

-- purchase queues are ephemeral
delete from ticket_release_queuers;
delete from ticket_reservation_queuers;
delete from ticket_reservations;
delete from users_in_purchase_flow;

-- everything pre-existing is paid out through E-sektionen
insert into sales_targets (id, group_id, email, iban)
select uuidv4(), groups.id, 'informationschef@esek.se', 'SE48 8000 0831 3907 4120 7096'
from groups
where groups.path = 'tlth.e';

-- bookkeeping tags live on the guild (second level group, e.g. tlth.e)
create temporary table migrated_guilds as
select guilds.id as group_id, guilds.path, uuidv4() as tag_id
from groups guilds
where guilds.path in (
    select subpath(creator.path, 0, 2)
    from activities
    inner join groups creator on creator.id = activities.creator_id
);

-- a default bookkeeping tag per guild that owned activities
insert into product_tags (id, group_id, hidden, name)
select tag_id, group_id, false, '{"en": "Ticket", "sv": "Biljett"}'::jsonb
from migrated_guilds;

-- we don't care about old tags
-- -- a bookkeeping tag per old free-text addon category
-- insert into product_tags (id, group_id, hidden, name)
-- select uuidv4(), categories.group_id, false, jsonb_build_object('en', categories.category)
-- from (
--     select distinct activities.creator_id as group_id, category
--     from ticket_addon_options
--     inner join ticket_addons on ticket_addons.id = ticket_addon_options.ticket_addon_id
--     inner join activities on activities.id = ticket_addons.activity_id
--     cross join lateral unnest(ticket_addon_options.bookkeeping_price_categories) as category
-- ) categories;

-- ticket kinds become ticket products
insert into products (
    id, group_id, image_id, kind, name, description, sales_price, sales_organization,
    sales_stock, stock_real, stock_unit, stock_price, bookkeeping_tag, max_uses,
    sales_start, sales_stop, resale_start, resale_stop, been_purchased,
    store_general_sales, store_online_self_serve, store_internal_id, store_alcohol_by_volume,
    tickets_been_released, tickets_guaranteed, tickets_require_other
)
select
    ticket_kinds.id, activities.creator_id, null, 'ticket', ticket_kinds.name, '{}'::jsonb,
    ticket_kinds.price, (select id from sales_targets limit 1),
    ticket_kinds.max_tickets - ticket_kinds.reserved_or_purchased_tickets,
    ticket_kinds.max_tickets - ticket_kinds.reserved_or_purchased_tickets, 'st', 0,
    (select migrated_guilds.tag_id from migrated_guilds
      where migrated_guilds.path = subpath(creator.path, 0, 2)),
    1, ticket_kinds.purchasing_available_start, ticket_kinds.purchasing_available_stop,
    ticket_kinds.allow_transfer_ticket_start, ticket_kinds.allow_transfer_ticket_stop,
    ticket_kinds.has_been_purchased, false, false, null, null,
    ticket_kinds.has_been_released, false, false
from ticket_kinds
inner join activities on activities.id = ticket_kinds.activity_id
inner join groups creator on creator.id = activities.creator_id;

-- addons
insert into product_addons (id, activity_id, idx, name, multiple_alternatives, has_text_field, required)
select id, activity_id, idx, name, multiple_alternatives, has_text_field, required from ticket_addons;

insert into product_addon_options (id, product_addon_id, idx, name, price)
select id, ticket_addon_id, idx, name, price from ticket_addon_options;

-- old addon bookkeeping categories are discarded

-- activity links & buying permissions
insert into product_activity_assignments (activity_id, product_id)
select activity_id, id from ticket_kinds;

insert into product_allowed_groups (product_id, group_id)
select ticket_kind_id, group_id from ticket_kind_allowed_groups;

-- activities that never had a ticket kind get a synthetic free one
create temporary table migrated_synthetic_tickets as
select uuidv4() as id, activities.id as activity_id, activities.creator_id as group_id,
       activities.title as name, activities.description, activities.time_start, activities.time_end
from activities
where not exists (select 1 from ticket_kinds where ticket_kinds.activity_id = activities.id);

insert into products (
    id, group_id, image_id, kind, name, description, sales_price, sales_organization,
    sales_stock, stock_real, stock_unit, stock_price, bookkeeping_tag, max_uses,
    sales_start, sales_stop, resale_start, resale_stop, been_purchased,
    store_general_sales, store_online_self_serve, store_internal_id, store_alcohol_by_volume,
    tickets_been_released, tickets_guaranteed, tickets_require_other
)
select
    synthetic.id, synthetic.group_id, null, 'ticket', synthetic.name, synthetic.description,
    0::money, (select id from sales_targets limit 1), 0, 0, 'st', 0,
    (select migrated_guilds.tag_id from migrated_guilds
      where migrated_guilds.path = subpath(creator.path, 0, 2)),
    1, synthetic.time_start, synthetic.time_end, synthetic.time_end, synthetic.time_end, false,
    false, false, null, null, false, false, false
from migrated_synthetic_tickets synthetic
inner join groups creator on creator.id = synthetic.group_id;

insert into product_activity_assignments (activity_id, product_id)
select activity_id, id from migrated_synthetic_tickets;

-- every activity needs a base ticket: the visibility ticket, else the cheapest, else the synthetic one
update activities
set base_ticket = coalesce(
    (select ticket_kinds.id from ticket_kinds
      where ticket_kinds.activity_id = activities.id and ticket_kinds.for_visibility
      limit 1),
    (select ticket_kinds.id from ticket_kinds
      where ticket_kinds.activity_id = activities.id
      order by ticket_kinds.price
      limit 1),
    (select migrated_synthetic_tickets.id from migrated_synthetic_tickets
      where migrated_synthetic_tickets.activity_id = activities.id)
);

update activities set sales_target_id = (select id from sales_targets limit 1);

alter table "public"."activities" alter column "base_ticket" set not null;

-- bought tickets become purchases. The old model had no purchase timestamp, so now() is used.
insert into purchases (
    id, product_id, product_price, activity_id, purchaser_id, owner_id, purchased_at
)
select
    purchased_tickets.id, purchased_tickets.ticket_kind_id, ticket_kinds.price,
    ticket_kinds.activity_id, purchased_tickets.purchaser_id, purchased_tickets.owner_id, now()
from purchased_tickets
inner join ticket_kinds on ticket_kinds.id = purchased_tickets.ticket_kind_id;

insert into purchase_transactions (purchase_id, seller_id, buyer_id, completed_at, transaction_id)
select purchased_tickets.id, null, purchased_tickets.purchaser_id, now(), purchased_tickets.transaction_id
from purchased_tickets;

-- Old QR codes encode the purchased ticket id, so each migrated purchase gets a
-- use row with that same id and stays scannable. Old validations are deliberately
-- not copied: repeat scans of a use are allowed (shown as a warning in the app).
insert into purchase_uses (id, purchase_id, timestamp)
select purchases.id, purchases.id, now()
from purchases;

insert into purchased_product_addons (
    addon_id, purchase_id, selected_options, selected_options_prices, selected_text
)
select
    purchased_ticket_addons.addon_id, purchased_ticket_addons.ticket_id,
    purchased_ticket_addons.selected_options,
    coalesce((
        select array_agg(coalesce(options.price, 0::money) order by selected.ordinality)
        from unnest(purchased_ticket_addons.selected_options) with ordinality as selected(idx, ordinality)
        left join product_addon_options options
            on options.product_addon_id = purchased_ticket_addons.addon_id
            and options.idx = selected.idx
    ), '{}'::money[]),
    purchased_ticket_addons.selected_text
from purchased_ticket_addons;

-- notification history
insert into push_notifications (id, sender, title, content, send_at, sent)
select id, sender, title, content, send_at, sent from notifications;

insert into activity_push_notifications (id, activity_id, notification_id)
select id, activity_id, notification_id from activity_notifications;

insert into purchased_ticket_push_notifications (id, activity_id, notification_id)
select id, activity_id, notification_id from purchased_ticket_notifications
union
select id, activity_id, notification_id from activity_buyers_notifications;

insert into news (id, target_group_id, title, content, questionnaire_product, published_at)
select
    group_notifications.id, group_notifications.group_id,
    notifications.title, notifications.content, null, notifications.send_at
from group_notifications
inner join notifications on notifications.id = group_notifications.notification_id;

insert into news_push_notifications (id, news_id, notification_id)
select group_notifications.id, group_notifications.id, group_notifications.notification_id
from group_notifications;

insert into activity_followers (user_id, activity_id, follow)
select user_id, activity_id, follow from activity_notification_overrides;

alter table "public"."activities" alter column "max_tickets_per_user" drop default;

alter table "public"."activities" alter column "sales_target_id" set not null;

alter table "public"."activities" drop column "creator_id";

alter table "public"."activities" drop column "max_tickets";

alter table "public"."group_adminships" add column "until" timestamp with time zone;

alter table "public"."group_memberships" add column "specific" boolean not null default false;

alter table "public"."group_memberships" add column "until" timestamp with time zone;

alter table "public"."groups" drop column "limit_membership_visibility";

alter table "public"."groups" add column "parent_members_can_join" boolean not null default false;

alter table "public"."users_in_purchase_flow" drop column "ticket_kind_id";

alter table "public"."users_in_purchase_flow" add column "product_id" uuid not null;

create or replace view "public"."notification_recipients" as  SELECT release_notification.notification_id,
    group_memberships.user_id
   FROM ticket_release_push_notifications release_notification
     JOIN product_allowed_groups ON product_allowed_groups.product_id = release_notification.ticket_id
     JOIN groups allowed_group ON allowed_group.id = product_allowed_groups.group_id
     JOIN groups member_group ON allowed_group.path @> member_group.path
     JOIN group_memberships ON group_memberships.group_id = member_group.id
  WHERE member_group.id = allowed_group.id OR NOT group_memberships.specific
UNION
 SELECT purchased_notification.notification_id,
    purchases.owner_id AS user_id
   FROM purchased_ticket_push_notifications purchased_notification
     JOIN purchases ON purchases.activity_id = purchased_notification.activity_id
UNION
 SELECT activity_notification.notification_id,
    activity_followers.user_id
   FROM activity_push_notifications activity_notification
     JOIN activity_followers ON activity_followers.activity_id = activity_notification.activity_id
  WHERE activity_followers.follow
UNION
 SELECT activity_notification.notification_id,
    product_buyers.user_id
   FROM activity_push_notifications activity_notification
     JOIN product_buyers ON product_buyers.activity_id = activity_notification.activity_id
UNION
 SELECT activity_notification.notification_id,
    group_memberships.user_id
   FROM activity_push_notifications activity_notification
     JOIN activity_hosts ON activity_hosts.activity_id = activity_notification.activity_id
     JOIN groups host_group ON host_group.id = activity_hosts.group_id
     JOIN groups member_group ON host_group.path @> member_group.path
     JOIN group_memberships ON group_memberships.group_id = member_group.id
  WHERE member_group.id = host_group.id OR NOT group_memberships.specific
UNION
 SELECT activity_notification.notification_id,
    activity_push_random_selected.user_id
   FROM activity_push_notifications activity_notification
     JOIN activity_push_random_selected ON activity_push_random_selected.activity_id = activity_notification.activity_id
UNION
 SELECT news_notification.notification_id,
    group_memberships.user_id
   FROM news_push_notifications news_notification
     JOIN news ON news.id = news_notification.news_id
     JOIN groups target_group ON target_group.id = news.target_group_id
     JOIN groups member_group ON target_group.path @> member_group.path
     JOIN group_memberships ON group_memberships.group_id = member_group.id
  WHERE member_group.id = target_group.id OR NOT group_memberships.specific;


CREATE CONSTRAINT TRIGGER release_queuer_has_matching_flow AFTER INSERT OR DELETE OR UPDATE ON public.product_release_queuers DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION ensure_purchase_flow_has_state();

CREATE CONSTRAINT TRIGGER reservation_queuer_has_matching_flow AFTER INSERT OR DELETE OR UPDATE ON public.product_reservation_queuers DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION ensure_purchase_flow_has_state();

CREATE CONSTRAINT TRIGGER reservation_has_matching_flow AFTER INSERT OR DELETE OR UPDATE ON public.product_reservations DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION ensure_purchase_flow_has_state();

CREATE UNIQUE INDEX activity_assigned_tags_pkey ON public.activity_assigned_tags USING btree (activity_id, tag_id);

CREATE UNIQUE INDEX activity_followers_pkey ON public.activity_followers USING btree (user_id, activity_id);

CREATE UNIQUE INDEX activity_opt_out_tags_pkey ON public.activity_opt_out_tags USING btree (user_id, tag_id);

CREATE INDEX activity_push_notifications_by_activity ON public.activity_push_notifications USING btree (activity_id);

CREATE UNIQUE INDEX activity_push_notifications_pkey ON public.activity_push_notifications USING btree (id, activity_id);

CREATE UNIQUE INDEX activity_push_random_selected_pkey ON public.activity_push_random_selected USING btree (activity_id, user_id);

CREATE UNIQUE INDEX activity_tags_pkey ON public.activity_tags USING btree (id);

CREATE INDEX activity_views_by_activity ON public.activity_views USING hash (activity_id);

CREATE UNIQUE INDEX activity_views_pkey ON public.activity_views USING btree (id);

CREATE UNIQUE INDEX activity_views_user_id_key ON public.activity_views USING btree (user_id);

CREATE INDEX group_memberships_expiry ON public.group_memberships USING btree (until) WHERE (until IS NOT NULL);

CREATE INDEX groups_parent_path ON public.groups USING btree (parent_path);

CREATE UNIQUE INDEX news_assigned_tags_pkey ON public.news_assigned_tags USING btree (news_id, tag_id);

CREATE UNIQUE INDEX news_pkey ON public.news USING btree (id);

CREATE INDEX news_push_notifications_by_news ON public.news_push_notifications USING btree (news_id);

CREATE UNIQUE INDEX news_push_notifications_pkey ON public.news_push_notifications USING btree (id);

CREATE UNIQUE INDEX news_push_opt_out_tags_pkey ON public.news_push_opt_out_tags USING btree (user_id, tag_id);

CREATE UNIQUE INDEX news_tags_pkey ON public.news_tags USING btree (id);

CREATE INDEX notification_events_by_notification ON public.notification_events USING btree (notification_id);

CREATE UNIQUE INDEX notification_events_pkey ON public.notification_events USING btree (id);

CREATE UNIQUE INDEX notification_stats_pkey ON public.notification_stats USING btree (notification_id);

CREATE UNIQUE INDEX physical_payment_terminals_pkey ON public.physical_payment_terminals USING btree (id);

CREATE UNIQUE INDEX physical_sales_clients_pkey ON public.physical_sales_clients USING btree (id);

CREATE INDEX product_activity_assignments_by_activity ON public.product_activity_assignments USING hash (activity_id);

CREATE UNIQUE INDEX product_activity_assignments_pkey ON public.product_activity_assignments USING btree (activity_id, product_id);

CREATE UNIQUE INDEX product_addon_option_bookkeepings_pkey ON public.product_addon_option_bookkeepings USING btree (id);

CREATE UNIQUE INDEX product_addon_options_pkey ON public.product_addon_options USING btree (id);

CREATE UNIQUE INDEX product_addons_pkey ON public.product_addons USING btree (id);

CREATE INDEX product_allowed_groups_by_group ON public.product_allowed_groups USING hash (group_id);

CREATE UNIQUE INDEX product_allowed_groups_pkey ON public.product_allowed_groups USING btree (group_id, product_id);

CREATE UNIQUE INDEX product_assigned_tags_pkey ON public.product_assigned_tags USING btree (tag_id, product_id);

CREATE UNIQUE INDEX product_buyers_pkey ON public.product_buyers USING btree (activity_id, user_id);

CREATE INDEX product_buyers_product ON public.product_buyers USING hash (product_id);

CREATE INDEX product_queuer_placement ON public.product_reservation_queuers USING btree (placement);

CREATE INDEX product_queuer_timeout ON public.product_release_queuers USING btree (started_queueing);

CREATE INDEX product_release_queuers_by_ticket ON public.product_release_queuers USING hash (product_id);

CREATE UNIQUE INDEX product_release_queuers_pkey ON public.product_release_queuers USING btree (user_id);

CREATE UNIQUE INDEX product_reservation_addons_pkey ON public.product_reservation_addons USING btree (reservation_id, addon_id);

CREATE UNIQUE INDEX product_reservation_placement_tails_pkey ON public.product_reservation_placement_tails USING btree (product_id);

CREATE UNIQUE INDEX product_reservation_queuers_pkey ON public.product_reservation_queuers USING btree (user_id);

CREATE UNIQUE INDEX product_reservation_queuers_product_id_placement_key ON public.product_reservation_queuers USING btree (product_id, placement);

CREATE INDEX product_reservation_timeout ON public.product_reservations USING btree (timeout);

CREATE UNIQUE INDEX product_reservations_pkey ON public.product_reservations USING btree (id);

CREATE UNIQUE INDEX product_reservations_transaction_id_key ON public.product_reservations USING btree (transaction_id);

CREATE UNIQUE INDEX product_reservations_user_id_key ON public.product_reservations USING btree (user_id);

CREATE INDEX product_reserved_transaction ON public.product_reservations USING hash (transaction_id);

CREATE UNIQUE INDEX product_tags_pkey ON public.product_tags USING btree (id);

CREATE INDEX products_by_group ON public.products USING hash (group_id);

CREATE UNIQUE INDEX products_pkey ON public.products USING btree (id);

CREATE INDEX products_sales_start ON public.products USING btree (sales_start);

CREATE INDEX purchase_transactions_by_purchase ON public.purchase_transactions USING btree (purchase_id);

CREATE UNIQUE INDEX purchase_use_validations_pkey ON public.purchase_use_validations USING btree (id);

CREATE INDEX purchase_uses_by_purchase ON public.purchase_uses USING btree (purchase_id);

CREATE UNIQUE INDEX purchase_uses_pkey ON public.purchase_uses USING btree (id);

CREATE INDEX purchased_product_addons_by_purchase ON public.purchased_product_addons USING btree (purchase_id);

CREATE UNIQUE INDEX purchased_product_addons_pkey ON public.purchased_product_addons USING btree (purchase_id, addon_id);

CREATE INDEX purchased_ticket_push_notifications_by_activity ON public.purchased_ticket_push_notifications USING btree (activity_id);

CREATE UNIQUE INDEX purchased_ticket_push_notifications_pkey ON public.purchased_ticket_push_notifications USING btree (id, activity_id);

CREATE INDEX purchases_by_activity ON public.purchases USING hash (activity_id);

CREATE INDEX purchases_by_owner ON public.purchases USING btree (owner_id);

CREATE INDEX purchases_by_product ON public.purchases USING hash (product_id);

CREATE UNIQUE INDEX purchases_pkey ON public.purchases USING btree (id);

CREATE UNIQUE INDEX push_notifications_pkey ON public.push_notifications USING btree (id);

CREATE INDEX push_notifications_unsent_send_time ON public.push_notifications USING btree (send_at) WHERE (sent = false);

CREATE UNIQUE INDEX sales_targets_pkey ON public.sales_targets USING btree (id);

CREATE INDEX stock_transaction_records_by_product ON public.stock_transaction_records USING hash (product_id);

CREATE UNIQUE INDEX stock_transaction_records_pkey ON public.stock_transaction_records USING btree (id);

CREATE INDEX stock_transactions_by_group ON public.stock_transactions USING hash (group_id);

CREATE UNIQUE INDEX stock_transactions_pkey ON public.stock_transactions USING btree (id);

CREATE UNIQUE INDEX ticket_release_groups_pkey ON public.ticket_release_groups USING btree (push_id, push_ticket_id, to_group_id);

CREATE INDEX ticket_release_push_notifications_by_ticket ON public.ticket_release_push_notifications USING hash (ticket_id);

CREATE UNIQUE INDEX ticket_release_push_notifications_pkey ON public.ticket_release_push_notifications USING btree (id, ticket_id);

CREATE UNIQUE INDEX users_in_purchase_flow_user_id_product_id_key ON public.users_in_purchase_flow USING btree (user_id, product_id);

CREATE INDEX vote_alternatives_by_round ON public.vote_alternatives USING hash (round_id);

CREATE UNIQUE INDEX vote_alternatives_pkey ON public.vote_alternatives USING btree (id);

CREATE UNIQUE INDEX vote_cast_votes_pkey ON public.vote_cast_votes USING btree (client_id, round_id);

CREATE UNIQUE INDEX vote_meeting_admins_pkey ON public.vote_meeting_admins USING btree (user_id, meeting_id);

CREATE UNIQUE INDEX vote_meetings_pkey ON public.vote_meetings USING btree (id);

CREATE UNIQUE INDEX vote_registered_clients_client_id_key ON public.vote_registered_clients USING btree (client_id);

CREATE UNIQUE INDEX vote_registered_clients_pkey ON public.vote_registered_clients USING btree (client_id, meeting_id);

CREATE UNIQUE INDEX vote_registered_users_pkey ON public.vote_registered_users USING btree (user_id, meeting_id);

CREATE INDEX vote_rounds_by_meeting ON public.vote_rounds USING hash (meeting_id);

CREATE UNIQUE INDEX vote_rounds_pkey ON public.vote_rounds USING btree (id);


alter table "public"."activity_assigned_tags" add constraint "activity_assigned_tags_pkey" PRIMARY KEY using index "activity_assigned_tags_pkey";

alter table "public"."activity_followers" add constraint "activity_followers_pkey" PRIMARY KEY using index "activity_followers_pkey";

alter table "public"."activity_opt_out_tags" add constraint "activity_opt_out_tags_pkey" PRIMARY KEY using index "activity_opt_out_tags_pkey";

alter table "public"."activity_push_notifications" add constraint "activity_push_notifications_pkey" PRIMARY KEY using index "activity_push_notifications_pkey";

alter table "public"."activity_push_random_selected" add constraint "activity_push_random_selected_pkey" PRIMARY KEY using index "activity_push_random_selected_pkey";

alter table "public"."activity_tags" add constraint "activity_tags_pkey" PRIMARY KEY using index "activity_tags_pkey";

alter table "public"."activity_views" add constraint "activity_views_pkey" PRIMARY KEY using index "activity_views_pkey";

alter table "public"."news" add constraint "news_pkey" PRIMARY KEY using index "news_pkey";

alter table "public"."news_assigned_tags" add constraint "news_assigned_tags_pkey" PRIMARY KEY using index "news_assigned_tags_pkey";

alter table "public"."news_push_notifications" add constraint "news_push_notifications_pkey" PRIMARY KEY using index "news_push_notifications_pkey";

alter table "public"."news_push_opt_out_tags" add constraint "news_push_opt_out_tags_pkey" PRIMARY KEY using index "news_push_opt_out_tags_pkey";

alter table "public"."news_tags" add constraint "news_tags_pkey" PRIMARY KEY using index "news_tags_pkey";

alter table "public"."notification_events" add constraint "notification_events_pkey" PRIMARY KEY using index "notification_events_pkey";

alter table "public"."notification_stats" add constraint "notification_stats_pkey" PRIMARY KEY using index "notification_stats_pkey";

alter table "public"."physical_payment_terminals" add constraint "physical_payment_terminals_pkey" PRIMARY KEY using index "physical_payment_terminals_pkey";

alter table "public"."physical_sales_clients" add constraint "physical_sales_clients_pkey" PRIMARY KEY using index "physical_sales_clients_pkey";

alter table "public"."product_activity_assignments" add constraint "product_activity_assignments_pkey" PRIMARY KEY using index "product_activity_assignments_pkey";

alter table "public"."product_addon_option_bookkeepings" add constraint "product_addon_option_bookkeepings_pkey" PRIMARY KEY using index "product_addon_option_bookkeepings_pkey";

alter table "public"."product_addon_options" add constraint "product_addon_options_pkey" PRIMARY KEY using index "product_addon_options_pkey";

alter table "public"."product_addons" add constraint "product_addons_pkey" PRIMARY KEY using index "product_addons_pkey";

alter table "public"."product_allowed_groups" add constraint "product_allowed_groups_pkey" PRIMARY KEY using index "product_allowed_groups_pkey";

alter table "public"."product_assigned_tags" add constraint "product_assigned_tags_pkey" PRIMARY KEY using index "product_assigned_tags_pkey";

alter table "public"."product_buyers" add constraint "product_buyers_pkey" PRIMARY KEY using index "product_buyers_pkey";

alter table "public"."product_release_queuers" add constraint "product_release_queuers_pkey" PRIMARY KEY using index "product_release_queuers_pkey";

alter table "public"."product_reservation_addons" add constraint "product_reservation_addons_pkey" PRIMARY KEY using index "product_reservation_addons_pkey";

alter table "public"."product_reservation_placement_tails" add constraint "product_reservation_placement_tails_pkey" PRIMARY KEY using index "product_reservation_placement_tails_pkey";

alter table "public"."product_reservation_queuers" add constraint "product_reservation_queuers_pkey" PRIMARY KEY using index "product_reservation_queuers_pkey";

alter table "public"."product_reservations" add constraint "product_reservations_pkey" PRIMARY KEY using index "product_reservations_pkey";

alter table "public"."product_tags" add constraint "product_tags_pkey" PRIMARY KEY using index "product_tags_pkey";

alter table "public"."products" add constraint "products_pkey" PRIMARY KEY using index "products_pkey";

alter table "public"."purchase_use_validations" add constraint "purchase_use_validations_pkey" PRIMARY KEY using index "purchase_use_validations_pkey";

alter table "public"."purchase_uses" add constraint "purchase_uses_pkey" PRIMARY KEY using index "purchase_uses_pkey";

alter table "public"."purchased_product_addons" add constraint "purchased_product_addons_pkey" PRIMARY KEY using index "purchased_product_addons_pkey";

alter table "public"."purchased_ticket_push_notifications" add constraint "purchased_ticket_push_notifications_pkey" PRIMARY KEY using index "purchased_ticket_push_notifications_pkey";

alter table "public"."purchases" add constraint "purchases_pkey" PRIMARY KEY using index "purchases_pkey";

alter table "public"."push_notifications" add constraint "push_notifications_pkey" PRIMARY KEY using index "push_notifications_pkey";

alter table "public"."sales_targets" add constraint "sales_targets_pkey" PRIMARY KEY using index "sales_targets_pkey";

alter table "public"."stock_transaction_records" add constraint "stock_transaction_records_pkey" PRIMARY KEY using index "stock_transaction_records_pkey";

alter table "public"."stock_transactions" add constraint "stock_transactions_pkey" PRIMARY KEY using index "stock_transactions_pkey";

alter table "public"."ticket_release_groups" add constraint "ticket_release_groups_pkey" PRIMARY KEY using index "ticket_release_groups_pkey";

alter table "public"."ticket_release_push_notifications" add constraint "ticket_release_push_notifications_pkey" PRIMARY KEY using index "ticket_release_push_notifications_pkey";

alter table "public"."vote_alternatives" add constraint "vote_alternatives_pkey" PRIMARY KEY using index "vote_alternatives_pkey";

alter table "public"."vote_cast_votes" add constraint "vote_cast_votes_pkey" PRIMARY KEY using index "vote_cast_votes_pkey";

alter table "public"."vote_meeting_admins" add constraint "vote_meeting_admins_pkey" PRIMARY KEY using index "vote_meeting_admins_pkey";

alter table "public"."vote_meetings" add constraint "vote_meetings_pkey" PRIMARY KEY using index "vote_meetings_pkey";

alter table "public"."vote_registered_clients" add constraint "vote_registered_clients_pkey" PRIMARY KEY using index "vote_registered_clients_pkey";

alter table "public"."vote_registered_users" add constraint "vote_registered_users_pkey" PRIMARY KEY using index "vote_registered_users_pkey";

alter table "public"."vote_rounds" add constraint "vote_rounds_pkey" PRIMARY KEY using index "vote_rounds_pkey";

alter table "public"."activities" add constraint "activities_max_tickets_per_user_check" CHECK ((max_tickets_per_user >= 1)) not valid;

alter table "public"."activities" validate constraint "activities_max_tickets_per_user_check";

alter table "public"."activity_views" add constraint "activity_views_user_id_key" UNIQUE using index "activity_views_user_id_key";

alter table "public"."physical_sales_clients" add constraint "has_reference" CHECK (((group_id IS NULL) <> (activity_id IS NULL))) not valid;

alter table "public"."physical_sales_clients" validate constraint "has_reference";

alter table "public"."product_addon_options" add constraint "product_addon_options_price_check" CHECK ((price >= (0)::money)) not valid;

alter table "public"."product_addon_options" validate constraint "product_addon_options_price_check";

alter table "public"."product_reservation_addons" add constraint "lengths_consistent" CHECK ((cardinality(selected_options) = cardinality(selected_options_prices))) not valid;

alter table "public"."product_reservation_addons" validate constraint "lengths_consistent";

alter table "public"."product_reservation_queuers" add constraint "product_reservation_queuers_placement_check" CHECK ((placement > 0)) not valid;

alter table "public"."product_reservation_queuers" validate constraint "product_reservation_queuers_placement_check";

alter table "public"."product_reservation_queuers" add constraint "product_reservation_queuers_product_id_placement_key" UNIQUE using index "product_reservation_queuers_product_id_placement_key";

alter table "public"."product_reservations" add constraint "product_reservations_transaction_id_key" UNIQUE using index "product_reservations_transaction_id_key";

alter table "public"."product_reservations" add constraint "product_reservations_user_id_key" UNIQUE using index "product_reservations_user_id_key";

alter table "public"."products" add constraint "products_max_uses_check" CHECK ((max_uses > 0)) not valid;

alter table "public"."products" validate constraint "products_max_uses_check";

alter table "public"."products" add constraint "products_sales_price_check" CHECK ((sales_price >= (0)::money)) not valid;

alter table "public"."products" validate constraint "products_sales_price_check";

alter table "public"."products" add constraint "products_sales_stock_check" CHECK (((sales_stock IS NULL) OR (sales_stock >= 0))) not valid;

alter table "public"."products" validate constraint "products_sales_stock_check";

alter table "public"."products" add constraint "products_store_alcohol_by_volume_check" CHECK (((store_alcohol_by_volume IS NULL) OR (store_alcohol_by_volume >= 0))) not valid;

alter table "public"."products" validate constraint "products_store_alcohol_by_volume_check";

alter table "public"."purchase_transactions" add constraint "first_transaction_exists" CHECK ((NOT ((seller_id IS NULL) AND (transaction_id IS NULL)))) not valid;

alter table "public"."purchase_transactions" validate constraint "first_transaction_exists";

alter table "public"."purchased_product_addons" add constraint "lengths_consistent" CHECK ((cardinality(selected_options) = cardinality(selected_options_prices))) not valid;

alter table "public"."purchased_product_addons" validate constraint "lengths_consistent";

alter table "public"."users_in_purchase_flow" add constraint "users_in_purchase_flow_user_id_product_id_key" UNIQUE using index "users_in_purchase_flow_user_id_product_id_key";

alter table "public"."vote_alternatives" add constraint "vote_alternatives_voted_for_check" CHECK ((voted_for >= 0)) not valid;

alter table "public"."vote_alternatives" validate constraint "vote_alternatives_voted_for_check";

alter table "public"."vote_registered_clients" add constraint "vote_registered_clients_client_id_key" UNIQUE using index "vote_registered_clients_client_id_key";

alter table "public"."vote_rounds" add constraint "vote_rounds_max_choises_check" CHECK ((max_choises > 0)) not valid;

alter table "public"."vote_rounds" validate constraint "vote_rounds_max_choises_check";

alter table "public"."activities" add constraint "activities_sales_target_id_fkey" FOREIGN KEY ("sales_target_id") REFERENCES "public"."sales_targets"("id") NOT VALID;

alter table "public"."activities" validate constraint "activities_sales_target_id_fkey";

alter table "public"."activities" add constraint "activities_stock_group_fkey" FOREIGN KEY ("stock_group") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."activities" validate constraint "activities_stock_group_fkey";

alter table "public"."activity_assigned_tags" add constraint "activity_assigned_tags_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."activity_assigned_tags" validate constraint "activity_assigned_tags_activity_id_fkey";

alter table "public"."activity_assigned_tags" add constraint "activity_assigned_tags_tag_id_fkey" FOREIGN KEY ("tag_id") REFERENCES "public"."activity_tags"("id") NOT VALID;

alter table "public"."activity_assigned_tags" validate constraint "activity_assigned_tags_tag_id_fkey";

alter table "public"."activity_followers" add constraint "activity_followers_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."activity_followers" validate constraint "activity_followers_activity_id_fkey";

alter table "public"."activity_followers" add constraint "activity_followers_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."activity_followers" validate constraint "activity_followers_user_id_fkey";

alter table "public"."activity_opt_out_tags" add constraint "activity_opt_out_tags_tag_id_fkey" FOREIGN KEY ("tag_id") REFERENCES "public"."activity_tags"("id") NOT VALID;

alter table "public"."activity_opt_out_tags" validate constraint "activity_opt_out_tags_tag_id_fkey";

alter table "public"."activity_opt_out_tags" add constraint "activity_opt_out_tags_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."activity_opt_out_tags" validate constraint "activity_opt_out_tags_user_id_fkey";

alter table "public"."activity_push_notifications" add constraint "activity_push_notifications_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."activity_push_notifications" validate constraint "activity_push_notifications_activity_id_fkey";

alter table "public"."activity_push_notifications" add constraint "activity_push_notifications_notification_id_fkey" FOREIGN KEY ("notification_id") REFERENCES "public"."push_notifications"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."activity_push_notifications" validate constraint "activity_push_notifications_notification_id_fkey";

alter table "public"."activity_push_random_selected" add constraint "activity_push_random_selected_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."activity_push_random_selected" validate constraint "activity_push_random_selected_activity_id_fkey";

alter table "public"."activity_push_random_selected" add constraint "activity_push_random_selected_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."activity_push_random_selected" validate constraint "activity_push_random_selected_user_id_fkey";

alter table "public"."activity_tags" add constraint "activity_tags_creator_id_fkey" FOREIGN KEY ("creator_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."activity_tags" validate constraint "activity_tags_creator_id_fkey";

alter table "public"."activity_views" add constraint "activity_views_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."activity_views" validate constraint "activity_views_activity_id_fkey";

alter table "public"."activity_views" add constraint "activity_views_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."activity_views" validate constraint "activity_views_user_id_fkey";

alter table "public"."news" add constraint "news_questionnaire_product_fkey" FOREIGN KEY ("questionnaire_product") REFERENCES "public"."products"("id") NOT VALID;

alter table "public"."news" validate constraint "news_questionnaire_product_fkey";

alter table "public"."news" add constraint "news_target_group_id_fkey" FOREIGN KEY ("target_group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."news" validate constraint "news_target_group_id_fkey";

alter table "public"."news_assigned_tags" add constraint "news_assigned_tags_news_id_fkey" FOREIGN KEY ("news_id") REFERENCES "public"."news"("id") NOT VALID;

alter table "public"."news_assigned_tags" validate constraint "news_assigned_tags_news_id_fkey";

alter table "public"."news_assigned_tags" add constraint "news_assigned_tags_tag_id_fkey" FOREIGN KEY ("tag_id") REFERENCES "public"."news_tags"("id") NOT VALID;

alter table "public"."news_assigned_tags" validate constraint "news_assigned_tags_tag_id_fkey";

alter table "public"."news_push_notifications" add constraint "news_push_notifications_news_id_fkey" FOREIGN KEY ("news_id") REFERENCES "public"."news"("id") NOT VALID;

alter table "public"."news_push_notifications" validate constraint "news_push_notifications_news_id_fkey";

alter table "public"."news_push_notifications" add constraint "news_push_notifications_notification_id_fkey" FOREIGN KEY ("notification_id") REFERENCES "public"."push_notifications"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."news_push_notifications" validate constraint "news_push_notifications_notification_id_fkey";

alter table "public"."news_push_opt_out_tags" add constraint "news_push_opt_out_tags_tag_id_fkey" FOREIGN KEY ("tag_id") REFERENCES "public"."news_tags"("id") NOT VALID;

alter table "public"."news_push_opt_out_tags" validate constraint "news_push_opt_out_tags_tag_id_fkey";

alter table "public"."news_push_opt_out_tags" add constraint "news_push_opt_out_tags_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."news_push_opt_out_tags" validate constraint "news_push_opt_out_tags_user_id_fkey";

alter table "public"."news_tags" add constraint "news_tags_creator_id_fkey" FOREIGN KEY ("creator_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."news_tags" validate constraint "news_tags_creator_id_fkey";

alter table "public"."notification_events" add constraint "notification_events_notification_id_fkey" FOREIGN KEY ("notification_id") REFERENCES "public"."push_notifications"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."notification_events" validate constraint "notification_events_notification_id_fkey";

alter table "public"."notification_events" add constraint "notification_events_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."notification_events" validate constraint "notification_events_user_id_fkey";

alter table "public"."notification_stats" add constraint "notification_stats_notification_id_fkey" FOREIGN KEY ("notification_id") REFERENCES "public"."push_notifications"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."notification_stats" validate constraint "notification_stats_notification_id_fkey";

alter table "public"."physical_payment_terminals" add constraint "physical_payment_terminals_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."physical_payment_terminals" validate constraint "physical_payment_terminals_group_id_fkey";

alter table "public"."physical_sales_clients" add constraint "physical_sales_clients_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."physical_sales_clients" validate constraint "physical_sales_clients_activity_id_fkey";

alter table "public"."physical_sales_clients" add constraint "physical_sales_clients_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."physical_sales_clients" validate constraint "physical_sales_clients_group_id_fkey";

alter table "public"."product_activity_assignments" add constraint "product_activity_assignments_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."product_activity_assignments" validate constraint "product_activity_assignments_activity_id_fkey";

alter table "public"."product_activity_assignments" add constraint "product_activity_assignments_product_id_fkey" FOREIGN KEY ("product_id") REFERENCES "public"."products"("id") NOT VALID;

alter table "public"."product_activity_assignments" validate constraint "product_activity_assignments_product_id_fkey";

alter table "public"."product_addon_option_bookkeepings" add constraint "product_addon_option_bookkeepings_category_fkey" FOREIGN KEY ("category") REFERENCES "public"."product_tags"("id") NOT VALID;

alter table "public"."product_addon_option_bookkeepings" validate constraint "product_addon_option_bookkeepings_category_fkey";

alter table "public"."product_addon_option_bookkeepings" add constraint "product_addon_option_bookkeepings_option_id_fkey" FOREIGN KEY ("option_id") REFERENCES "public"."product_addon_options"("id") NOT VALID;

alter table "public"."product_addon_option_bookkeepings" validate constraint "product_addon_option_bookkeepings_option_id_fkey";

alter table "public"."product_addon_options" add constraint "product_addon_options_product_addon_id_fkey" FOREIGN KEY ("product_addon_id") REFERENCES "public"."product_addons"("id") NOT VALID;

alter table "public"."product_addon_options" validate constraint "product_addon_options_product_addon_id_fkey";

alter table "public"."product_addons" add constraint "product_addons_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."product_addons" validate constraint "product_addons_activity_id_fkey";

alter table "public"."product_allowed_groups" add constraint "product_allowed_groups_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."product_allowed_groups" validate constraint "product_allowed_groups_group_id_fkey";

alter table "public"."product_allowed_groups" add constraint "product_allowed_groups_product_id_fkey" FOREIGN KEY ("product_id") REFERENCES "public"."products"("id") NOT VALID;

alter table "public"."product_allowed_groups" validate constraint "product_allowed_groups_product_id_fkey";

alter table "public"."product_assigned_tags" add constraint "product_assigned_tags_product_id_fkey" FOREIGN KEY ("product_id") REFERENCES "public"."products"("id") NOT VALID;

alter table "public"."product_assigned_tags" validate constraint "product_assigned_tags_product_id_fkey";

alter table "public"."product_assigned_tags" add constraint "product_assigned_tags_tag_id_fkey" FOREIGN KEY ("tag_id") REFERENCES "public"."product_tags"("id") NOT VALID;

alter table "public"."product_assigned_tags" validate constraint "product_assigned_tags_tag_id_fkey";

alter table "public"."product_buyers" add constraint "product_buyers_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."product_buyers" validate constraint "product_buyers_activity_id_fkey";

alter table "public"."product_buyers" add constraint "product_buyers_product_id_fkey" FOREIGN KEY ("product_id") REFERENCES "public"."products"("id") NOT VALID;

alter table "public"."product_buyers" validate constraint "product_buyers_product_id_fkey";

alter table "public"."product_buyers" add constraint "product_buyers_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."product_buyers" validate constraint "product_buyers_user_id_fkey";

alter table "public"."product_release_queuers" add constraint "product_release_queuers_product_id_fkey" FOREIGN KEY ("product_id") REFERENCES "public"."products"("id") NOT VALID;

alter table "public"."product_release_queuers" validate constraint "product_release_queuers_product_id_fkey";

alter table "public"."product_release_queuers" add constraint "product_release_queuers_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."product_release_queuers" validate constraint "product_release_queuers_user_id_fkey";

alter table "public"."product_release_queuers" add constraint "product_release_queuers_user_id_product_id_fkey" FOREIGN KEY ("user_id", "product_id") REFERENCES "public"."users_in_purchase_flow"("user_id", "product_id") NOT VALID;

alter table "public"."product_release_queuers" validate constraint "product_release_queuers_user_id_product_id_fkey";

alter table "public"."product_reservation_addons" add constraint "product_reservation_addons_addon_id_fkey" FOREIGN KEY ("addon_id") REFERENCES "public"."product_addons"("id") NOT VALID;

alter table "public"."product_reservation_addons" validate constraint "product_reservation_addons_addon_id_fkey";

alter table "public"."product_reservation_addons" add constraint "product_reservation_addons_reservation_id_fkey" FOREIGN KEY ("reservation_id") REFERENCES "public"."product_reservations"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."product_reservation_addons" validate constraint "product_reservation_addons_reservation_id_fkey";

alter table "public"."product_reservation_placement_tails" add constraint "product_reservation_placement_tails_product_id_fkey" FOREIGN KEY ("product_id") REFERENCES "public"."products"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."product_reservation_placement_tails" validate constraint "product_reservation_placement_tails_product_id_fkey";

alter table "public"."product_reservation_queuers" add constraint "product_reservation_queuers_product_id_fkey" FOREIGN KEY ("product_id") REFERENCES "public"."products"("id") NOT VALID;

alter table "public"."product_reservation_queuers" validate constraint "product_reservation_queuers_product_id_fkey";

alter table "public"."product_reservation_queuers" add constraint "product_reservation_queuers_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."product_reservation_queuers" validate constraint "product_reservation_queuers_user_id_fkey";

alter table "public"."product_reservation_queuers" add constraint "product_reservation_queuers_user_id_product_id_fkey" FOREIGN KEY ("user_id", "product_id") REFERENCES "public"."users_in_purchase_flow"("user_id", "product_id") NOT VALID;

alter table "public"."product_reservation_queuers" validate constraint "product_reservation_queuers_user_id_product_id_fkey";

alter table "public"."product_reservations" add constraint "product_reservations_product_id_fkey" FOREIGN KEY ("product_id") REFERENCES "public"."products"("id") NOT VALID;

alter table "public"."product_reservations" validate constraint "product_reservations_product_id_fkey";

alter table "public"."product_reservations" add constraint "product_reservations_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."product_reservations" validate constraint "product_reservations_user_id_fkey";

alter table "public"."product_reservations" add constraint "product_reservations_user_id_product_id_fkey" FOREIGN KEY ("user_id", "product_id") REFERENCES "public"."users_in_purchase_flow"("user_id", "product_id") NOT VALID;

alter table "public"."product_reservations" validate constraint "product_reservations_user_id_product_id_fkey";

alter table "public"."product_tags" add constraint "product_tags_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."product_tags" validate constraint "product_tags_group_id_fkey";

alter table "public"."products" add constraint "products_bookkeeping_tag_fkey" FOREIGN KEY ("bookkeeping_tag") REFERENCES "public"."product_tags"("id") NOT VALID;

alter table "public"."products" validate constraint "products_bookkeeping_tag_fkey";

alter table "public"."products" add constraint "products_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."products" validate constraint "products_group_id_fkey";

alter table "public"."products" add constraint "products_image_id_fkey" FOREIGN KEY ("image_id") REFERENCES "public"."images"("id") NOT VALID;

alter table "public"."products" validate constraint "products_image_id_fkey";

alter table "public"."products" add constraint "products_sales_organization_fkey" FOREIGN KEY ("sales_organization") REFERENCES "public"."sales_targets"("id") NOT VALID;

alter table "public"."products" validate constraint "products_sales_organization_fkey";

alter table "public"."purchase_transactions" add constraint "purchase_transactions_buyer_id_fkey" FOREIGN KEY ("buyer_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."purchase_transactions" validate constraint "purchase_transactions_buyer_id_fkey";

alter table "public"."purchase_transactions" add constraint "purchase_transactions_purchase_id_fkey" FOREIGN KEY ("purchase_id") REFERENCES "public"."purchases"("id") NOT VALID;

alter table "public"."purchase_transactions" validate constraint "purchase_transactions_purchase_id_fkey";

alter table "public"."purchase_transactions" add constraint "purchase_transactions_seller_id_fkey" FOREIGN KEY ("seller_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."purchase_transactions" validate constraint "purchase_transactions_seller_id_fkey";

alter table "public"."purchase_use_validations" add constraint "purchase_use_validations_use_id_fkey" FOREIGN KEY ("use_id") REFERENCES "public"."purchase_uses"("id") NOT VALID;

alter table "public"."purchase_use_validations" validate constraint "purchase_use_validations_use_id_fkey";

alter table "public"."purchase_uses" add constraint "purchase_uses_purchase_id_fkey" FOREIGN KEY ("purchase_id") REFERENCES "public"."purchases"("id") NOT VALID;

alter table "public"."purchase_uses" validate constraint "purchase_uses_purchase_id_fkey";

alter table "public"."purchased_product_addons" add constraint "purchased_product_addons_addon_id_fkey" FOREIGN KEY ("addon_id") REFERENCES "public"."product_addons"("id") NOT VALID;

alter table "public"."purchased_product_addons" validate constraint "purchased_product_addons_addon_id_fkey";

alter table "public"."purchased_product_addons" add constraint "purchased_product_addons_purchase_id_fkey" FOREIGN KEY ("purchase_id") REFERENCES "public"."purchases"("id") NOT VALID;

alter table "public"."purchased_product_addons" validate constraint "purchased_product_addons_purchase_id_fkey";

alter table "public"."purchased_ticket_push_notifications" add constraint "purchased_ticket_push_notifications_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."purchased_ticket_push_notifications" validate constraint "purchased_ticket_push_notifications_activity_id_fkey";

alter table "public"."purchased_ticket_push_notifications" add constraint "purchased_ticket_push_notifications_notification_id_fkey" FOREIGN KEY ("notification_id") REFERENCES "public"."push_notifications"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."purchased_ticket_push_notifications" validate constraint "purchased_ticket_push_notifications_notification_id_fkey";

alter table "public"."purchases" add constraint "purchases_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."purchases" validate constraint "purchases_activity_id_fkey";

alter table "public"."purchases" add constraint "purchases_owner_id_fkey" FOREIGN KEY ("owner_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."purchases" validate constraint "purchases_owner_id_fkey";

alter table "public"."purchases" add constraint "purchases_product_id_fkey" FOREIGN KEY ("product_id") REFERENCES "public"."products"("id") NOT VALID;

alter table "public"."purchases" validate constraint "purchases_product_id_fkey";

alter table "public"."purchases" add constraint "purchases_purchaser_id_fkey" FOREIGN KEY ("purchaser_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."purchases" validate constraint "purchases_purchaser_id_fkey";

alter table "public"."sales_targets" add constraint "sales_targets_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."sales_targets" validate constraint "sales_targets_group_id_fkey";

alter table "public"."stock_transaction_records" add constraint "stock_transaction_records_product_id_fkey" FOREIGN KEY ("product_id") REFERENCES "public"."products"("id") NOT VALID;

alter table "public"."stock_transaction_records" validate constraint "stock_transaction_records_product_id_fkey";

alter table "public"."stock_transaction_records" add constraint "stock_transaction_records_transaction_id_fkey" FOREIGN KEY ("transaction_id") REFERENCES "public"."stock_transactions"("id") NOT VALID;

alter table "public"."stock_transaction_records" validate constraint "stock_transaction_records_transaction_id_fkey";

alter table "public"."stock_transactions" add constraint "stock_transactions_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."stock_transactions" validate constraint "stock_transactions_activity_id_fkey";

alter table "public"."stock_transactions" add constraint "stock_transactions_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."stock_transactions" validate constraint "stock_transactions_group_id_fkey";

alter table "public"."ticket_release_groups" add constraint "ticket_release_groups_push_id_push_ticket_id_fkey" FOREIGN KEY ("push_id", "push_ticket_id") REFERENCES "public"."ticket_release_push_notifications"("id", "ticket_id") ON DELETE CASCADE NOT VALID;

alter table "public"."ticket_release_groups" validate constraint "ticket_release_groups_push_id_push_ticket_id_fkey";

alter table "public"."ticket_release_groups" add constraint "ticket_release_groups_to_group_id_fkey" FOREIGN KEY ("to_group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."ticket_release_groups" validate constraint "ticket_release_groups_to_group_id_fkey";

alter table "public"."ticket_release_push_notifications" add constraint "ticket_release_push_notifications_notification_id_fkey" FOREIGN KEY ("notification_id") REFERENCES "public"."push_notifications"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."ticket_release_push_notifications" validate constraint "ticket_release_push_notifications_notification_id_fkey";

alter table "public"."ticket_release_push_notifications" add constraint "ticket_release_push_notifications_ticket_id_fkey" FOREIGN KEY ("ticket_id") REFERENCES "public"."products"("id") NOT VALID;

alter table "public"."ticket_release_push_notifications" validate constraint "ticket_release_push_notifications_ticket_id_fkey";

alter table "public"."users_in_purchase_flow" add constraint "users_in_purchase_flow_product_id_fkey" FOREIGN KEY ("product_id") REFERENCES "public"."products"("id") NOT VALID;

alter table "public"."users_in_purchase_flow" validate constraint "users_in_purchase_flow_product_id_fkey";

alter table "public"."vote_alternatives" add constraint "vote_alternatives_round_id_fkey" FOREIGN KEY ("round_id") REFERENCES "public"."vote_rounds"("id") NOT VALID;

alter table "public"."vote_alternatives" validate constraint "vote_alternatives_round_id_fkey";

alter table "public"."vote_cast_votes" add constraint "vote_cast_votes_client_id_fkey" FOREIGN KEY ("client_id") REFERENCES "public"."vote_registered_clients"("client_id") NOT VALID;

alter table "public"."vote_cast_votes" validate constraint "vote_cast_votes_client_id_fkey";

alter table "public"."vote_cast_votes" add constraint "vote_cast_votes_round_id_fkey" FOREIGN KEY ("round_id") REFERENCES "public"."vote_rounds"("id") NOT VALID;

alter table "public"."vote_cast_votes" validate constraint "vote_cast_votes_round_id_fkey";

alter table "public"."vote_meeting_admins" add constraint "vote_meeting_admins_meeting_id_fkey" FOREIGN KEY ("meeting_id") REFERENCES "public"."vote_meetings"("id") NOT VALID;

alter table "public"."vote_meeting_admins" validate constraint "vote_meeting_admins_meeting_id_fkey";

alter table "public"."vote_meeting_admins" add constraint "vote_meeting_admins_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."vote_meeting_admins" validate constraint "vote_meeting_admins_user_id_fkey";

alter table "public"."vote_meetings" add constraint "vote_meetings_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."vote_meetings" validate constraint "vote_meetings_activity_id_fkey";

alter table "public"."vote_meetings" add constraint "vote_meetings_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."vote_meetings" validate constraint "vote_meetings_group_id_fkey";

alter table "public"."vote_registered_clients" add constraint "vote_registered_clients_meeting_id_fkey" FOREIGN KEY ("meeting_id") REFERENCES "public"."vote_meetings"("id") NOT VALID;

alter table "public"."vote_registered_clients" validate constraint "vote_registered_clients_meeting_id_fkey";

alter table "public"."vote_registered_users" add constraint "vote_registered_users_meeting_id_fkey" FOREIGN KEY ("meeting_id") REFERENCES "public"."vote_meetings"("id") NOT VALID;

alter table "public"."vote_registered_users" validate constraint "vote_registered_users_meeting_id_fkey";

alter table "public"."vote_registered_users" add constraint "vote_registered_users_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."vote_registered_users" validate constraint "vote_registered_users_user_id_fkey";

alter table "public"."vote_rounds" add constraint "vote_rounds_meeting_id_fkey" FOREIGN KEY ("meeting_id") REFERENCES "public"."vote_meetings"("id") NOT VALID;

alter table "public"."vote_rounds" validate constraint "vote_rounds_meeting_id_fkey";

alter table "public"."ticket_kind_addons" add constraint "ticket_kind_addons_addon_id_fkey" FOREIGN KEY ("addon_id") REFERENCES "public"."product_addons"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."ticket_kind_addons" validate constraint "ticket_kind_addons_addon_id_fkey";

alter table "public"."ticket_kind_addons" add constraint "ticket_kind_addons_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."products"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."ticket_kind_addons" validate constraint "ticket_kind_addons_ticket_kind_id_fkey";

alter table "public"."ticket_kind_transfer_groups" add constraint "ticket_kind_transfer_groups_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."products"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."ticket_kind_transfer_groups" validate constraint "ticket_kind_transfer_groups_ticket_kind_id_fkey";

alter table "public"."activities" add constraint "activities_image_id_fkey" FOREIGN KEY ("image_id") REFERENCES "public"."images"("id") NOT VALID;

alter table "public"."activities" validate constraint "activities_image_id_fkey";

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

alter table "public"."allow_admins_from_group_view_activities" add constraint "allow_admins_from_group_view_activities_access_group_id_fkey" FOREIGN KEY ("access_group_id") REFERENCES "public"."groups"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."allow_admins_from_group_view_activities" validate constraint "allow_admins_from_group_view_activities_access_group_id_fkey";

alter table "public"."allow_admins_from_group_view_activities" add constraint "allow_admins_from_group_view_activities_host_group_id_fkey" FOREIGN KEY ("host_group_id") REFERENCES "public"."groups"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."allow_admins_from_group_view_activities" validate constraint "allow_admins_from_group_view_activities_host_group_id_fkey";

alter table "public"."group_adminships" add constraint "group_adminships_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."group_adminships" validate constraint "group_adminships_group_id_fkey";

alter table "public"."group_adminships" add constraint "group_adminships_group_membership_fk" FOREIGN KEY ("user_id", "group_id") REFERENCES "public"."group_memberships"("user_id", "group_id") NOT VALID;

alter table "public"."group_adminships" validate constraint "group_adminships_group_membership_fk";

alter table "public"."group_member_requests" add constraint "group_member_requests_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."group_member_requests" validate constraint "group_member_requests_group_id_fkey";

alter table "public"."group_memberships" add constraint "group_memberships_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."group_memberships" validate constraint "group_memberships_group_id_fkey";

alter table "public"."groups" add constraint "groups_logo_id_fkey" FOREIGN KEY ("logo_id") REFERENCES "public"."images"("id") NOT VALID;

alter table "public"."groups" validate constraint "groups_logo_id_fkey";

alter table "public"."groups" add constraint "groups_parent_path_fkey" FOREIGN KEY ("parent_path") REFERENCES "public"."groups"("path") NOT VALID;

alter table "public"."groups" validate constraint "groups_parent_path_fkey";

alter table "public"."groups_ask_to_join" add constraint "groups_ask_to_join_joiner_id_fkey" FOREIGN KEY ("joiner_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."groups_ask_to_join" validate constraint "groups_ask_to_join_joiner_id_fkey";

alter table "public"."groups_ask_to_join" add constraint "groups_ask_to_join_target_id_fkey" FOREIGN KEY ("target_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."groups_ask_to_join" validate constraint "groups_ask_to_join_target_id_fkey";

alter table "public"."ticket_kind_transfer_groups" add constraint "ticket_kind_transfer_groups_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."ticket_kind_transfer_groups" validate constraint "ticket_kind_transfer_groups_group_id_fkey";

alter table "public"."users_in_purchase_flow" add constraint "users_in_purchase_flow_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."users_in_purchase_flow" validate constraint "users_in_purchase_flow_user_id_fkey";

drop table "public"."activity_buyers_notifications";
drop table "public"."activity_notification_overrides";
drop table "public"."activity_notifications";
drop table "public"."group_notifications";
drop table "public"."notifications";
drop table "public"."purchased_ticket_addons";
drop table "public"."purchased_ticket_notifications";
drop table "public"."purchased_ticket_validations";
drop table "public"."purchased_tickets";
drop table "public"."ticket_addon_options";
drop table "public"."ticket_addons";
drop table "public"."ticket_kind_allowed_groups";
drop table "public"."ticket_kinds";
drop table "public"."ticket_release_queuers";
drop table "public"."ticket_reservation_addons";
drop table "public"."ticket_reservation_placement_tails";
drop table "public"."ticket_reservation_queuers";
drop table "public"."ticket_reservations";
drop table "public"."user_group_settings";

-- consistent product naming for the two surviving ticket link tables
alter index if exists "public"."ticket_kind_transfer_groups_by_ticket_kind" rename to "product_transfer_groups_by_product";
alter table "public"."ticket_kind_addons" rename to "product_addon_links";
alter table "public"."product_addon_links" rename column "ticket_kind_id" to "product_id";
alter table "public"."product_addon_links" rename constraint "ticket_kind_addons_pkey" to "product_addon_links_pkey";
alter table "public"."product_addon_links" rename constraint "ticket_kind_addons_addon_id_fkey" to "product_addon_links_addon_id_fkey";
alter table "public"."product_addon_links" rename constraint "ticket_kind_addons_ticket_kind_id_fkey" to "product_addon_links_product_id_fkey";

alter table "public"."ticket_kind_transfer_groups" rename to "product_transfer_groups";
alter table "public"."product_transfer_groups" rename column "ticket_kind_id" to "product_id";
alter table "public"."product_transfer_groups" rename constraint "ticket_kind_transfer_groups_pkey" to "product_transfer_groups_pkey";
alter table "public"."product_transfer_groups" rename constraint "ticket_kind_transfer_groups_group_id_fkey" to "product_transfer_groups_group_id_fkey";
alter table "public"."product_transfer_groups" rename constraint "ticket_kind_transfer_groups_ticket_kind_id_fkey" to "product_transfer_groups_product_id_fkey";

alter table "public"."news" validate constraint "news_questionnaire_product_fkey";
alter table "public"."product_activity_assignments" validate constraint "product_activity_assignments_product_id_fkey";
alter table "public"."product_transfer_groups" validate constraint "product_transfer_groups_product_id_fkey";

drop type "public"."notification_level";

alter table "public"."purchases" add column "resale_transaction_id" uuid;
alter table "public"."purchases" add column "resale_buyer_id" text;
alter table "public"."purchases" add constraint "purchases_resale_transaction_id_key" unique ("resale_transaction_id");
alter table "public"."purchases" add constraint "purchases_resale_buyer_id_fkey" foreign key ("resale_buyer_id") references "public"."users"("id");
