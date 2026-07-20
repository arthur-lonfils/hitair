-- hitair — online "Challenge" / party mode schema.
--
-- Designed for the *publishable* (anon) key embedded in the client: security is
-- Row-Level Security, not key secrecy. Anyone may create a party and submit a
-- score; nobody may edit or delete rows. Apply this once (Supabase SQL editor
-- or the Supabase connector).

create table if not exists public.parties (
    code        text primary key,             -- short shareable code, e.g. "7Q2F9K"
    visibility  text        not null default 'private'
                  check (visibility in ('public', 'private')),
    max_players integer     not null default 8 check (max_players between 1 and 64),
    track_id    bigint      not null,          -- Deezer track id (defines the song)
    title       text        not null,
    artist      text        not null,
    album       text,
    schedule    integer[]   not null,          -- clip lengths in milliseconds, ascending
    host_name   text        not null default 'host',
    created_at  timestamptz not null default now()
);

create index if not exists parties_public_recent
    on public.parties (created_at desc) where visibility = 'public';

create table if not exists public.scores (
    id          bigint generated always as identity primary key,
    party_code  text        not null references public.parties(code) on delete cascade,
    player_name text        not null default 'player',
    solved      boolean     not null,
    clips_used  integer     not null,          -- levels revealed before the correct guess
    time_ms     integer     not null,          -- ms from first play to solve/give-up
    mistakes    integer     not null,
    created_at  timestamptz not null default now()
);

-- Leaderboard order: solved first, then fewest clips, then fastest.
create index if not exists scores_leaderboard
    on public.scores (party_code, solved desc, clips_used, time_ms);

-- --- Row-Level Security --------------------------------------------------

alter table public.parties enable row level security;
alter table public.scores  enable row level security;

create policy "parties are insertable by anyone"
    on public.parties for insert to anon, authenticated with check (true);
create policy "parties are readable by anyone"
    on public.parties for select to anon, authenticated using (true);
-- Lobbies are ephemeral: the client deletes its ad when the lobby empties and
-- updates it when the host changes settings. Trust-based like the insert policy.
create policy "parties are deletable by anyone"
    on public.parties for delete to anon, authenticated using (true);
create policy "parties are updatable by anyone"
    on public.parties for update to anon, authenticated using (true) with check (true);

create policy "scores are insertable by anyone"
    on public.scores for insert to anon, authenticated with check (true);
create policy "scores are readable by anyone"
    on public.scores for select to anon, authenticated using (true);

grant select, insert, update, delete on public.parties to anon, authenticated;
grant select, insert on public.scores  to anon, authenticated;
grant usage, select on all sequences in schema public to anon, authenticated;
