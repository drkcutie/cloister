-- public.command() and public.state() become extensible, so later rules add functions instead
-- of replacing these two (which parallel work would otherwise fight over):
--   * a command whose type the switch below doesn't know runs
--     cloister.command_<type>(uid uuid, cmd jsonb) returns void, if it exists;
--   * state() merges in the jsonb of every cloister.state_part_<name>(uid uuid).
-- start_block also takes the extras for one Block (extra_domains, extra_apps).

create function cloister.string_array(v jsonb) returns text[]
language plpgsql immutable
set search_path = ''
as $$
begin
  if v is null or v = 'null'::jsonb then
    return '{}';
  end if;
  if jsonb_typeof(v) <> 'array'
     or exists (select 1 from jsonb_array_elements(v) e where jsonb_typeof(e) <> 'string') then
    perform cloister.reject('invalid_command');
  end if;
  return array(select jsonb_array_elements_text(v));
end
$$;

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
        perform cloister.reject('unknown_block');
      end if;
      -- Clamped so any out-of-range number reads as out of range, not as an overflow.
      perform cloister.check_depth_rating(block.deep, block.ends_at, block.depth_rating,
        least(greatest((cmd->>'rating')::numeric, 0), 6)::int);
      update public.blocks b set depth_rating = (cmd->>'rating')::smallint
      where b.user_id = uid and b.id = block.id;
    when 'cancel_block' then
      perform cloister.check_keys(cmd, array['block_id'], array[]::text[]);
      delete from public.blocks b where b.user_id = uid and b.id = cmd->>'block_id';
      perform cloister.reject('unknown_block');  -- reached only when nothing matched
    else
      if kind ~ '^[a-z][a-z_]{0,50}$'
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
         or invalid_parameter_value then
      return jsonb_build_object('outcome', 'rejected', 'reason', 'invalid_command');
  end;
  return jsonb_build_object('outcome', 'accepted', 'reason', null);
end
$$;

create or replace function public.state() returns jsonb
language plpgsql stable
set search_path = ''
as $$
declare
  uid uuid := (select auth.uid());
  result jsonb;
  part jsonb;
  f record;
begin
  select jsonb_build_object('blocks', coalesce(jsonb_agg(jsonb_build_object(
    'id', id,
    'starts_at', cloister.rfc3339(starts_at),
    'ends_at', cloister.rfc3339(ends_at),
    'blocklists', to_jsonb(blocklists),
    'deep', deep)
    || case when intent is null then '{}'::jsonb else jsonb_build_object('intent', intent) end
    || case when depth_rating is null then '{}'::jsonb
            else jsonb_build_object('depth_rating', depth_rating) end
    || case when extra_domains = '{}' then '{}'::jsonb
            else jsonb_build_object('extra_domains', to_jsonb(extra_domains)) end
    || case when extra_apps = '{}' then '{}'::jsonb
            else jsonb_build_object('extra_apps', to_jsonb(extra_apps)) end
    || case when quit_social then jsonb_build_object('quit_social', true) else '{}'::jsonb end
    order by starts_at, id), '[]'::jsonb))
  into result
  from public.blocks
  where user_id = uid;
  if uid is null then
    return result;
  end if;
  for f in select p.proname::text as name from pg_catalog.pg_proc p
           join pg_catalog.pg_namespace n on n.oid = p.pronamespace
           where n.nspname = 'cloister' and p.proname like 'state\_part\_%'
           order by 1 loop
    execute format('select cloister.%I($1)', f.name) into part using uid;
    result := result || coalesce(part, '{}'::jsonb);
  end loop;
  return result;
end
$$;
