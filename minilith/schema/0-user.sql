create table users (
    -- `lund-university:<stil-id>` or `email:<email>`
    id text primary key,
    -- needs to be pseudoanonymised according to GDPR, therefore we have to store the encrypted bytes. Remember to pad.
    name bytea not null,
    -- same as for name
    language bytea not null,
    -- GDPR remove all accounts older than 2 years after this: this is a GDPR requirement
    latest_refresh timestamptz not null default now(),
    creation timestamptz not null default now(),
    -- don't remove name when becoming inactive!
    inactive_since timestamptz
    -- constraint no_name_when_deleted check ((not is_active) and name = null or is_active and name != null)
);

create table admin_personal_accounts (
    admin_id text primary key references users(id) on delete cascade check (admin_id like 'email:%'),
    user_id text not null references users(id) on delete cascade
);
create index admin_personal_accounts_user on admin_personal_accounts(user_id);
