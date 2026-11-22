-- we do not have any fancy crypto system here, since this system is going to be deployed on one 
-- server. Besides, we developers have access to deploy a new version, therefore we have access 
-- to deploy a malicious version and cheat. Therefore, keeping the info limited so one cannot after
-- the fact look up who voted what etc, limits possible abuse by future developer on current data.

create table vote_meetings (
    id uuid primary key,
    group_id uuid not null references groups(id),
    -- not null if we host a vote on a meeting
    activity_id uuid references activities (id),
    -- if we should make sure it's one vote per person, requires registering devices manually
    secure_voting bool not null,
    title jsonb not null
);
create table vote_meeting_admins (
    user_id text not null,
    meeting_id uuid references vote_meetings(id) on delete cascade,
    -- cannot create new rounds / close rounds
    readonly bool not null,
    primary key (user_id, meeting_id)
);
create table vote_rounds (
    id uuid primary key,
    meeting_id uuid not null references vote_meetings(id) on delete cascade,
    title jsonb not null,
    secret text not null,
    max_choises integer not null check (max_choises > 0),
    open bool not null
);
create table vote_alternatives (
    id uuid primary key,
    round_id uuid not null references vote_rounds(id) on delete cascade,
    title jsonb not null,
    voted_for integer not null check (voted_for >= 0)
);
-- client app generates UUID, shows QR with UUID & timestamp (using server time),
-- admin scans, it's added to this table
--
-- We don't know which user owns which client
create table vote_registered_clients (
    client_id uuid,
    meeting_id uuid references vote_meetings (id) on delete cascade,
    primary key (client_id, meeting_id),
    -- a client id is only ever registered for one meeting
    unique (client_id)
);
-- only used when vote_meetings->secure_voting = true
create table vote_cast_votes (
    -- has to be a registered client for this meeting
    client_id uuid references vote_registered_clients(client_id) on delete cascade,
    round_id uuid not null references vote_rounds(id) on delete cascade,
    primary key (client_id, round_id)
);
-- add it to here at same time as registered clients. Makes sure one person votes at most once, 
-- without tying a user to a vote or even if they voted at all; we don't know which user has which 
-- client id (unless we're bad boys and track it in rust)
create table vote_registered_users (
    user_id text references users(id),
    meeting_id uuid references vote_meetings (id) on delete cascade,
    primary key (user_id, meeting_id)
);
