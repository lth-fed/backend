-- Transfer availability was previously restricted to the same groups as purchasing unless the
-- unrestricted bypass was enabled. Preserve the bounded/default policy for every ticket kind
-- whose transfer interval is enabled; legacy unrestricted bypasses intentionally stay bounded.
insert into ticket_kind_transfer_groups (ticket_kind_id, group_id)
select allowed.ticket_kind_id, allowed.group_id
from ticket_kind_allowed_groups allowed
inner join ticket_kinds kind on kind.id = allowed.ticket_kind_id
where kind.allow_transfer_ticket_stop > kind.allow_transfer_ticket_start
on conflict do nothing;
