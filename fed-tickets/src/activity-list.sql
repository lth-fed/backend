select distinct on (a.id)
    a.id,
    creator.name as "creator_name!: DIS",
    creator.path,
    a.title as "title!: DIS",
    a.description as "description!: DIS",
    a.location as "location!: Location",
    a.time_start,
    a.time_end,
    img.url
from group_memberships
inner join groups member_group on member_group.id = group_memberships.group_id
-- get the ticket_kinds we're allowed to purchase
inner join groups allowed_group on allowed_group.path @> member_group.path
inner join ticket_kind_allowed_groups tk_ag on tk_ag.group_id = allowed_group.id
inner join ticket_kinds kind on kind.id = tk_ag.ticket_kind_id
-- get the activity
inner join activities a on a.id = kind.activity_id

-- extra data (not used for logic)
inner join groups creator on creator.id = a.creator_id
inner join images img on img.id = a.image_id

where group_memberships.user_id = $1
and (
    member_group.limit_membership_visibility = false
    or tk_ag.group_id = group_memberships.group_id
)
and time_end + '6 hours' > now()
and not exists (
    select 1
    from user_group_settings as settings
    inner join groups on groups.id = settings.group_id
    where settings.user_id = $1
    and groups.path @> creator.path
    and settings.visible = false
)
--order by time_start asc
limit 100
