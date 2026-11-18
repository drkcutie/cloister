-- The weekly Deep Hours target, the Weekly Review, no Missed Ritual before the first settings,
-- and the Shutdown Delay from inside the ritual window. Mirrors cloister-core/src/review.rs and
-- ritual.rs; pinned by the shared fixtures weekly-target-*, weekly-review-*,
-- no-missed-ritual-before-settings and shutdown-delay-from-the-ritual. See fixtures/README.md.
--
-- Doesn't touch public.command, public.state or public.enforcement: the new command and state
-- parts go through the extension hooks (cloister.command_<type>, cloister.state_part_<name>).
-- Replaces these functions from 20261119090000_shutdown_ritual.sql (a later replacement must
-- keep their behaviour here): cloister.ritual_due_at, cloister.shutdown_delay_inserting,
-- cloister.ritual_inserting, cloister.command_complete_ritual.

-- When the account's settings began. A ritual due before then doesn't exist. Null = unknown
-- (rows from before this migration): every Workday counts.
alter table public.settings add column settings_since timestamptz;

create function cloister.settings_since_stamping() returns trigger
language plpgsql
set search_path = ''
as $$
begin
  if tg_op = 'INSERT' then
    new.settings_since := cloister.server_now();
  else
    new.settings_since := old.settings_since;
  end if;
  return new;
end
$$;
create trigger settings_since_stamping before insert or update on public.settings
  for each row execute function cloister.settings_since_stamping();

create function cloister.state_part_settings_since(uid uuid) returns jsonb
language sql stable
set search_path = ''
as $$
  select coalesce((select jsonb_build_object('settings_since', cloister.rfc3339(s.settings_since))
                   from public.settings s where s.user_id = uid and s.settings_since is not null),
                  '{}'::jsonb)
$$;

-- Replaces the shutdown_ritual version: no ritual is due before the first settings.
create or replace function cloister.ritual_due_at(uid uuid, d date, t_at timestamptz) returns timestamptz
language sql stable
set search_path = ''
as $$
  select due
  from (select cloister.workday_shutdown(uid, d, t_at)
               + case when exists (select 1 from public.shutdown_delays x where x.user_id = uid and x.date = d)
                      then interval '30 minutes' else interval '0' end as due) r
  where r.due >= coalesce((select s.settings_since from public.settings s where s.user_id = uid),
                          '-infinity'::timestamptz)
$$;

-- The weekly target. hours = in force as of the last change; pending_* = a lower target waiting
-- for the next week boundary (local Monday midnight).
create table public.weekly_targets (
  user_id uuid primary key default auth.uid() references auth.users (id) on delete cascade,
  hours smallint not null,
  pending_hours smallint,
  pending_effective_at timestamptz,
  updated_at timestamptz not null default now()
);
alter table public.weekly_targets enable row level security;
create policy own_select on public.weekly_targets for select to authenticated
  using ((select auth.uid()) = user_id);
revoke insert, update, delete, truncate on public.weekly_targets from anon, authenticated;
grant select on public.weekly_targets to authenticated;
alter publication supabase_realtime add table public.weekly_targets;

-- The first local Monday midnight after `at` (engine: next_week_boundary).
create function cloister.next_week_boundary(tz text, at timestamptz) returns timestamptz
language plpgsql stable
set search_path = ''
as $$
declare
  z text := coalesce(tz, 'UTC');
  d date;
begin
  begin
    perform at at time zone z;
  exception when others then
    z := 'UTC';
  end;
  d := (at at time zone z)::date;
  return ((d - (extract(isodow from d)::int - 1) + 7)::timestamp) at time zone z;
end
$$;

-- The target in force for the user at `t_at` (null: none).
create function cloister.weekly_target_at(uid uuid, t_at timestamptz) returns smallint
language sql stable
set search_path = ''
as $$
  select case when w.pending_effective_at <= t_at then w.pending_hours else w.hours end
  from public.weekly_targets w where w.user_id = uid
$$;

-- Guard: raising applies now, lowering from the next week boundary. NEW.hours is the request.
create function cloister.weekly_target_writing() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
  base smallint;
begin
  if new.hours is null or new.hours not between 1 and 100 then
    perform cloister.reject('invalid_weekly_target');
  end if;
  new.updated_at := now;
  new.pending_hours := null;
  new.pending_effective_at := null;
  if tg_op = 'UPDATE' then
    new.user_id := old.user_id;
    base := case when old.pending_effective_at <= now then old.pending_hours else old.hours end;
    if base > new.hours then
      new.pending_hours := new.hours;
      new.pending_effective_at := cloister.next_week_boundary(cloister.user_time_zone(new.user_id), now);
      new.hours := base;
    end if;
  end if;
  return new;
end
$$;
create trigger weekly_target_writing before insert or update on public.weekly_targets
  for each row execute function cloister.weekly_target_writing();

create function cloister.weekly_target_deleting() returns trigger
language plpgsql
set search_path = ''
as $$
begin
  if cloister.account_gone(old.user_id) then
    return old;
  end if;
  perform cloister.reject('invalid_weekly_target');
  return null;
end
$$;
create trigger weekly_target_deleting before delete on public.weekly_targets
  for each row execute function cloister.weekly_target_deleting();

-- Set (or ask for) a target; the guard decides what is in force.
create function cloister.write_weekly_target(uid uuid, requested smallint) returns void
language plpgsql
set search_path = ''
as $$
begin
  insert into public.weekly_targets as w (user_id, hours) values (uid, requested)
  on conflict (user_id) do update set hours = excluded.hours;
end
$$;

-- A JSON hours value as a smallint, clamped so an out-of-range number reads as out of range.
-- Not an integer: invalid_command.
create function cloister.hours_of(v jsonb) returns smallint
language plpgsql immutable
set search_path = ''
as $$
begin
  if jsonb_typeof(v) <> 'number' or (v::text)::numeric <> trunc((v::text)::numeric)
     or (v::text)::numeric < 0 then
    perform cloister.reject('invalid_command');
  end if;
  return least((v::text)::numeric, 1000)::smallint;
end
$$;

create function cloister.command_set_weekly_target(uid uuid, cmd jsonb) returns void
language plpgsql
set search_path = ''
as $$
begin
  perform cloister.check_keys(cmd, array['hours'], array[]::text[]);
  perform cloister.write_weekly_target(uid, cloister.hours_of(cmd->'hours'));
end
$$;

create function cloister.state_part_weekly_target(uid uuid) returns jsonb
language sql stable
set search_path = ''
as $$
  select coalesce((
    select jsonb_build_object('weekly_target', jsonb_build_object('hours', w.hours))
        || case when w.pending_effective_at is null then '{}'::jsonb
                else jsonb_build_object('pending_weekly_target', jsonb_build_object(
                       'hours', w.pending_hours,
                       'effective_at', cloister.rfc3339(w.pending_effective_at))) end
    from public.weekly_targets w where w.user_id = uid), '{}'::jsonb)
$$;

-- Whether local date `d` is the last Workday of its ISO week, as seen at `t_at`.
create function cloister.is_last_workday_of_week(uid uuid, d date, t_at timestamptz) returns boolean
language sql stable
set search_path = ''
as $$
  select not exists (
    select 1 from generate_series(1, 7 - extract(isodow from d)::int) n
    where cloister.workday_shutdown(uid, d + n, t_at) is not null)
$$;

-- Replaces the shutdown_ritual version: a Shutdown Delay may be used from Workday Start until
-- 30 minutes after Shutdown Time (so also from the ritual), while today's ritual is open.
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
  select * into s from cloister.settings_at(new.user_id, now);
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

-- Replaces the shutdown_ritual version: adds the Weekly Review check before the typed text.
-- `typed` and `weekly_target` travel in transaction-local settings, since they aren't stored.
create or replace function cloister.ritual_inserting() returns trigger
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
  target text := nullif(current_setting('cloister.weekly_target', true), '');
  last_day boolean;
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
  last_day := cloister.is_last_workday_of_week(new.user_id, r.date, now);
  if last_day and target is null then
    perform cloister.reject('weekly_review_required');
  end if;
  if not last_day and target is not null then
    perform cloister.reject('weekly_review_not_due');
  end if;
  if target is not null and target::int not between 1 and 100 then
    perform cloister.reject('invalid_weekly_target');
  end if;
  if coalesce(current_setting('cloister.typed', true), '') <> 'Shutdown complete' then
    perform cloister.reject('confirmation_mismatch');
  end if;
  new.date := r.date;
  new.completed_at := now;
  return new;
end
$$;

-- Replaces the shutdown_ritual version: takes the Weekly Review's `weekly_target` (optional).
create or replace function cloister.command_complete_ritual(uid uuid, cmd jsonb) returns void
language plpgsql
set search_path = ''
as $$
declare
  target smallint;
begin
  perform cloister.check_keys(cmd, array['typed'], array['weekly_target']);
  if jsonb_typeof(cmd->'typed') <> 'string' then
    perform cloister.reject('invalid_command');
  end if;
  if coalesce(jsonb_typeof(cmd->'weekly_target'), 'null') <> 'null' then
    target := cloister.hours_of(cmd->'weekly_target');
  end if;
  perform set_config('cloister.typed', cmd->>'typed', true);
  perform set_config('cloister.weekly_target', coalesce(target::text, ''), true);
  -- date and completed_at are set by the guard.
  insert into public.rituals (user_id, date, completed_at)
  values (uid, current_date, cloister.server_now());
  perform set_config('cloister.typed', '', true);
  perform set_config('cloister.weekly_target', '', true);
  if target is not null then
    perform cloister.write_weekly_target(uid, target);
  end if;
end
$$;
