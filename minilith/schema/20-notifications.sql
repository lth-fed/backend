create type push_platform as enum ('ios', 'android');
-- what a push refers to; also sent to clients in the push payload and notification API
create type notification_kind as enum ('activity', 'news', 'ticket_release', 'transfer', 'reservation', 'reservation_queue');

create table push_devices (
    device_id text primary key,
    user_id text not null references users (id) on delete cascade,
    push_token text not null,
    platform push_platform not null,
    updated_at timestamptz not null default now(),
    unique (platform, push_token)
);

create table news (
    id uuid primary key,
    target_group_id uuid not null references groups(id),
    -- not markdown
    title jsonb not null,
    -- markdown
    content jsonb not null,
    -- used to render questionnaires, which are implemented using product addons.
    questionnaire_product uuid references products(id),
    published_at timestamptz not null
);
create table news_tags (
    id uuid primary key,
    creator_id uuid not null references groups(id),
    title jsonb not null,
    -- false for tags everyone gets, such as "Important": they can't be opted out of
    can_opt_out boolean not null default true
);
create table news_assigned_tags (
    news_id uuid not null references news (id),
    tag_id uuid not null references news_tags (id),
    primary key (news_id, tag_id)
);
create table news_push_opt_out_tags (
    user_id text not null references users(id),
    tag_id uuid not null references news_tags(id),
    primary key (user_id, tag_id)
);

create table push_notifications (
    id uuid primary key,
    kind notification_kind not null,
    sender jsonb not null,
    title jsonb not null,
    -- not markdown
    content jsonb not null,
    send_at timestamptz not null,
    sent boolean not null default false
);

-- per-notification read state, overriding the watermark below: `read` null follows the watermark,
-- true/false is the user's own choice (marking read or unread). `dismissed` hides it from the inbox.
create table notification_reads (
    user_id text not null references users(id) on delete cascade,
    notification_id uuid not null references push_notifications(id) on delete cascade,
    read_at timestamptz not null default now(),
    read boolean,
    dismissed boolean not null default false,
    primary key (user_id, notification_id)
);
-- "mark all read" watermark; falls back to users.creation so new accounts don't see
-- old history as unread
create table user_notification_state (
    user_id text primary key references users(id) on delete cascade,
    last_full_read timestamptz not null default now()
);

-- update on any change to activity visibility
create table activity_push_random_selected (
    activity_id uuid not null references activities(id) on delete cascade,
    user_id text not null references users(id) on delete cascade,
    primary key (activity_id, user_id)
);

-- specific tables for specific kind of notifications, so we can at send time look up which users should get them

-- for ticket releases
-- ONLY send for base ticket & later additional tickets
create table ticket_release_push_notifications (
    id uuid not null,
    ticket_id uuid not null references products (id),
    primary key (id, ticket_id),
    notification_id uuid not null references push_notifications (id) on delete cascade
);
-- to those who can see an activity
create table activity_push_notifications (
    id uuid not null,
    activity_id uuid not null references activities (id),
    primary key (id, activity_id),
    notification_id uuid not null references push_notifications (id) on delete cascade
);
-- restrict a ticket release notification to specific groups
create table ticket_release_groups (
    push_id uuid not null,
    push_ticket_id uuid not null,
    to_group_id uuid not null references groups(id),
    primary key (push_id, push_ticket_id, to_group_id),
    foreign key (push_id, push_ticket_id)
        references ticket_release_push_notifications (id, ticket_id) on delete cascade
);
-- to those who have bought a ticket
create table purchased_ticket_push_notifications (
    id uuid not null,
    activity_id uuid not null references activities (id),
    primary key (id, activity_id),
    notification_id uuid not null references push_notifications (id) on delete cascade
);
-- To a news, which is published from a group
create table news_push_notifications (
    id uuid primary key,
    news_id uuid not null references news (id),
    notification_id uuid not null references push_notifications (id) on delete cascade
);

-- Whether `target_user` gets an activity's news: their own choice for it (the activity's bell,
-- `activity_followers`) if they made one, otherwise yes unless they opted out of every tag the
-- activity carries. An activity without tags reaches everyone, and one with a tag the user still
-- wants (such as "Important") reaches them whatever else it is tagged with.
create function follows_activity(target_user text, target_activity uuid) returns boolean
language sql stable as $$
    select coalesce(
        (
            select follow from activity_followers
            where user_id = target_user and activity_id = target_activity
        ),
        not exists (
            select 1 from activity_assigned_tags assigned
            where assigned.activity_id = target_activity
        )
        or exists (
            select 1 from activity_assigned_tags assigned
            where assigned.activity_id = target_activity
            and not exists (
                select 1 from activity_opt_out_tags opt_out
                where opt_out.user_id = target_user and opt_out.tag_id = assigned.tag_id
            )
        )
    )
$$;

-- Whether `target_user` gets a push for a news post: yes unless they opted out of every tag the
-- post carries (the same rule as `follows_activity`, without a per-post choice).
create function wants_news(target_user text, target_news uuid) returns boolean
language sql stable as $$
    select not exists (
            select 1 from news_assigned_tags assigned
            where assigned.news_id = target_news
        )
        or exists (
            select 1 from news_assigned_tags assigned
            where assigned.news_id = target_news
            and not exists (
                select 1 from news_push_opt_out_tags opt_out
                where opt_out.user_id = target_user and opt_out.tag_id = assigned.tag_id
            )
        )
$$;

-- Who a notification is for. `receives` applies the user's preferences: a row with `receives`
-- false is someone eligible who muted it. Pushes and the history only go to `receives`; an
-- activity's own notification list shows everything the user is eligible for.
--
-- Activity news reaches everyone who can see the activity unless they muted it
-- (`follows_activity`). News for ticket holders can't be muted. Hidden activities send nothing.
create view notification_recipients as
with eligible as (
    -- ticket release: members of the groups allowed to buy the ticket (and their descendants),
    -- muted with the ticket's activity
    select
        release_notification.notification_id,
        group_memberships.user_id,
        coalesce(
            (
                select bool_or(follows_activity(group_memberships.user_id, assignment.activity_id))
                from product_activity_assignments assignment
                where assignment.product_id = release_notification.ticket_id
            ),
            true
        ) as receives
    from ticket_release_push_notifications release_notification
    inner join product_allowed_groups
        on product_allowed_groups.product_id = release_notification.ticket_id
    inner join groups allowed_group
        on allowed_group.id = product_allowed_groups.group_id
    inner join groups member_group
        on allowed_group.path @> member_group.path
    inner join group_memberships
        on group_memberships.group_id = member_group.id
    where member_group.id = allowed_group.id
        or not group_memberships.specific

    union all

    -- everyone who owns a product for the activity; can't be muted
    select
        purchased_notification.notification_id,
        purchases.owner_id,
        true
    from purchased_ticket_push_notifications purchased_notification
    inner join activities activity
        on activity.id = purchased_notification.activity_id and not activity.is_hidden
    inner join purchases
        on purchases.activity_id = purchased_notification.activity_id

    union all

    -- everyone who can see the activity: members of the groups allowed to buy one of its products
    select
        activity_notification.notification_id,
        group_memberships.user_id,
        follows_activity(group_memberships.user_id, activity_notification.activity_id)
    from activity_push_notifications activity_notification
    inner join activities activity
        on activity.id = activity_notification.activity_id and not activity.is_hidden
    inner join product_activity_assignments assignment
        on assignment.activity_id = activity_notification.activity_id
    inner join product_allowed_groups
        on product_allowed_groups.product_id = assignment.product_id
    inner join groups allowed_group
        on allowed_group.id = product_allowed_groups.group_id
    inner join groups member_group
        on allowed_group.path @> member_group.path
    inner join group_memberships
        on group_memberships.group_id = member_group.id
    where member_group.id = allowed_group.id
        or not group_memberships.specific

    union all

    -- the activity's followers
    select
        activity_notification.notification_id,
        activity_followers.user_id,
        true
    from activity_push_notifications activity_notification
    inner join activities activity
        on activity.id = activity_notification.activity_id and not activity.is_hidden
    inner join activity_followers
        on activity_followers.activity_id = activity_notification.activity_id
    where activity_followers.follow

    union all

    -- people who are guaranteed / invited to buy something for the activity
    select
        activity_notification.notification_id,
        product_buyers.user_id,
        follows_activity(product_buyers.user_id, activity_notification.activity_id)
    from activity_push_notifications activity_notification
    inner join activities activity
        on activity.id = activity_notification.activity_id and not activity.is_hidden
    inner join product_buyers
        on product_buyers.activity_id = activity_notification.activity_id

    union all

    -- members of a hosting group (and descendants)
    select
        activity_notification.notification_id,
        group_memberships.user_id,
        follows_activity(group_memberships.user_id, activity_notification.activity_id)
    from activity_push_notifications activity_notification
    inner join activities activity
        on activity.id = activity_notification.activity_id and not activity.is_hidden
    inner join activity_hosts
        on activity_hosts.activity_id = activity_notification.activity_id
    inner join groups host_group
        on host_group.id = activity_hosts.group_id
    inner join groups member_group
        on host_group.path @> member_group.path
    inner join group_memberships
        on group_memberships.group_id = member_group.id
    where member_group.id = host_group.id
        or not group_memberships.specific

    union all

    -- the random sample of everyone else who can see the activity
    select
        activity_notification.notification_id,
        activity_push_random_selected.user_id,
        follows_activity(activity_push_random_selected.user_id, activity_notification.activity_id)
    from activity_push_notifications activity_notification
    inner join activities activity
        on activity.id = activity_notification.activity_id and not activity.is_hidden
    inner join activity_push_random_selected
        on activity_push_random_selected.activity_id = activity_notification.activity_id

    union all

    -- news: members of the target group (and descendants), muted by news tag
    select
        news_notification.notification_id,
        group_memberships.user_id,
        wants_news(group_memberships.user_id, news.id)
    from news_push_notifications news_notification
    inner join news on news.id = news_notification.news_id
    inner join groups target_group on target_group.id = news.target_group_id
    inner join groups member_group on target_group.path @> member_group.path
    inner join group_memberships
        on group_memberships.group_id = member_group.id
    where member_group.id = target_group.id
        or not group_memberships.specific
)
select notification_id, user_id, bool_or(receives) as receives
from eligible
group by notification_id, user_id;
