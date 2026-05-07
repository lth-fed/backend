create table users (
    -- `lund-university:<stil-id>` or `email:<email>`
    id text primary key,
    -- needs to be pseudoanonymised according to GDPR, therefore we have to store the encrypted bytes
    name bytea not null,
    -- same as for name, also remember to salt so the encrypted data isn't the same for all users with the same language
    language bytea not null,
    -- GDPR remove all accounts older than 2 years after this: this is a GDPR requirement
    latest_refresh timestamp not null,
    -- don't remove name when becoming inactive!
    inactive_since timestamp
    -- constraint no_name_when_deleted check ((not is_active) and name = null or is_active and name != null)
);
