-- ALSO UPDATE THE ACCESS QUERY IN `./context.rs`

with visible_activities as (
    select distinct on (assignment.activity_id)
        assignment.activity_id,
        false as admin_access
    from group_memberships
    inner join groups member_group on member_group.id = group_memberships.group_id
    -- products we're allowed to purchase
    inner join groups allowed_group on allowed_group.path @> member_group.path
    inner join product_allowed_groups tk_ag on tk_ag.group_id = allowed_group.id
    inner join product_activity_assignments assignment
        on assignment.product_id = tk_ag.product_id

    where group_memberships.user_id = $1
    and (
        not group_memberships.specific
        or tk_ag.group_id = group_memberships.group_id
    )
    -- filters
    and exists (
        select 1
        from activity_hosts
        where activity_hosts.activity_id = assignment.activity_id
    )

    union all

    select activity_id, false as admin_access
    from product_buyers
    where user_id = $1
    and exists (select 1 from activity_hosts where activity_hosts.activity_id = product_buyers.activity_id)

    union all

    -- admin
    -- explicitly invited to view other group's activities
    select distinct on (host.activity_id)
        host.activity_id,
        true as admin_access
    from group_adminships
    inner join groups admin_group on admin_group.id = group_adminships.group_id
    inner join groups supergroup on admin_group.path <@ supergroup.path
    inner join allow_admins_from_group_view_activities allowed_to_view on (allowed_to_view.access_group_id = supergroup.id)
    inner join activity_hosts host on (host.group_id = allowed_to_view.host_group_id)
    inner join activities a on a.id = host.activity_id
    where
        (group_adminships.user_id = $1 or group_adminships.user_id in
            (select admin_id from admin_personal_accounts where user_id = $1))
        and (
            a.is_hidden_for_other_admins = false
            or host.group_id = admin_group.id
        )

    union all

    -- admin
    -- one's own and one's parents' events
    select distinct on (host.activity_id)
        host.activity_id,
        true as admin_access
    from group_adminships
    inner join groups admin_group on admin_group.id = group_adminships.group_id
    inner join groups subgroup on (admin_group.path @> subgroup.path 
        or admin_group.path <@ subgroup.path)
    inner join activity_hosts host on (host.group_id = subgroup.id)
    where
        (group_adminships.user_id = $1 or group_adminships.user_id in
            (select admin_id from admin_personal_accounts where user_id = $1))
),
-- this is here so that if the query gets a result from an admin and non-admin pathway, we always
-- get the admin_access = true
visible_activity_ids as (
    select activity_id, bool_or(admin_access) as admin_access
    from visible_activities
    group by activity_id
)

select a.id,
    array(
        select host.group_id
        from activity_hosts host
        where host.activity_id = a.id
    ) as "host_ids!: Vec<Uuid>",
    array(
        select host_group.path::text
        from activity_hosts host
        inner join groups host_group on host_group.id = host.group_id
        where host.activity_id = a.id
        order by host_group.path
    ) as "host_paths!: Vec<String>",
    array(
        select assigned.tag_id
        from activity_assigned_tags assigned
        where assigned.activity_id = a.id
        order by assigned.tag_id
    ) as "tag_ids!: Vec<Uuid>",
    a.title as "title!: DIS",
    a.description as "description!: DIS",
    a.location as "location!: Location",
    a.time_start,
    a.time_end,
    a.is_hidden,
    img.url,
    creator.name as "sales_target_group_name!: DIS",
    creator.path as sales_target_group_path,
    (select tk.sales_start
        from products tk
        inner join product_activity_assignments assignment on assignment.product_id = tk.id
        where assignment.activity_id = a.id
        and tk.kind = 'ticket'
        and (exists (
            select 1 from product_allowed_groups g
            inner join groups ag on ag.id = g.group_id
            inner join groups ug on ag.path @> ug.path
            inner join group_memberships m on m.group_id = ug.id
            where g.product_id = tk.id and m.user_id = $1
            and (not m.specific or g.group_id = m.group_id)
        ) or exists (
            select 1 from product_buyers b
            where b.product_id = tk.id and b.activity_id = a.id and b.user_id = $1
        ))
        and tk.sales_stop > now()
        and coalesce(tk.sales_stock, 0) > 0
        order by tk.sales_start
        limit 1
    ) as "earliest_ticket_release?"
from visible_activity_ids
-- get the activity
inner join activities a on a.id = activity_id

-- extra data
inner join sales_targets target on target.id = a.sales_target_id
inner join groups creator on creator.id = target.group_id
inner join images img on img.id = a.image_id
where (a.is_hidden = false or admin_access)
    -- $2: paging start, $3: paging end
    -- if $2 == null, apply this filter
    and ($2::timestamptz is not null or time_end + '6 hours' > now())
    -- if $2 != null, apply this filter
    and ($2::timestamptz is null or (time_end > $2::timestamptz and time_start < $3::timestamptz))
order by time_start, a.id
