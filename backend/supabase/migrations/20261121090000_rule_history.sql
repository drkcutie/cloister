-- Three rule changes, mirroring cloister-core (commit 278286c) and pinned by the shared fixtures
-- schedule-ratchet, schedule-change-before-the-day-starts, workday-removed-keeps-its-ritual,
-- settings-tightest-stays-a-workday and the past_settings in the settings-* fixtures.
-- See fixtures/README.md ("The ratchet", put_schedule, "Settings and Evening Lockdown",
-- "Which settings a date uses").
--
-- S3: a day's Schedule plan is in force (the ratchet's floor) from the date's local midnight in
--     the Schedule's time zone, and put_schedule keeps today's plan before it starts when the
--     change would cut today's Planned Deep Minutes.
-- S4: settings history (public.settings.past_settings). A date that has begun keeps the Workday
--     it was at its local midnight; today takes the settings in force now if they keep it a Workday.
-- S5: the stricter of two settings keeps the times in force when the stricter times would leave
--     no working time (Workday Start at or after Shutdown Time).
--
-- Doesn't touch public.command, public.state or public.enforcement. Replaces (a later
-- replacement must keep their behaviour here):
--   cloister.settings_at, cloister.settings_writing (20261118180000), cloister.day_plan_workday
--   (20261118180000), cloister.workday_shutdown and cloister.next_workday_start (20261119090000),
--   cloister.shutdown_delay_inserting (20261120101000), cloister.day_plan_writing and
--   cloister.schedule_writing (20261118072046).

-- S4: settings history -------------------------------------------------------------------------
-- Each entry {workdays, workday_start, shutdown_time, until} was in force from the previous
-- entry's `until` (or always, for the first) until its own. Sorted by `until`. Written only by
-- the settings_writing guard.
alter table public.settings add column past_settings jsonb not null default '[]';

-- The settings in force for the user at `t_at` (no row: none): the first history entry ending
-- after `t_at`, else the pending ones once their day has come, else the stored ones.
create or replace function cloister.settings_at(uid uuid, t_at timestamptz)
returns table (workdays smallint[], workday_start text, shutdown_time text)
language sql stable
set search_path = ''
as $$
  select case when h.x is not null
                then array(select d::smallint from jsonb_array_elements_text(h.x->'workdays') d)
              when s.pending_effective_at <= t_at then s.pending_workdays
              else s.workdays end,
         case when h.x is not null then h.x->>'workday_start'
              when s.pending_effective_at <= t_at then s.pending_workday_start
              else s.workday_start end,
         case when h.x is not null then h.x->>'shutdown_time'
              when s.pending_effective_at <= t_at then s.pending_shutdown_time
              else s.shutdown_time end
  from public.settings s
  left join lateral (
    select e.x from jsonb_array_elements(s.past_settings) with ordinality as e(x, o)
    where (e.x->>'until')::timestamptz > t_at
    order by e.o
    limit 1
  ) h on true
  where s.user_id = uid
$$;

-- `past` with the given settings appended as in force until `until`, unless the last entry
-- already ends at or after it.
create function cloister.past_settings_push(past jsonb, days smallint[], workday_start text,
                                            shutdown_time text, until timestamptz) returns jsonb
language sql immutable
set search_path = ''
as $$
  select case
    when jsonb_array_length(past) > 0 and (past->-1->>'until')::timestamptz >= until then past
    else past || jsonb_build_array(jsonb_build_object(
           'workdays', to_jsonb(days), 'workday_start', workday_start,
           'shutdown_time', shutdown_time, 'until', cloister.rfc3339(until)))
  end
$$;

-- Replaces the settings_evening_lockdown version: the stricter times only apply when they leave
-- working time (S5), and the settings being replaced go into the history (S4).
create or replace function cloister.settings_writing() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
  base record;
  tight_days smallint[];
  tight_start text;
  tight_shutdown text;
  past jsonb;
begin
  perform cloister.check_settings(new.workdays, new.workday_start, new.shutdown_time);
  new.workdays := array(select distinct d from unnest(new.workdays) d order by d);
  new.updated_at := now;
  if tg_op = 'INSERT' then
    new.pending_workdays := null;
    new.pending_workday_start := null;
    new.pending_shutdown_time := null;
    new.pending_effective_at := null;
    new.past_settings := '[]';
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
  if tight_start collate "C" >= tight_shutdown collate "C" then
    tight_start := base.workday_start;
    tight_shutdown := base.shutdown_time;
  end if;

  -- History: the stored settings until pending ones took over, then the settings in force
  -- until now (when the change replaces them).
  past := old.past_settings;
  if old.pending_effective_at is not null and old.pending_effective_at <= now
     and (old.workdays, old.workday_start, old.shutdown_time) is distinct from
         (old.pending_workdays, old.pending_workday_start, old.pending_shutdown_time) then
    past := cloister.past_settings_push(past, old.workdays, old.workday_start, old.shutdown_time,
                                        old.pending_effective_at);
  end if;
  if (base.workdays, base.workday_start, base.shutdown_time) is distinct from
     (tight_days, tight_start, tight_shutdown) then
    past := cloister.past_settings_push(past, base.workdays, base.workday_start, base.shutdown_time, now);
  end if;
  new.past_settings := past;

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

-- The past_settings part of public.state() (omitted when empty, as the engine does).
create function cloister.state_part_past_settings(uid uuid) returns jsonb
language sql stable
set search_path = ''
as $$
  select coalesce((
    select jsonb_build_object('past_settings', jsonb_agg(jsonb_build_object(
             'workdays', e.x->'workdays',
             'workday_start', e.x->>'workday_start',
             'shutdown_time', e.x->>'shutdown_time',
             'until', cloister.rfc3339((e.x->>'until')::timestamptz)) order by e.o))
    from public.settings s, jsonb_array_elements(s.past_settings) with ordinality as e(x, o)
    where s.user_id = uid
    having count(*) > 0), '{}'::jsonb)
$$;

-- The settings local date `d` uses, as seen at `t_at` (engine: workday()). A date still to come
-- uses the settings in force at its local midnight; a date that has begun uses the ones in force
-- at its local midnight (the history), except that today uses the settings in force now when
-- they keep it a Workday. No row: no settings.
create function cloister.workday_settings(uid uuid, d date, t_at timestamptz)
returns table (workdays smallint[], workday_start text, shutdown_time text)
language plpgsql stable
set search_path = ''
as $$
declare
  tz text := cloister.user_time_zone(uid);
  begins timestamptz := d::timestamp at time zone tz;
  cur record;
  past record;
begin
  select * into cur from cloister.settings_at(uid, greatest(t_at, begins));
  if not found then
    return;
  end if;
  if begins > t_at
     or ((t_at at time zone tz)::date = d and extract(isodow from d)::smallint = any (cur.workdays)) then
    return query select cur.workdays, cur.workday_start, cur.shutdown_time;
    return;
  end if;
  select * into past from cloister.settings_at(uid, begins);
  return query select past.workdays, past.workday_start, past.shutdown_time;
end
$$;

-- Replaces the shutdown_ritual version: the settings the date uses (cloister.workday_settings).
create or replace function cloister.workday_shutdown(uid uuid, d date, t_at timestamptz) returns timestamptz
language sql stable
set search_path = ''
as $$
  select (d + s.shutdown_time::time) at time zone cloister.user_time_zone(uid)
  from cloister.workday_settings(uid, d, t_at) s
  where extract(isodow from d)::smallint = any (s.workdays)
$$;

-- Replaces the shutdown_ritual version: the settings each date uses (cloister.workday_settings).
create or replace function cloister.next_workday_start(uid uuid, t_at timestamptz) returns timestamptz
language sql stable
set search_path = ''
as $$
  select (d + s.workday_start::time) at time zone tz
  from (select cloister.user_time_zone(uid) as tz) z,
       generate_series(0, 14) n,
       lateral (select ((t_at at time zone z.tz)::date + n) as d) dd,
       lateral cloister.workday_settings(uid, dd.d, t_at) s
  where extract(isodow from dd.d)::smallint = any (s.workdays)
    and s.workday_start::time < s.shutdown_time::time
    and (dd.d + s.workday_start::time) at time zone z.tz > t_at
  order by n
  limit 1
$$;

-- Replaces the settings_evening_lockdown version: the bounds come from the settings the date
-- uses. A plan put_schedule keeps for today (cloister.keeping_schedule_plan) is saved as it is,
-- as in the engine.
create or replace function cloister.day_plan_workday() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  tz text := cloister.user_time_zone(new.user_id);
  s record;
begin
  if coalesce(current_setting('cloister.keeping_schedule_plan', true), '') = 'on' then
    return new;
  end if;
  if tg_op = 'UPDATE' and new.starts_at = old.starts_at and new.ends_at = old.ends_at
     and jsonb_typeof(new.slots) = 'array'
     and cloister.slots_without_ratings(new.slots) = cloister.slots_without_ratings(old.slots) then
    return new;
  end if;
  select * into s from cloister.workday_settings(new.user_id, new.date, cloister.server_now());
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

-- Replaces the weekly_review version: the window comes from the settings today uses
-- (cloister.workday_settings); otherwise the same.
create or replace function cloister.shutdown_delay_inserting() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
  tz text := cloister.user_time_zone(new.user_id);
  today date := (now at time zone tz)::date;
  s record;
begin
  if not exists (select 1 from public.settings x where x.user_id = new.user_id) then
    perform cloister.reject('no_shutdown_time');
  end if;
  select * into s from cloister.workday_settings(new.user_id, today, now);
  if not (extract(isodow from today)::smallint = any (s.workdays)) then
    perform cloister.reject('rest_day');
  end if;
  if now < (today + s.workday_start::time) at time zone tz
     or now >= ((today + s.shutdown_time::time) at time zone tz) + interval '30 minutes' then
    perform cloister.reject('evening_lockdown');
  end if;
  if exists (select 1 from public.rituals r where r.user_id = new.user_id and r.date = today) then
    perform cloister.reject('ritual_already_completed');
  end if;
  new.date := today;
  new.used_at := now;
  if exists (select 1 from public.shutdown_delays x
             where x.user_id = new.user_id
               and x.iso_week = (extract(isoyear from new.date) * 100 + extract(week from new.date))::int) then
    perform cloister.reject('shutdown_delay_used');
  end if;
  return new;
end
$$;

-- S3: the plan in force ------------------------------------------------------------------------
-- The Schedule's plan for date `d` once `d` has begun (local midnight) in that Schedule's time
-- zone; null before that or when no Schedule covers its weekday.
create function cloister.schedule_plan_in_force(uid uuid, d date, t_at timestamptz) returns jsonb
language sql stable
set search_path = ''
as $$
  select cloister.schedule_plan_of(s, d)
  from public.schedules s
  where s.user_id = uid and extract(isodow from d)::smallint = any (s.weekdays)
    and (t_at at time zone s.time_zone)::date >= d
  order by s.id
  limit 1
$$;

-- Replaces the day_plans version: without a saved plan, the Schedule's plan is the floor from the
-- date's local midnight (cloister.schedule_plan_in_force), not from its start. Otherwise the same.
create or replace function cloister.day_plan_writing() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
  old_slots jsonb := '[]';
  old_minutes int := 0;
  fallback jsonb;
  r record;
begin
  if tg_op = 'UPDATE' then
    new.user_id := old.user_id;
    new.date := old.date;
    old_slots := old.slots;
    old_minutes := old.planned_deep_minutes;
  else
    -- No saved plan yet: once the Schedule's day has begun, its plan is the floor.
    fallback := cloister.schedule_plan_in_force(new.user_id, new.date, now);
    if fallback is not null then
      old_slots := fallback->'slots';
      old_minutes := (fallback->>'planned_deep_minutes')::int;
    end if;
  end if;
  if jsonb_typeof(new.slots) <> 'array' then
    perform cloister.reject('invalid_command');
  end if;
  -- Normalise: trimmed labels, RFC 3339 times, sorted by start. Unchanged Slots keep their
  -- Depth Rating.
  new.slots := coalesce((
    select jsonb_agg(cloister.slot_json(n.id, n.starts_at, n.ends_at, cloister.trim_label(n.label),
                                        n.deep, coalesce(n.depth_rating, o.depth_rating))
                     order by n.starts_at, n.id)
    from cloister.slots_of(new.slots) n
    left join cloister.slots_of(old_slots) o
      on o.id = n.id and o.starts_at = n.starts_at and o.ends_at = n.ends_at
     and o.label = cloister.trim_label(n.label) and o.deep = n.deep), '[]'::jsonb);

  if new.ends_at <= new.starts_at or new.ends_at - new.starts_at > interval '24 hours' then
    perform cloister.reject('invalid_day_plan');
  end if;
  if exists (select 1 from cloister.slots_of(new.slots) s
             where s.id is null or s.label = '' or s.ends_at <= s.starts_at
                or s.starts_at < new.starts_at or s.ends_at > new.ends_at) then
    perform cloister.reject('invalid_slot');
  end if;
  if exists (select 1 from (
               select s.starts_at, lag(s.ends_at) over (order by s.starts_at, s.id) as prev_end
               from cloister.slots_of(new.slots) s) x
             where x.starts_at < x.prev_end) then
    perform cloister.reject('slots_overlap');
  end if;
  if (select count(*) <> count(distinct s.id) from cloister.slots_of(new.slots) s)
     or exists (select 1 from public.day_plans p, cloister.slots_of(p.slots) o
                where p.user_id = new.user_id and p.date <> new.date
                  and o.id in (select s.id from cloister.slots_of(new.slots) s))
     or exists (select 1 from public.blocks b
                where b.user_id = new.user_id
                  and b.id in (select s.id from cloister.slots_of(new.slots) s)) then
    perform cloister.reject('duplicate_slot');
  end if;

  -- The ratchet.
  if exists (select 1 from cloister.slots_of(old_slots) o
             where o.deep and o.starts_at <= now
               and not exists (select 1 from cloister.slots_of(new.slots) n
                               where n.id = o.id and n.starts_at = o.starts_at
                                 and n.ends_at = o.ends_at and n.label = o.label and n.deep)) then
    perform cloister.reject('deep_session_has_started');
  end if;
  if exists (select 1 from cloister.slots_of(new.slots) n
             where n.deep and n.starts_at < now
               and not exists (select 1 from cloister.slots_of(old_slots) o
                               where o.deep and n.id = o.id and n.starts_at = o.starts_at
                                 and n.ends_at = o.ends_at and n.label = o.label)) then
    perform cloister.reject('slot_in_the_past');
  end if;
  -- A Depth Rating changes only by being recorded, under the same rules as for Blocks.
  for r in
    select n.deep, n.ends_at, n.depth_rating as rating, o.depth_rating as recorded
    from cloister.slots_of(new.slots) n
    left join cloister.slots_of(old_slots) o
      on o.id = n.id and o.starts_at = n.starts_at and o.ends_at = n.ends_at
     and o.label = n.label and o.deep = n.deep
    where n.depth_rating is distinct from o.depth_rating
  loop
    perform cloister.check_depth_rating(r.deep, r.ends_at, r.recorded::smallint, r.rating);
  end loop;
  new.planned_deep_minutes := coalesce((
    select sum(floor(extract(epoch from s.ends_at - s.starts_at) / 60))::int
    from cloister.slots_of(new.slots) s where s.deep), 0);
  if new.planned_deep_minutes < old_minutes then
    perform cloister.reject('slot_cannot_be_removed');
  end if;
  if tg_op = 'UPDATE' then
    new.created_at := old.created_at;
  else
    new.created_at := now;
  end if;
  new.updated_at := now;
  return new;
end
$$;

-- Replaces the day_plans version: today's plan on the old Schedule (no saved plan, not ended) is
-- saved as its Day Plan when it has started or when the new Schedule would give today fewer
-- Planned Deep Minutes (none if it no longer covers today). Otherwise the same.
create or replace function cloister.schedule_writing() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
  today date;
  plan jsonb;
  new_minutes int;
begin
  if not exists (select 1 from pg_catalog.pg_timezone_names where name = new.time_zone) then
    perform cloister.reject('unknown_time_zone');
  end if;
  if not cloister.is_hhmm(new.starts) or not cloister.is_hhmm(new.ends) or new.ends <= new.starts
     or exists (select 1 from unnest(new.weekdays) d where d is null or d not between 1 and 7)
     or cardinality(new.weekdays) <> (select count(distinct d) from unnest(new.weekdays) d) then
    perform cloister.reject('invalid_schedule');
  end if;
  if jsonb_typeof(new.slots) <> 'array' then
    perform cloister.reject('invalid_command');
  end if;
  if exists (select 1 from jsonb_array_elements(new.slots) x
             where not cloister.is_hhmm(x->>'starts') or not cloister.is_hhmm(x->>'ends')
                or cloister.trim_label(x->>'label') = '' or x->>'ends' <= x->>'starts'
                or x->>'starts' < new.starts or x->>'ends' > new.ends) then
    perform cloister.reject('invalid_slot');
  end if;
  new.slots := coalesce((
    select jsonb_agg(jsonb_build_object('starts', x->>'starts', 'ends', x->>'ends',
                                        'label', cloister.trim_label(x->>'label'),
                                        'deep', coalesce((x->>'deep')::boolean, false))
                     order by x->>'starts')
    from jsonb_array_elements(new.slots) x), '[]'::jsonb);
  if exists (select 1 from (
               select x->>'starts' as starts, lag(x->>'ends') over (order by x->>'starts') as prev_end
               from jsonb_array_elements(new.slots) x) y
             where y.starts < y.prev_end) then
    perform cloister.reject('slots_overlap');
  end if;
  if exists (select 1 from public.schedules s
             where s.user_id = new.user_id and s.id <> new.id and s.weekdays && new.weekdays) then
    perform cloister.reject('schedules_overlap');
  end if;
  new.weekdays := array(select d from unnest(new.weekdays) d order by d);
  if tg_op = 'UPDATE' then
    new.user_id := old.user_id;
    new.id := old.id;
    new.created_at := old.created_at;
    -- Today on the old Schedule keeps its plan when it is running, or when the change would cut
    -- its deep time: save it as its Day Plan.
    today := (now at time zone old.time_zone)::date;
    if extract(isodow from today)::smallint = any (old.weekdays)
       and not exists (select 1 from public.day_plans p
                       where p.user_id = old.user_id and p.date = today) then
      plan := cloister.schedule_plan_of(old, today);
      new_minutes := case when extract(isodow from today)::smallint = any (new.weekdays)
                          then (cloister.schedule_plan_of(new, today)->>'planned_deep_minutes')::int
                          else 0 end;
      if now < (plan->>'ends_at')::timestamptz
         and ((plan->>'starts_at')::timestamptz <= now
              or new_minutes < (plan->>'planned_deep_minutes')::int) then
        perform set_config('cloister.keeping_schedule_plan', 'on', true);
        insert into public.day_plans (user_id, date, starts_at, ends_at, slots)
        values (old.user_id, today, (plan->>'starts_at')::timestamptz,
                (plan->>'ends_at')::timestamptz, plan->'slots');
        perform set_config('cloister.keeping_schedule_plan', '', true);
      end if;
    end if;
  else
    new.created_at := now;
  end if;
  return new;
end
$$;
