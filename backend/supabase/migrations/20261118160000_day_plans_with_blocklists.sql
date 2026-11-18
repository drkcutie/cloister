-- Ticket #7 (Day Plans, 20261118072046) and ticket #4 (Blocklists, 20261118150010/150020) both
-- replaced public.command, public.state and public.enforcement. This migration holds the one
-- definition of each with both sets of rules:
--   * command: the extensible command of #4 (start_block extras, cloister.command_<type>), with
--     #7's Depth Rating of a deep Slot and its write_day_plan / put_schedule hook;
--   * state: #4's extensible state() stays; #7's day_plans and schedules come in as a state part;
--   * enforcement: #4's Blocklist edits, extras, opt-outs and Quit-Social, plus #7's running deep
--     Slots, which block the Distraction List categories not opted out at the Slot's start, and
--     (like any Block that isn't the Quit-Social Block) those not opted out now.
-- A later change to any of these three must keep all of this.

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
                                  array['deep', 'blocklists', 'confirm_long', 'client_time', 'intent',
                                        'extra_domains', 'extra_apps']);
      if jsonb_typeof(cmd->'minutes') <> 'number' then
        perform cloister.reject('invalid_command');
      end if;
      -- Clamp before building an interval; the triggers still see the true category.
      minutes := least(greatest((cmd->>'minutes')::numeric, -1), 24 * 60 + 1);
      if jsonb_typeof(coalesce(cmd->'intent', '""')) not in ('string', 'null') then
        perform cloister.reject('invalid_command');
      end if;
      insert into public.blocks (user_id, id, starts_at, ends_at, blocklists, deep, confirmed_long, intent,
                                 extra_domains, extra_apps)
      values (uid, cmd->>'block_id', now, now + make_interval(mins => minutes::int),
              array(select jsonb_array_elements_text(coalesce(cmd->'blocklists', '[]'))),
              coalesce((cmd->>'deep')::boolean, true),
              coalesce((cmd->>'confirm_long')::boolean, false),
              cmd->>'intent',
              cloister.string_array(cmd->'extra_domains'),
              cloister.string_array(cmd->'extra_apps'));
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
        -- Not a Block: a deep Slot's id (rate_slot rejects unknown_block otherwise).
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
      if cloister.day_plan_command(uid, cmd) then
        null;  -- write_day_plan, put_schedule
      elsif kind ~ '^[a-z][a-z_]{0,50}$'
         and to_regprocedure(format('cloister.command_%s(uuid, jsonb)', kind)) is not null then
        execute format('select cloister.%I($1, $2)', 'command_' || kind) using uid, cmd;
      else
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

-- The Day Plans and Schedules part of public.state() (keys omitted when empty, as the engine does).
create function cloister.state_part_day_plans(uid uuid) returns jsonb
language sql stable
set search_path = ''
as $$
  select coalesce((
      select jsonb_build_object('day_plans', jsonb_agg(jsonb_build_object(
        'date', to_char(date, 'YYYY-MM-DD'),
        'starts_at', cloister.rfc3339(starts_at),
        'ends_at', cloister.rfc3339(ends_at),
        'planned_deep_minutes', planned_deep_minutes,
        'slots', slots) order by date))
      from public.day_plans where user_id = uid
      having count(*) > 0), '{}'::jsonb)
  || coalesce((
      select jsonb_build_object('schedules', jsonb_agg(jsonb_build_object(
        'id', id, 'time_zone', time_zone, 'weekdays', to_jsonb(weekdays),
        'starts', starts, 'ends', ends, 'slots', slots) order by id collate "C"))
      from public.schedules where user_id = uid
      having count(*) > 0), '{}'::jsonb)
$$;

-- The starts of the deep Slots running at `t_at` (same plans as planned_deep_session_active).
create function cloister.running_deep_slot_starts(uid uuid, t_at timestamptz)
returns setof timestamptz
language sql stable
set search_path = ''
as $$
  select s.starts_at
  from public.day_plans p, cloister.slots_of(p.slots) s
  where p.user_id = uid and s.deep and s.starts_at <= t_at and t_at < s.ends_at
  union
  select x.starts_at
  from (select distinct (t_at at time zone s.time_zone)::date as d
        from public.schedules s where s.user_id = uid) days,
       cloister.slots_of(cloister.schedule_plan(uid, days.d)->'slots') x
  where not exists (select 1 from public.day_plans p where p.user_id = uid and p.date = days.d)
    and x.deep and x.starts_at <= t_at and t_at < x.ends_at
$$;

create or replace function public.enforcement() returns jsonb
language sql stable
set search_path = ''
as $$
  with me as (
    select (select auth.uid()) as uid, cloister.server_now() as now
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
    union
    select unnest(cloister.distraction_ids_in_force(me.uid, s.starts_at)) from me, slot_starts s
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
