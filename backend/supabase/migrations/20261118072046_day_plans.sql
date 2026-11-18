-- Day Plans, Slots and Schedules, mirroring the Rust rules engine (cloister-core/src/day_plan.rs)
-- and pinned by the shared fixtures in fixtures/rules (day-plan-*, schedule-*).
--
-- A Day Plan keeps its Slots as one jsonb array in the engine's JSON form, so a single row-level
-- trigger sees the whole day and can enforce the ratchet: Planned Deep Minutes never go down, a
-- started Deep Session never changes, and no deep Slot starts before server time. Deep Slots
-- are Deep Sessions from their start time; the Enforcement derives them, they are not copied
-- into public.blocks. A date with no saved Day Plan runs on the Schedule covering its weekday.
--
-- Clients only read these tables. Writes go through public.command() (redefined below with the
-- write_day_plan and put_schedule commands), and the triggers guard every write.

-- Tables -----------------------------------------------------------------------------------
create table public.day_plans (
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  date date not null,
  -- The Workday's bounds (Workday Start to Shutdown Time); given with the plan until those
  -- settings exist.
  starts_at timestamptz not null,
  ends_at timestamptz not null,
  -- Total length of the deep Slots, in minutes. Computed by the trigger; only goes up.
  planned_deep_minutes int not null default 0,
  -- [{id, starts_at, ends_at, label, deep, depth_rating?}], sorted by start. See fixtures/README.md.
  slots jsonb not null default '[]',
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  primary key (user_id, date)
);

create table public.schedules (
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  id text not null check (length(id) between 1 and 64),
  time_zone text not null,
  weekdays smallint[] not null default '{}',
  starts text not null,
  ends text not null,
  -- [{starts, ends, label, deep}] in local "HH:MM", sorted by start.
  slots jsonb not null default '[]',
  created_at timestamptz not null default now(),
  primary key (user_id, id)
);

alter table public.day_plans enable row level security;
alter table public.schedules enable row level security;
create policy own_select on public.day_plans for select to authenticated
  using ((select auth.uid()) = user_id);
create policy own_select on public.schedules for select to authenticated
  using ((select auth.uid()) = user_id);
revoke insert, update, delete, truncate on public.day_plans, public.schedules from anon, authenticated;
grant select on public.day_plans, public.schedules to authenticated;

alter publication supabase_realtime add table public.day_plans, public.schedules;

-- Helpers ----------------------------------------------------------------------------------
-- The Slots of a jsonb array, typed.
create function cloister.slots_of(slots jsonb)
returns table (id text, starts_at timestamptz, ends_at timestamptz, label text, deep boolean,
               depth_rating int, ord bigint)
language sql stable
set search_path = ''
as $$
  select x->>'id', (x->>'starts_at')::timestamptz, (x->>'ends_at')::timestamptz, x->>'label',
         coalesce((x->>'deep')::boolean, false), (x->>'depth_rating')::int, o
  from jsonb_array_elements(coalesce(slots, '[]'::jsonb)) with ordinality as t(x, o)
$$;

-- One Slot in the engine's JSON form.
create function cloister.slot_json(id text, starts_at timestamptz, ends_at timestamptz,
                                   label text, deep boolean, depth_rating int) returns jsonb
language sql stable
set search_path = ''
as $$
  select jsonb_build_object('id', id, 'starts_at', cloister.rfc3339(starts_at),
                            'ends_at', cloister.rfc3339(ends_at), 'label', label, 'deep', deep)
    || case when depth_rating is null then '{}'::jsonb
            else jsonb_build_object('depth_rating', depth_rating) end
$$;

create function cloister.trim_label(label text) returns text
language sql immutable
set search_path = ''
as $$ select regexp_replace(coalesce(label, ''), '^\s+|\s+$', '', 'g') $$;

create function cloister.is_hhmm(t text) returns boolean
language sql immutable
set search_path = ''
as $$ select coalesce(t ~ '^([01][0-9]|2[0-3]):[0-5][0-9]$', false) $$;

-- The plan Schedule `s` gives date `d`, in the engine's JSON form. Slot ids are
-- `<schedule id>-<date>-<n>`, n counting from 1 in order of start.
create function cloister.schedule_plan_of(s public.schedules, d date) returns jsonb
language sql stable
set search_path = ''
as $$
  with slots as (
    select o,
           ((d + (x->>'starts')::time) at time zone s.time_zone) as starts_at,
           ((d + (x->>'ends')::time) at time zone s.time_zone) as ends_at,
           x->>'label' as label,
           coalesce((x->>'deep')::boolean, false) as deep
    from jsonb_array_elements(s.slots) with ordinality as t(x, o)
  )
  select jsonb_build_object(
    'date', to_char(d, 'YYYY-MM-DD'),
    'starts_at', cloister.rfc3339((d + s.starts::time) at time zone s.time_zone),
    'ends_at', cloister.rfc3339((d + s.ends::time) at time zone s.time_zone),
    'planned_deep_minutes', coalesce((
      select sum(floor(extract(epoch from ends_at - starts_at) / 60))::int from slots where deep), 0),
    'slots', coalesce((
      select jsonb_agg(cloister.slot_json(s.id || '-' || to_char(d, 'YYYY-MM-DD') || '-' || o,
                                          starts_at, ends_at, label, deep, null) order by o)
      from slots), '[]'::jsonb))
$$;

-- The plan the user's Schedule gives date `d`, or null when no Schedule covers its weekday.
create function cloister.schedule_plan(uid uuid, d date) returns jsonb
language sql stable
set search_path = ''
as $$
  select cloister.schedule_plan_of(s, d)
  from public.schedules s
  where s.user_id = uid and extract(isodow from d)::smallint = any (s.weekdays)
  order by s.id
  limit 1
$$;

-- Whether a deep Slot is a running Deep Session at `t_at`: from a saved Day Plan, or from the
-- Schedule's plan of the current date in each Schedule's time zone when that date has no plan.
create function cloister.planned_deep_session_active(uid uuid, t_at timestamptz) returns boolean
language sql stable
set search_path = ''
as $$
  select exists (
    select 1 from public.day_plans p, cloister.slots_of(p.slots) s
    where p.user_id = uid and s.deep and s.starts_at <= t_at and t_at < s.ends_at
  ) or exists (
    select 1
    from (select distinct (t_at at time zone s.time_zone)::date as d
          from public.schedules s where s.user_id = uid) days,
         cloister.slots_of(cloister.schedule_plan(uid, days.d)->'slots') x
    where not exists (select 1 from public.day_plans p where p.user_id = uid and p.date = days.d)
      and x.deep and x.starts_at <= t_at and t_at < x.ends_at
  )
$$;

-- Writing a Day Plan (engine order: day bounds, Slots, overlap, ids, then the ratchet) --------
create function cloister.day_plan_writing() returns trigger
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
    fallback := cloister.schedule_plan(new.user_id, new.date);
    if fallback is not null and (fallback->>'starts_at')::timestamptz <= now then
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
create trigger day_plan_writing before insert or update on public.day_plans
  for each row execute function cloister.day_plan_writing();

-- A Day Plan can't be deleted while its owner exists (only the account cascade removes it).
create function cloister.day_plan_deleting() returns trigger
language plpgsql
set search_path = ''
as $$
begin
  if exists (select 1 from auth.users where id = old.user_id) then
    perform cloister.reject('slot_cannot_be_removed');
  end if;
  return old;
end
$$;
create trigger day_plan_deleting before delete on public.day_plans
  for each row execute function cloister.day_plan_deleting();

-- A Block id can't be a Slot's id (fires after block_inserting, so the engine's order holds).
create function cloister.block_slot_id() returns trigger
language plpgsql
set search_path = ''
as $$
begin
  if exists (select 1 from public.day_plans p, cloister.slots_of(p.slots) s
             where p.user_id = new.user_id and s.id = new.id) then
    perform cloister.reject('duplicate_block');
  end if;
  return new;
end
$$;
create trigger block_slot_id before insert on public.blocks
  for each row execute function cloister.block_slot_id();

-- Writing a Schedule (engine order: time zone, weekdays and times, Slots, overlap, others) ---
create function cloister.schedule_writing() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
  today date;
  plan jsonb;
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
    -- A day already running on the old Schedule keeps its plan: save it as its Day Plan.
    today := (now at time zone old.time_zone)::date;
    if extract(isodow from today)::smallint = any (old.weekdays)
       and not exists (select 1 from public.day_plans p
                       where p.user_id = old.user_id and p.date = today) then
      plan := cloister.schedule_plan_of(old, today);
      if (plan->>'starts_at')::timestamptz <= now and now < (plan->>'ends_at')::timestamptz then
        insert into public.day_plans (user_id, date, starts_at, ends_at, slots)
        values (old.user_id, today, (plan->>'starts_at')::timestamptz,
                (plan->>'ends_at')::timestamptz, plan->'slots');
      end if;
    end if;
  else
    new.created_at := now;
  end if;
  return new;
end
$$;
create trigger schedule_writing before insert or update on public.schedules
  for each row execute function cloister.schedule_writing();

-- State and Enforcement --------------------------------------------------------------------
-- The signed-in user's state, in the engine's JSON form (day_plans and schedules omitted
-- when empty, as the engine does).
create or replace function public.state() returns jsonb
language sql stable
set search_path = ''
as $$
  select jsonb_build_object('blocks', coalesce((
      select jsonb_agg(jsonb_build_object(
        'id', id,
        'starts_at', cloister.rfc3339(starts_at),
        'ends_at', cloister.rfc3339(ends_at),
        'blocklists', to_jsonb(blocklists),
        'deep', deep)
        || case when intent is null then '{}'::jsonb else jsonb_build_object('intent', intent) end
        || case when depth_rating is null then '{}'::jsonb
                else jsonb_build_object('depth_rating', depth_rating) end
        order by starts_at, id)
      from public.blocks where user_id = (select auth.uid())), '[]'::jsonb))
  || coalesce((
      select jsonb_build_object('day_plans', jsonb_agg(jsonb_build_object(
        'date', to_char(date, 'YYYY-MM-DD'),
        'starts_at', cloister.rfc3339(starts_at),
        'ends_at', cloister.rfc3339(ends_at),
        'planned_deep_minutes', planned_deep_minutes,
        'slots', slots) order by date))
      from public.day_plans where user_id = (select auth.uid())
      having count(*) > 0), '{}'::jsonb)
  || coalesce((
      select jsonb_build_object('schedules', jsonb_agg(jsonb_build_object(
        'id', id, 'time_zone', time_zone, 'weekdays', to_jsonb(weekdays),
        'starts', starts, 'ends', ends, 'slots', slots) order by id))
      from public.schedules where user_id = (select auth.uid())
      having count(*) > 0), '{}'::jsonb)
$$;

-- What the signed-in user's Devices must enforce at server time. A running deep Slot blocks
-- the Distraction List, like a deep Block with no extra Blocklists.
create or replace function public.enforcement() returns jsonb
language sql stable
set search_path = ''
as $$
  with active as (
    select distinct unnest(blocklists) as list
    from public.blocks
    where user_id = (select auth.uid())
      and starts_at <= cloister.server_now() and cloister.server_now() < ends_at
    union
    select unnest(cloister.distraction_ids())
    where cloister.planned_deep_session_active((select auth.uid()), cloister.server_now())
  ), domains as (
    select unnest(l.domains) as domain
    from public.blocklists l join active a on a.list = l.id
  ), all_domains as (
    select domain from domains
    union
    select unnest(r.related) from public.related_domains r join domains d using (domain)
  )
  select jsonb_build_object(
    'blocked_domains', coalesce((select jsonb_agg(domain order by domain) from all_domains), '[]'::jsonb),
    'blocked_apps', coalesce((select jsonb_agg(app order by app) from (
        select distinct unnest(l.apps) as app
        from public.blocklists l join active a on a.list = l.id) apps), '[]'::jsonb),
    'ritual', 'none')
$$;

-- Commands ---------------------------------------------------------------------------------
-- The Slots of a write_day_plan command, checked for shape, as a jsonb array for the trigger.
create function cloister.command_slots(slots jsonb) returns jsonb
language plpgsql
set search_path = ''
as $$
begin
  if jsonb_typeof(slots) <> 'array' then
    perform cloister.reject('invalid_command');
  end if;
  if exists (select 1 from jsonb_array_elements(slots) x
             where jsonb_typeof(x) <> 'object'
                or exists (select 1 from jsonb_object_keys(x) k
                           where k not in ('id', 'starts_at', 'ends_at', 'label', 'deep'))
                or not (x ? 'id' and x ? 'starts_at' and x ? 'ends_at' and x ? 'label')
                or jsonb_typeof(x->'id') <> 'string' or jsonb_typeof(x->'label') <> 'string'
                or jsonb_typeof(coalesce(x->'deep', 'false')) <> 'boolean') then
    perform cloister.reject('invalid_command');
  end if;
  return coalesce((
    select jsonb_agg(jsonb_build_object(
      'id', x->>'id',
      'starts_at', cloister.rfc3339((x->>'starts_at')::timestamptz),
      'ends_at', cloister.rfc3339((x->>'ends_at')::timestamptz),
      'label', x->>'label',
      'deep', coalesce((x->>'deep')::boolean, false)))
    from jsonb_array_elements(slots) x), '[]'::jsonb);
end
$$;

-- The Day Plan and Schedule commands. Returns false for any other command type.
create function cloister.day_plan_command(uid uuid, cmd jsonb) returns boolean
language plpgsql
set search_path = ''
as $$
declare
  kind text := cmd->>'type';
  plan_date date;
  days smallint[];
begin
  case kind
  when 'write_day_plan' then
    perform cloister.check_keys(cmd, array['date', 'starts_at', 'ends_at', 'slots'], array[]::text[]);
    plan_date := (cmd->>'date')::date;
    if exists (select 1 from public.day_plans p where p.user_id = uid and p.date = plan_date) then
      update public.day_plans p
      set starts_at = (cmd->>'starts_at')::timestamptz, ends_at = (cmd->>'ends_at')::timestamptz,
          slots = cloister.command_slots(cmd->'slots')
      where p.user_id = uid and p.date = plan_date;
    else
      insert into public.day_plans (user_id, date, starts_at, ends_at, slots)
      values (uid, plan_date, (cmd->>'starts_at')::timestamptz, (cmd->>'ends_at')::timestamptz,
              cloister.command_slots(cmd->'slots'));
    end if;
  when 'put_schedule' then
    perform cloister.check_keys(cmd, array['schedule_id', 'time_zone', 'weekdays', 'starts', 'ends', 'slots'],
                                array[]::text[]);
    if jsonb_typeof(cmd->'weekdays') <> 'array' or jsonb_typeof(cmd->'slots') <> 'array'
       or jsonb_typeof(cmd->'schedule_id') <> 'string' or jsonb_typeof(cmd->'time_zone') <> 'string'
       or jsonb_typeof(cmd->'starts') <> 'string' or jsonb_typeof(cmd->'ends') <> 'string'
       or exists (select 1 from jsonb_array_elements(cmd->'weekdays') d where jsonb_typeof(d) <> 'number')
       or exists (select 1 from jsonb_array_elements(cmd->'slots') x
                  where jsonb_typeof(x) <> 'object'
                     or exists (select 1 from jsonb_object_keys(x) k
                                where k not in ('starts', 'ends', 'label', 'deep'))
                     or not (x ? 'starts' and x ? 'ends' and x ? 'label')
                     or jsonb_typeof(x->'starts') <> 'string' or jsonb_typeof(x->'ends') <> 'string'
                     or jsonb_typeof(x->'label') <> 'string'
                     or jsonb_typeof(coalesce(x->'deep', 'false')) <> 'boolean') then
      perform cloister.reject('invalid_command');
    end if;
    -- Clamped so any out-of-range number reads as out of range, not as an overflow.
    days := array(select least(greatest(d::numeric, -1), 99)::smallint
                  from jsonb_array_elements_text(cmd->'weekdays') d);
    if exists (select 1 from public.schedules s where s.user_id = uid and s.id = cmd->>'schedule_id') then
      update public.schedules s
      set time_zone = cmd->>'time_zone', weekdays = days,
          starts = cmd->>'starts', ends = cmd->>'ends', slots = cmd->'slots'
      where s.user_id = uid and s.id = cmd->>'schedule_id';
    else
      insert into public.schedules (user_id, id, time_zone, weekdays, starts, ends, slots)
      values (uid, cmd->>'schedule_id', cmd->>'time_zone', days,
              cmd->>'starts', cmd->>'ends', cmd->'slots');
    end if;
  else
    return false;
  end case;
  return true;
end
$$;

-- Recording a Depth Rating for a deep Slot's Deep Session (the Block branch comes first).
create function cloister.rate_slot(uid uuid, slot_id text, rating numeric) returns void
language plpgsql
set search_path = ''
as $$
declare
  plan_date date;
  slot_deep boolean;
  slot_ends timestamptz;
  recorded int;
begin
  select p.date, x.deep, x.ends_at, x.depth_rating into plan_date, slot_deep, slot_ends, recorded
  from public.day_plans p, cloister.slots_of(p.slots) x
  where p.user_id = uid and x.id = slot_id;
  if plan_date is null then
    perform cloister.reject('unknown_block');
  end if;
  perform cloister.check_depth_rating(slot_deep, slot_ends, recorded::smallint,
                                      least(greatest(rating, 0), 6)::int);
  update public.day_plans p
  set slots = (select jsonb_agg(case when x->>'id' = slot_id
                                     then x || jsonb_build_object('depth_rating', rating::int)
                                     else x end order by o)
               from jsonb_array_elements(p.slots) with ordinality as t(x, o))
  where p.user_id = uid and p.date = plan_date;
end
$$;

-- public.command() with the Day Plan commands added (Block branches unchanged, except that a
-- Depth Rating may name a deep Slot).
create or replace function public.command(cmd jsonb) returns jsonb
language plpgsql security definer
set search_path = ''
as $$
declare
  uid uuid := auth.uid();
  now timestamptz := cloister.server_now();
  kind text := cmd->>'type';
  minutes numeric;
  block public.blocks;
begin
  if uid is null then
    return jsonb_build_object('outcome', 'rejected', 'reason', 'not_signed_in');
  end if;
  begin
    if jsonb_typeof(cmd) <> 'object' then
      perform cloister.reject('invalid_command');
    end if;
    case kind
    when 'start_block' then
      perform cloister.check_keys(cmd, array['block_id', 'minutes'],
                                  array['deep', 'blocklists', 'confirm_long', 'client_time', 'intent']);
      if jsonb_typeof(cmd->'minutes') <> 'number' then
        perform cloister.reject('invalid_command');
      end if;
      -- Clamp before building an interval; the triggers still see the true category.
      minutes := least(greatest((cmd->>'minutes')::numeric, -1), 24 * 60 + 1);
      if jsonb_typeof(coalesce(cmd->'intent', '""')) not in ('string', 'null') then
        perform cloister.reject('invalid_command');
      end if;
      insert into public.blocks (user_id, id, starts_at, ends_at, blocklists, deep, confirmed_long, intent)
      values (uid, cmd->>'block_id', now, now + make_interval(mins => minutes::int),
              array(select jsonb_array_elements_text(coalesce(cmd->'blocklists', '[]'))),
              coalesce((cmd->>'deep')::boolean, true),
              coalesce((cmd->>'confirm_long')::boolean, false),
              cmd->>'intent');
    when 'extend_block' then
      perform cloister.check_keys(cmd, array['block_id', 'ends_at'], array['confirm_long']);
      select * into block from public.blocks b
        where b.user_id = uid and b.id = cmd->>'block_id' for update;
      if not found then
        perform cloister.reject('unknown_block');
      end if;
      if now >= block.ends_at then
        perform cloister.reject('block_has_ended');
      end if;
      if (cmd->>'ends_at')::timestamptz <> block.ends_at then
        update public.blocks b
        set ends_at = (cmd->>'ends_at')::timestamptz,
            confirmed_long = coalesce((cmd->>'confirm_long')::boolean, false)
        where b.user_id = uid and b.id = block.id;
      end if;
    when 'record_depth_rating' then
      perform cloister.check_keys(cmd, array['block_id', 'rating'], array[]::text[]);
      if jsonb_typeof(cmd->'rating') <> 'number' then
        perform cloister.reject('invalid_command');
      end if;
      select * into block from public.blocks b
        where b.user_id = uid and b.id = cmd->>'block_id' for update;
      if not found then
        perform cloister.rate_slot(uid, cmd->>'block_id', (cmd->>'rating')::numeric);
      else
        -- Clamped so any out-of-range number reads as out of range, not as an overflow.
        perform cloister.check_depth_rating(block.deep, block.ends_at, block.depth_rating,
          least(greatest((cmd->>'rating')::numeric, 0), 6)::int);
        update public.blocks b set depth_rating = (cmd->>'rating')::smallint
        where b.user_id = uid and b.id = block.id;
      end if;
    when 'cancel_block' then
      perform cloister.check_keys(cmd, array['block_id'], array[]::text[]);
      delete from public.blocks b where b.user_id = uid and b.id = cmd->>'block_id';
      perform cloister.reject('unknown_block');  -- reached only when nothing matched
    else
      if not cloister.day_plan_command(uid, cmd) then
        perform cloister.reject('invalid_command');
      end if;
    end case;
  exception
    when sqlstate 'CL001' then
      return jsonb_build_object('outcome', 'rejected', 'reason', sqlerrm);
    when invalid_text_representation or invalid_datetime_format or datetime_field_overflow
         or invalid_parameter_value or numeric_value_out_of_range then
      return jsonb_build_object('outcome', 'rejected', 'reason', 'invalid_command');
  end;
  return jsonb_build_object('outcome', 'accepted', 'reason', null);
end
$$;

revoke execute on function public.command(jsonb) from public, anon;
grant execute on function public.command(jsonb) to authenticated;
