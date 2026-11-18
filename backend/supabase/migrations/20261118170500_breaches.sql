-- Breaches: a Device's enforcement was disabled during a Block (Android: the VPN or the app
-- blocker turned off). Recorded at server time and shown in the Weekly Review; nothing else
-- follows from one. Written only through public.command({"type": "record_breach", ...}).

create table public.breaches (
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  id text not null check (length(id) between 1 and 64),
  kind text not null check (kind in ('vpn_turned_off', 'app_blocker_turned_off')),
  occurred_at timestamptz not null,
  -- The Device's own clock, for diagnosis only.
  client_time timestamptz,
  primary key (user_id, id)
);
alter table public.breaches enable row level security;
create policy own_select on public.breaches for select to authenticated
  using ((select auth.uid()) = user_id);
revoke insert, update, delete, truncate on public.breaches from anon, authenticated;
grant select on public.breaches to authenticated;
alter publication supabase_realtime add table public.breaches;

-- record_breach {breach_id, kind, client_time?}: idempotent by id, so a Device can resend it.
create function cloister.command_record_breach(uid uuid, cmd jsonb) returns void
language plpgsql
set search_path = ''
as $$
begin
  perform cloister.check_keys(cmd, array['breach_id', 'kind'], array['client_time']);
  if jsonb_typeof(cmd->'breach_id') <> 'string'
     or jsonb_typeof(cmd->'kind') <> 'string'
     or cmd->>'kind' not in ('vpn_turned_off', 'app_blocker_turned_off')
     or jsonb_typeof(coalesce(cmd->'client_time', 'null')) not in ('string', 'null') then
    perform cloister.reject('invalid_command');
  end if;
  insert into public.breaches (user_id, id, kind, occurred_at, client_time)
  values (uid, cmd->>'breach_id', cmd->>'kind', cloister.server_now(),
          (cmd->>'client_time')::timestamptz)
  on conflict (user_id, id) do nothing;
end
$$;

create function cloister.state_part_breaches(uid uuid) returns jsonb
language sql stable
set search_path = ''
as $$
  select jsonb_strip_nulls(jsonb_build_object(
    'breaches', (
      select jsonb_agg(jsonb_build_object('id', id, 'kind', kind,
                                          'occurred_at', cloister.rfc3339(occurred_at))
                       order by occurred_at, id collate "C")
      from public.breaches where user_id = uid)))
$$;
