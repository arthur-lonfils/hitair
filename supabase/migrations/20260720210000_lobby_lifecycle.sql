-- Lobby lifecycle: let a client remove or modify its own lobby "ad" (a `parties`
-- row) so empty lobbies disappear from Browse and hosts can change settings.
--
-- Access stays trust-based, matching the existing insert policy: the client uses
-- the publishable key and there is no per-user identity to scope rows to. For an
-- ephemeral music-game lobby that's an acceptable trade-off.

create policy "parties are deletable by anyone"
    on public.parties for delete to anon, authenticated using (true);

create policy "parties are updatable by anyone"
    on public.parties for update to anon, authenticated using (true) with check (true);

grant delete, update on public.parties to anon, authenticated;

-- One-time cleanup: clear accumulated test/placeholder lobby rows (cascades to
-- their scores). New lobbies are inserted on demand and empty ones self-delete.
delete from public.parties;
