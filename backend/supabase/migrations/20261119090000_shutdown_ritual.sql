-- The Shutdown Ritual, Shutdown Delays, Missed Rituals and Loose Ends, mirroring the Rust rules
-- engine (cloister-core/src/ritual.rs) and pinned by the shared fixtures (ritual-*,
-- shutdown-delay-*, missed-ritual-*, loose-ends). See fixtures/README.md, "Shutdown Ritual".
--
-- Only completed rituals are stored; a Missed Ritual is a Workday whose ritual lapsed (at the
-- next Workday Start) with no completion. One Shutdown Delay per ISO week (of the local date) is
-- a unique index, so two Devices can't both use one.
--
-- Replaces cloister.lockdown() (a Shutdown Delay holds back Evening Lockdown for 30 minutes)
-- and public.enforcement() (keeps everything from 20261118180000_settings_evening_lockdown.sql
-- and reports the ritual screen). A later replacement must keep both.

create table public.loose_ends (
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  id text not null check (length(id) between 1 and 64),
  text text not null,
  captured_at timestamptz not null,
  primary key (user_id, id)
);

create table public.rituals (
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  date date not null,
  completed_at timestamptz not null,
  primary key (user_id, date)
);

create table public.shutdown_delays (
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  date date not null,
  used_at timestamptz not null,
  iso_week int generated always as
    ((extract(isoyear from date) * 100 + extract(week from date))::int) stored,
  primary key (user_id, date),
  unique (user_id, iso_week)
);

alter table public.loose_ends enable row level security;
alter table public.rituals enable row level security;
alter table public.shutdown_delays enable row level security;
create policy own_select on public.loose_ends for select to authenticated
  using ((select auth.uid()) = user_id);
create policy own_select on public.rituals for select to authenticated
  using ((select auth.uid()) = user_id);
create policy own_select on public.shutdown_delays for select to authenticated
  using ((select auth.uid()) = user_id);
revoke insert, update, delete, truncate on public.loose_ends, public.rituals, public.shutdown_delays
  from anon, authenticated;
grant select on public.loose_ends, public.rituals, public.shutdown_delays to authenticated;
alter publication supabase_realtime add table public.loose_ends, public.rituals, public.shutdown_delays;

-- Records can't be rewritten or removed (except with the account).
create function cloister.ritual_record_fixed() returns trigger
language plpgsql
set search_path = ''
as $$
begin
  if tg_op = 'DELETE' and cloister.account_gone(old.user_id) then
    return old;
  end if;
  perform cloister.reject('invalid_command');
  return null;
end
$$;
create trigger ritual_record_fixed before update or delete on public.rituals
  for each row execute function cloister.ritual_record_fixed();
create trigger shutdown_delay_fixed before update or delete on public.shutdown_delays
  for each row execute function cloister.ritual_record_fixed();
create trigger loose_end_fixed before update on public.loose_ends
  for each row execute function cloister.ritual_record_fixed();

-- The UTC Shutdown Time of local date `d` if it is a Workday, under the settings in force at
-- the later of `t_at` and the date's local midnight (as the engine's workday()).
create function cloister.workday_shutdown(uid uuid, d date, t_at timestamptz) returns timestamptz
language sql stable
set search_path = ''
as $$
  select (d + s.shutdown_time::time) at time zone cloister.user_time_zone(uid)
  from cloister.settings_at(uid, greatest(t_at, d::timestamp at time zone cloister.user_time_zone(uid))) s
  where extract(isodow from d)::smallint = any (s.workdays)
$$;

-- When the ritual of `d` becomes due: Shutdown Time, 30 minutes later with a Shutdown Delay.
create function cloister.ritual_due_at(uid uuid, d date, t_at timestamptz) returns timestamptz
language sql stable
set search_path = ''
as $$
  select cloister.workday_shutdown(uid, d, t_at)
         + case when exists (select 1 from public.shutdown_delays x where x.user_id = uid and x.date = d)
                then interval '30 minutes' else interval '0' end
$$;

-- The first Workday Start after `t_at` within two weeks (engine: next_workday_start).
create function cloister.next_workday_start(uid uuid, t_at timestamptz) returns timestamptz
language sql stable
set search_path = ''
as $$
  select (d + s.workday_start::time) at time zone tz
  from (select cloister.user_time_zone(uid) as tz) z,
       generate_series(0, 14) n,
       lateral (select ((t_at at time zone z.tz)::date + n) as d) dd,
       lateral cloister.settings_at(uid, greatest(t_at, dd.d::timestamp at time zone z.tz)) s
  where extract(isodow from dd.d)::smallint = any (s.workdays)
    and s.workday_start::time < s.shutdown_time::time
    and (dd.d + s.workday_start::time) at time zone z.tz > t_at
  order by n
  limit 1
$$;

-- The latest ritual due at or before `t_at` (looking back 15 days).
create function cloister.ritual(uid uuid, t_at timestamptz)
returns table (date date, due_at timestamptz, lapses_at timestamptz, completed_at timestamptz)
language sql stable
set search_path = ''
as $$
  select r.d, r.due, cloister.next_workday_start(uid, r.due),
         (select x.completed_at from public.rituals x where x.user_id = uid and x.date = r.d)
  from (
    select dd.d, cloister.ritual_due_at(uid, dd.d, t_at) as due, n
    from generate_series(0, 14) n,
         lateral (select ((t_at at time zone cloister.user_time_zone(uid))::date - n) as d) dd
  ) r
  where r.due <= t_at
  order by r.n
  limit 1
$$;

-- The ritual screen at `t_at`: none, delayed, due, completed or missed.
create function cloister.ritual_screen(uid uuid, t_at timestamptz) returns text
language plpgsql stable
set search_path = ''
as $$
declare
  today date := (t_at at time zone cloister.user_time_zone(uid))::date;
  r record;
  lapsed boolean;
begin
  if exists (select 1 from public.shutdown_delays x
             where x.user_id = uid and x.date = today and x.used_at <= t_at)
     and t_at < cloister.ritual_due_at(uid, today, t_at) then
    return 'delayed';
  end if;
  select * into r from cloister.ritual(uid, t_at);
  if not found then
    return 'none';
  end if;
  lapsed := r.lapses_at is not null and r.lapses_at <= t_at;
  return case
    when r.completed_at is not null and not lapsed then 'completed'
    when r.completed_at is not null then 'none'
    when not lapsed then 'due'
    else 'missed'
  end;
end
$$;

-- Replaces the settings_evening_lockdown version: a Shutdown Delay for the local date holds
-- Evening Lockdown back until 30 minutes after Shutdown Time (local wall time).
create or replace function cloister.lockdown(uid uuid, t_at timestamptz) returns text
language sql stable
set search_path = ''
as $$
  select case
    when not (extract(isodow from l.t)::smallint = any (s.workdays)) then 'rest_day'
    when l.t::time < s.workday_start::time then 'evening_lockdown'
    when l.t::time >= s.shutdown_time::time
         and not exists (select 1 from public.shutdown_delays x
                         where x.user_id = uid and x.date = l.t::date
                           and l.t < l.t::date + s.shutdown_time::time + interval '30 minutes')
      then 'evening_lockdown'
  end
  from cloister.settings_at(uid, t_at) s,
       (select t_at at time zone cloister.user_time_zone(uid) as t) l
$$;

-- Guards: every write goes through these, whoever makes it.
create function cloister.loose_end_inserting() returns trigger
language plpgsql
set search_path = ''
as $$
begin
  new.text := btrim(new.text, E' \t\n\r');
  if new.text = '' or char_length(new.text) > 500 or btrim(new.id) = '' then
    perform cloister.reject('invalid_loose_end');
  end if;
  if exists (select 1 from public.loose_ends l where l.user_id = new.user_id and l.id = new.id) then
    perform cloister.reject('duplicate_loose_end');
  end if;
  new.captured_at := cloister.server_now();
  return new;
end
$$;
create trigger loose_end_inserting before insert on public.loose_ends
  for each row execute function cloister.loose_end_inserting();

create function cloister.shutdown_delay_inserting() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
  why text;
begin
  if not exists (select 1 from public.settings s where s.user_id = new.user_id) then
    perform cloister.reject('no_shutdown_time');
  end if;
  why := cloister.lockdown(new.user_id, now);
  if why is not null then
    perform cloister.reject(why);
  end if;
  new.date := (now at time zone cloister.user_time_zone(new.user_id))::date;
  new.used_at := now;
  if exists (select 1 from public.shutdown_delays x
             where x.user_id = new.user_id
               and x.iso_week = (extract(isoyear from new.date) * 100 + extract(week from new.date))::int) then
    perform cloister.reject('shutdown_delay_used');
  end if;
  return new;
end
$$;
create trigger shutdown_delay_inserting before insert on public.shutdown_delays
  for each row execute function cloister.shutdown_delay_inserting();

-- Completing the ritual. `typed` travels in a transaction-local setting, since it isn't stored.
create function cloister.ritual_inserting() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
  r record;
  plan public.day_plans;
  tomorrow date;
  reached timestamptz;
  slot record;
begin
  select * into r from cloister.ritual(new.user_id, now);
  if not found then
    perform cloister.reject('ritual_not_due');
  end if;
  if r.completed_at is not null then
    perform cloister.reject('ritual_already_completed');
  end if;
  if r.lapses_at is not null and r.lapses_at <= now then
    perform cloister.reject('ritual_not_due');
  end if;
  if not exists (select 1 from public.loose_ends l
                 where l.user_id = new.user_id and l.captured_at >= r.due_at) then
    perform cloister.reject('loose_end_required');
  end if;
  if r.lapses_at is not null then
    tomorrow := (r.lapses_at at time zone cloister.user_time_zone(new.user_id))::date;
    select * into plan from public.day_plans p where p.user_id = new.user_id and p.date = tomorrow;
    if not found then
      perform cloister.reject('day_plan_required');
    end if;
    if not exists (select 1 from cloister.slots_of(plan.slots) s where s.deep) then
      perform cloister.reject('deep_slot_required');
    end if;
    reached := plan.starts_at;
    for slot in select * from cloister.slots_of(plan.slots) s order by s.starts_at, s.ord loop
      if slot.starts_at <> reached then
        perform cloister.reject('workday_not_covered');
      end if;
      reached := slot.ends_at;
    end loop;
    if reached <> plan.ends_at then
      perform cloister.reject('workday_not_covered');
    end if;
  end if;
  if coalesce(current_setting('cloister.typed', true), '') <> 'Shutdown complete' then
    perform cloister.reject('confirmation_mismatch');
  end if;
  new.date := r.date;
  new.completed_at := now;
  return new;
end
$$;
create trigger ritual_inserting before insert on public.rituals
  for each row execute function cloister.ritual_inserting();

-- Commands (dispatched by public.command).
create function cloister.command_capture_loose_end(uid uuid, cmd jsonb) returns void
language plpgsql
set search_path = ''
as $$
begin
  perform cloister.check_keys(cmd, array['loose_end_id', 'text'], array[]::text[]);
  if jsonb_typeof(cmd->'loose_end_id') <> 'string' or jsonb_typeof(cmd->'text') <> 'string' then
    perform cloister.reject('invalid_command');
  end if;
  if length(cmd->>'loose_end_id') > 64 then
    perform cloister.reject('invalid_loose_end');
  end if;
  insert into public.loose_ends (user_id, id, text, captured_at)
  values (uid, cmd->>'loose_end_id', cmd->>'text', cloister.server_now());
end
$$;

create function cloister.command_close_loose_end(uid uuid, cmd jsonb) returns void
language plpgsql
set search_path = ''
as $$
begin
  perform cloister.check_keys(cmd, array['loose_end_id'], array[]::text[]);
  delete from public.loose_ends l where l.user_id = uid and l.id = cmd->>'loose_end_id';
  if not found then
    perform cloister.reject('unknown_loose_end');
  end if;
end
$$;

create function cloister.command_use_shutdown_delay(uid uuid, cmd jsonb) returns void
language plpgsql
set search_path = ''
as $$
begin
  perform cloister.check_keys(cmd, array[]::text[], array[]::text[]);
  -- date and used_at are set by the guard.
  insert into public.shutdown_delays (user_id, date, used_at)
  values (uid, current_date, cloister.server_now());
exception
  when unique_violation then
    perform cloister.reject('shutdown_delay_used');
end
$$;

create function cloister.command_complete_ritual(uid uuid, cmd jsonb) returns void
language plpgsql
set search_path = ''
as $$
begin
  perform cloister.check_keys(cmd, array['typed'], array[]::text[]);
  if jsonb_typeof(cmd->'typed') <> 'string' then
    perform cloister.reject('invalid_command');
  end if;
  perform set_config('cloister.typed', cmd->>'typed', true);
  -- date and completed_at are set by the guard.
  insert into public.rituals (user_id, date, completed_at)
  values (uid, current_date, cloister.server_now());
  perform set_config('cloister.typed', '', true);
end
$$;

-- The ritual part of public.state() (each key omitted when empty, as the engine does).
create function cloister.state_part_rituals(uid uuid) returns jsonb
language sql stable
set search_path = ''
as $$
  select jsonb_strip_nulls(jsonb_build_object(
    'loose_ends', (select jsonb_agg(jsonb_build_object(
                     'id', l.id, 'text', l.text, 'captured_at', cloister.rfc3339(l.captured_at))
                     order by l.captured_at, l.id collate "C")
                   from public.loose_ends l where l.user_id = uid),
    'rituals', (select jsonb_agg(jsonb_build_object(
                  'date', r.date, 'completed_at', cloister.rfc3339(r.completed_at)) order by r.date)
                from public.rituals r where r.user_id = uid),
    'shutdown_delays', (select jsonb_agg(jsonb_build_object(
                          'date', x.date, 'used_at', cloister.rfc3339(x.used_at)) order by x.date)
                        from public.shutdown_delays x where x.user_id = uid)))
$$;

-- Replaces the settings_evening_lockdown version: the same, plus the ritual screen.
create or replace function public.enforcement() returns jsonb
language sql stable
set search_path = ''
as $$
  with me as (
    select (select auth.uid()) as uid, cloister.server_now() as now
  ), locked as (
    select 1 from me where cloister.lockdown(me.uid, me.now) is not null
  ), active as (
    select b.* from public.blocks b, me
    where b.user_id = me.uid and b.starts_at <= me.now and me.now < b.ends_at
  ), slot_starts as (
    select cloister.running_deep_slot_starts(me.uid, me.now) as starts_at from me
  ), lists as (
    select unnest(blocklists) as list from active
    union
    select unnest(cloister.distraction_ids_in_force(me.uid, me.now)) from me
    where exists (select 1 from active where not quit_social)
       or exists (select 1 from slot_starts)
       or exists (select 1 from locked)
    union
    select unnest(cloister.distraction_ids_in_force(me.uid, s.starts_at)) from me, slot_starts s
    union
    select c.id from public.blocklists c where c.kind = 'work' and exists (select 1 from locked)
  ), listed as (
    select l.list, 'domain' as kind, unnest(c.domains) as entry
    from lists l join public.blocklists c on c.id = l.list
    union
    select l.list, 'app', unnest(c.apps) from lists l join public.blocklists c on c.id = l.list
    union
    select e.blocklist, e.kind, e.entry
    from public.blocklist_entries e join lists l on l.list = e.blocklist, me
    where e.user_id = me.uid
  ), effective as (
    select x.* from listed x, me
    where not exists (select 1 from public.blocklist_removals r
                      where r.user_id = me.uid and r.blocklist = x.list and r.kind = x.kind
                        and r.entry = x.entry and r.effective_at <= me.now)
  ), domains as (
    select entry as domain from effective where kind = 'domain'
    union
    select unnest(extra_domains) from active
  ), all_domains as (
    select domain from domains
    union
    select unnest(r.related) from public.related_domains r join domains d using (domain)
  ), apps as (
    select entry as app from effective where kind = 'app'
    union
    select unnest(extra_apps) from active
  )
  select jsonb_build_object(
    'blocked_domains', coalesce((select jsonb_agg(domain order by domain collate "C") from all_domains), '[]'::jsonb),
    'blocked_apps', coalesce((select jsonb_agg(app order by app collate "C") from apps), '[]'::jsonb),
    'ritual', (select cloister.ritual_screen(me.uid, me.now) from me))
$$;
