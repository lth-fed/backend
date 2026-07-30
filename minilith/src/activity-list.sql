with visible_activities as (
    select distinct on (kind.activity_id)
        kind.activity_id
    from group_memberships
    inner join groups member_group on member_group.id = group_memberships.group_id
    -- get the ticket_kinds we're allowed to purchase
    inner join groups allowed_group on allowed_group.path @> member_group.path
    inner join ticket_kind_allowed_groups tk_ag on tk_ag.group_id = allowed_group.id
    inner join ticket_kinds kind on kind.id = tk_ag.ticket_kind_id

    where group_memberships.user_id = $1
    and (
        member_group.limit_membership_visibility = false
        or tk_ag.group_id = group_memberships.group_id
    )
    -- filters
    and exists (
        select 1
        from activity_hosts
        inner join groups as activity_group
            on activity_group.id = activity_hosts.group_id
        left join lateral (
            select settings.visible
            from user_group_settings as settings
            inner join groups as settings_group
                on settings_group.id = settings.group_id
            where settings.user_id = $1
            and settings_group.path @> activity_group.path
            order by nlevel(settings_group.path) desc
            limit 1
        ) as closest_setting on true
        where activity_hosts.activity_id = kind.activity_id
        and coalesce(closest_setting.visible, true)
    )

    union all

    -- admin
    -- explicitly invited to view other group's activities
    select distinct on (host.activity_id)
        host.activity_id
    from group_adminships
    inner join groups admin_group on admin_group.id = group_adminships.group_id
    inner join allow_admins_from_group_view_activities allowed_to_view on (allowed_to_view.access_group_id = admin_group.id)
    inner join activity_hosts host on (host.group_id = allowed_to_view.host_group_id)
    inner join activities a on a.id = host.activity_id
    where
        group_adminships.user_id = $1
        and (
            a.is_hidden_for_other_admins = false
            or host.group_id = admin_group.id
        )

    union all

    -- admin
    -- one's own events
    select distinct on (host.activity_id)
        host.activity_id
    from group_adminships
    inner join groups admin_group on admin_group.id = group_adminships.group_id
    inner join groups subgroup on admin_group.path @> sungroup.path
    inner join activity_hosts host on (host.group_id = subgroup.id)
    where
        group_adminships.user_id = $1
)

select a.id,
    a.title as "title!: DIS",
    a.description as "description!: DIS",
    a.location as "location!: Location",
    a.time_start,
    a.time_end,
    a.is_hidden,
    img.url,
    creator.name as "creator_name!: DIS",
    creator.path as creator_path
from visible_activities
-- get the activity
inner join activities a on a.id = activity_id

-- extra data
inner join groups creator on creator.id = a.creator_id
inner join images img on img.id = a.image_id
where a.is_hidden = false
    and time_end + '6 hours' > now()
order by time_start, a.id
-- todo: add paging by month for admin calendar
limit 100
