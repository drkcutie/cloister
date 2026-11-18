-- Workdays, Workday Start and Shutdown Time, Evening Lockdown and Rest Days, mirroring the Rust
-- rules engine (cloister-core/src/settings.rs) and pinned by the shared fixtures
-- (settings-*, evening-lockdown-*, rest-day, no-deep-session-in-lockdown, day-plan-workday-bounds).
--
-- Settings follow "tighten now, loosen tomorrow" (ADR 0001): the guard trigger keeps the
-- stricter part of a change (Workdays in both, the later Workday Start, the earlier Shutdown
-- Time) in force at once and parks the request in the pending_* columns until the next day
-- boundary. Evening Lockdown runs on the clock: public.enforcement() adds the Work List and the
-- Distraction List from Shutdown Time to the next Workday Start and all day on a Rest Day.
--
-- public.enforcement() is replaced here; it keeps everything from
-- 20261118160000_day_plans_with_blocklists.sql. A later replacement must keep this too.

create table public.settings (
  user_id uuid primary key default auth.uid() references auth.users (id) on delete cascade,
  -- In force as of the last change (ISO weekdays, sorted; local "HH:MM").
  workdays smallint[] not null,
  workday_start text not null,
  shutdown_time text not null,
  -- The requested settings when they loosen something, in force from pending_effective_at.
  pending_workdays smallint[],
  pending_workday_start text,
  pending_shutdown_time text,
  pending_effective_at timestamptz,
  updated_at timestamptz not null default now()
);
alter table public.settings enable row level security;
create policy own_select on public.settings for select to authenticated
  using ((select auth.uid()) = user_id);
revoke insert, update, delete, truncate on public.settings from anon, authenticated;
grant select on public.settings to authenticated;
alter publication supabase_realtime add table public.settings;

-- The settings in force for the user at `t_at` (no row: none).
create function cloister.settings_at(uid uuid, t_at timestamptz)
returns table (workdays smallint[], workday_start text, shutdown_time text)
language sql stable
set search_path = ''
as $$
  select case when s.pending_effective_at <= t_at then s.pending_workdays else s.workdays end,
         case when s.pending_effective_at <= t_at then s.pending_workday_start else s.workday_start end,
         case when s.pending_effective_at <= t_at then s.pending_shutdown_time else s.shutdown_time end
  from public.settings s
  where s.user_id = uid
$$;

-- 'rest_day', 'evening_lockdown', or null when the clock allows work at `t_at` (or no settings).
-- The values are also the rejection codes for a Deep Session start.
create function cloister.lockdown(uid uuid, t_at timestamptz) returns text
language sql stable
set search_path = ''
as $$
  select case
    when not (extract(isodow from l.t)::smallint = any (s.workdays)) then 'rest_day'
    when l.t::time < s.workday_start::time or l.t::time >= s.shutdown_time::time then 'evening_lockdown'
  end
  from cloister.settings_at(uid, t_at) s,
       (select t_at at time zone cloister.user_time_zone(uid) as t) l
$$;

create function cloister.check_settings(days smallint[], workday_start text, shutdown_time text)
returns void
language plpgsql immutable
set search_path = ''
as $$
begin
  if days is null
     or exists (select 1 from unnest(days) d where d is null or d not between 1 and 7)
     or cardinality(days) <> (select count(distinct d) from unnest(days) d)
     or not cloister.is_hhmm(workday_start) or not cloister.is_hhmm(shutdown_time) then
    perform cloister.reject('invalid_settings');
  end if;
  if shutdown_time::time <= workday_start::time then
    perform cloister.reject('invalid_settings');
  end if;
end
$$;

-- Guard: decides on every write, whoever makes it. NEW carries the requested settings.
create function cloister.settings_writing() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
  base record;
  tight_days smallint[];
  tight_start text;
  tight_shutdown text;
begin
  perform cloister.check_settings(new.workdays, new.workday_start, new.shutdown_time);
  new.workdays := array(select distinct d from unnest(new.workdays) d order by d);
  new.updated_at := now;
  if tg_op = 'INSERT' then
    new.pending_workdays := null;
    new.pending_workday_start := null;
    new.pending_shutdown_time := null;
    new.pending_effective_at := null;
    return new;
  end if;
  new.user_id := old.user_id;
  if old.pending_effective_at is not null and old.pending_effective_at <= now then
    select old.pending_workdays as workdays, old.pending_workday_start as workday_start,
           old.pending_shutdown_time as shutdown_time into base;
  else
    select old.workdays as workdays, old.workday_start as workday_start,
           old.shutdown_time as shutdown_time into base;
  end if;
  tight_days := array(select d from unnest(base.workdays) d where d = any (new.workdays) order by d);
  tight_start := greatest(base.workday_start collate "C", new.workday_start collate "C");
  tight_shutdown := least(base.shutdown_time collate "C", new.shutdown_time collate "C");
  if tight_days = new.workdays and tight_start = new.workday_start
     and tight_shutdown = new.shutdown_time then
    new.pending_workdays := null;
    new.pending_workday_start := null;
    new.pending_shutdown_time := null;
    new.pending_effective_at := null;
  else
    new.pending_workdays := new.workdays;
    new.pending_workday_start := new.workday_start;
    new.pending_shutdown_time := new.shutdown_time;
    new.pending_effective_at := cloister.next_day_boundary(cloister.user_time_zone(new.user_id), now);
    new.workdays := tight_days;
    new.workday_start := tight_start;
    new.shutdown_time := tight_shutdown;
  end if;
  return new;
end
$$;
create trigger settings_writing before insert or update on public.settings
  for each row execute function cloister.settings_writing();

create function cloister.settings_deleting() returns trigger
language plpgsql
set search_path = ''
as $$
begin
  if cloister.account_gone(old.user_id) then
    return old;
  end if;
  perform cloister.reject('invalid_settings');
  return null;
end
$$;
create trigger settings_deleting before delete on public.settings
  for each row execute function cloister.settings_deleting();

-- change_settings (dispatched by public.command). Engine order: time zone known, shape, then
-- a time zone change (only while no settings exist).
create function cloister.command_change_settings(uid uuid, cmd jsonb) returns void
language plpgsql
set search_path = ''
as $$
declare
  tz text;
  days smallint[];
begin
  perform cloister.check_keys(cmd, array['workdays', 'workday_start', 'shutdown_time'], array['time_zone']);
  if jsonb_typeof(cmd->'workdays') <> 'array'
     or exists (select 1 from jsonb_array_elements(cmd->'workdays') d where jsonb_typeof(d) <> 'number')
     or jsonb_typeof(cmd->'workday_start') <> 'string' or jsonb_typeof(cmd->'shutdown_time') <> 'string'
     or jsonb_typeof(coalesce(cmd->'time_zone', 'null')) not in ('string', 'null') then
    perform cloister.reject('invalid_command');
  end if;
  tz := cmd->>'time_zone';
  if tz is not null and not exists (select 1 from pg_catalog.pg_timezone_names where name = tz) then
    perform cloister.reject('unknown_time_zone');
  end if;
  -- Clamped so any out-of-range number reads as out of range, not as an overflow.
  days := array(select least(greatest(d::numeric, -1), 99)::smallint
                from jsonb_array_elements_text(cmd->'workdays') d);
  perform cloister.check_settings(days, cmd->>'workday_start', cmd->>'shutdown_time');
  if tz is not null and tz <> cloister.user_time_zone(uid) then
    if exists (select 1 from public.settings s where s.user_id = uid) then
      perform cloister.reject('time_zone_cannot_change');
    end if;
    update public.accounts a set time_zone = tz where a.user_id = uid;
  end if;
  if exists (select 1 from public.settings s where s.user_id = uid) then
    update public.settings s
    set workdays = days, workday_start = cmd->>'workday_start', shutdown_time = cmd->>'shutdown_time'
    where s.user_id = uid;
  else
    insert into public.settings (user_id, workdays, workday_start, shutdown_time)
    values (uid, days, cmd->>'workday_start', cmd->>'shutdown_time');
  end if;
end
$$;

-- The settings part of public.state() (omitted until set, as the engine does).
create function cloister.state_part_settings(uid uuid) returns jsonb
language sql stable
set search_path = ''
as $$
  select coalesce((
    select jsonb_build_object('settings', jsonb_build_object(
             'workdays', to_jsonb(s.workdays),
             'workday_start', s.workday_start,
             'shutdown_time', s.shutdown_time))
        || case when s.pending_effective_at is null then '{}'::jsonb
                else jsonb_build_object('pending_settings', jsonb_build_object(
                       'workdays', to_jsonb(s.pending_workdays),
                       'workday_start', s.pending_workday_start,
                       'shutdown_time', s.pending_shutdown_time,
                       'effective_at', cloister.rfc3339(s.pending_effective_at))) end
    from public.settings s where s.user_id = uid), '{}'::jsonb)
$$;

-- No Deep Session starts during Evening Lockdown or on a Rest Day. Fires after block_inserting
-- and block_slot_id (triggers run in name order), so duplicate_block comes first, as in the engine.
create function cloister.block_starting_deep_session() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  why text;
begin
  if new.deep and not new.quit_social then
    why := cloister.lockdown(new.user_id, cloister.server_now());
    if why is not null then
      perform cloister.reject(why);
    end if;
  end if;
  return new;
end
$$;
create trigger block_starting_deep_session before insert on public.blocks
  for each row execute function cloister.block_starting_deep_session();

-- With settings, a Day Plan runs from its date's Workday Start to its Shutdown Time (settings in
-- force at the later of now and the date's local midnight), and a Rest Day has none. Fires before
-- day_plan_writing, so these checks come first, as in the engine. Recording a Depth Rating
-- (only ratings change) isn't a write of the plan and isn't checked.
create function cloister.slots_without_ratings(slots jsonb) returns jsonb
language sql immutable
set search_path = ''
as $$
  select coalesce(jsonb_agg(x - 'depth_rating' order by o), '[]'::jsonb)
  from jsonb_array_elements(slots) with ordinality as t(x, o)
$$;

create function cloister.day_plan_workday() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  tz text := cloister.user_time_zone(new.user_id);
  s record;
begin
  if tg_op = 'UPDATE' and new.starts_at = old.starts_at and new.ends_at = old.ends_at
     and jsonb_typeof(new.slots) = 'array'
     and cloister.slots_without_ratings(new.slots) = cloister.slots_without_ratings(old.slots) then
    return new;
  end if;
  select * into s
  from cloister.settings_at(new.user_id, greatest(cloister.server_now(), new.date::timestamp at time zone tz));
  if not found then
    return new;
  end if;
  if not (extract(isodow from new.date)::smallint = any (s.workdays)) then
    perform cloister.reject('rest_day');
  end if;
  if new.starts_at <> (new.date + s.workday_start::time) at time zone tz
     or new.ends_at <> (new.date + s.shutdown_time::time) at time zone tz then
    perform cloister.reject('invalid_day_plan');
  end if;
  return new;
end
$$;
create trigger day_plan_workday before insert or update on public.day_plans
  for each row execute function cloister.day_plan_workday();

-- Replaces the day_plans_with_blocklists version: the same, plus Evening Lockdown and Rest Days,
-- which enforce the Work List and the Distraction List categories in force.
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
    'ritual', 'none')
$$;
