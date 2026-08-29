create table ticket_kind_transfer_groups (
    ticket_kind_id uuid not null references ticket_kinds(id) on delete cascade,
    group_id uuid not null references groups(id),
    primary key (group_id, ticket_kind_id)
);

create index ticket_kind_transfer_groups_by_ticket_kind
    on ticket_kind_transfer_groups using hash (ticket_kind_id);

alter table ticket_kinds
    drop column allow_transfer_ticket_bypass_allowed_groups;
