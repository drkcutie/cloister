-- The Block rules (ADR 0004), mirroring the Rust rules engine and pinned by fixtures/rules.
--
-- Triggers on public.blocks guard every write, whoever makes it. public.command() takes a
-- command in the shared fixture JSON form and turns it into a write, so the triggers decide.
-- A rule violation raises SQLSTATE CL001 with the stable reason code as the message.

create function cloister.reject(reason text) returns void
language plpgsql
set search_path = ''
as $$
begin
  raise exception using errcode = 'CL001', message = reason;
end
$$;

-- The 24-hour cap and the long-Block confirmation (same order of checks as the engine).
create function cloister.check_length(length interval, confirmed_long boolean) returns void
language plpgsql
set search_path = ''
as $$
begin
  if length <= interval '0' then
    perform cloister.reject('invalid_length');
  elsif length > interval '24 hours' then
    perform cloister.reject('block_too_long');
  elsif length > interval '4 hours' and not confirmed_long then
    perform cloister.reject('confirmation_required');
  end if;
end
$$;

create function cloister.distraction_ids() returns text[]
language sql stable
set search_path = ''
as $$ select array(select id from public.blocklists where kind = 'distraction' order by id) $$;

-- Recording a Depth Rating: Deep Sessions only, once, from their end, 1 to 5 (engine order).
create function cloister.check_depth_rating(deep boolean, ends_at timestamptz, recorded smallint,
                                            rating int) returns void
language plpgsql
set search_path = ''
as $$
begin
  if not deep then
    perform cloister.reject('not_a_deep_session');
  elsif cloister.server_now() < ends_at then
    perform cloister.reject('deep_session_not_ended');
  elsif rating is null or rating not between 1 and 5 then
    perform cloister.reject('invalid_depth_rating');
  elsif recorded is not null then
    perform cloister.reject('depth_rating_already_recorded');
  end if;
end
$$;

-- Starting a Block: it starts at server time whatever the client sent (only the length the
-- client asked for counts), always covers the Distraction List, and can't reuse an id. The
-- Intent is trimmed; a blank one is none.
create function cloister.block_inserting() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
  length interval := new.ends_at - new.starts_at;
begin
  perform cloister.check_length(length, new.confirmed_long);
  if exists (select 1 from unnest(new.blocklists) as l(id)
             where l.id not in (select b.id from public.blocklists b)) then
    perform cloister.reject('unknown_blocklist');
  end if;
  if exists (select 1 from public.blocks b where b.user_id = new.user_id and b.id = new.id) then
    perform cloister.reject('duplicate_block');
  end if;
  new.starts_at := now;
  new.ends_at := now + length;
  new.blocklists := array(select distinct x from unnest(new.blocklists || cloister.distraction_ids()) as t(x) order by 1);
  new.intent := nullif(regexp_replace(new.intent, '^\s+|\s+$', '', 'g'), '');
  new.depth_rating := null;
  new.created_at := now;
  return new;
end
$$;
create trigger block_inserting before insert on public.blocks
  for each row execute function cloister.block_inserting();

-- Changing a Block: its end only moves later, within the cap, and only while it runs. Nothing
-- else about it changes, except that a Deep Session gets its Depth Rating once it has ended.
create function cloister.block_updating() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
begin
  if new.user_id <> old.user_id or new.id <> old.id or new.starts_at <> old.starts_at
     or new.blocklists <> old.blocklists or new.deep <> old.deep
     or new.created_at <> old.created_at or new.intent is distinct from old.intent then
    perform cloister.reject('block_cannot_be_changed');
  end if;
  if new.ends_at <> old.ends_at then
    if now >= old.ends_at then
      perform cloister.reject('block_has_ended');
    end if;
    if new.ends_at < old.ends_at then
      perform cloister.reject('block_cannot_be_shortened');
    end if;
    perform cloister.check_length(new.ends_at - new.starts_at, new.confirmed_long);
  end if;
  if new.depth_rating is distinct from old.depth_rating then
    perform cloister.check_depth_rating(old.deep, old.ends_at, old.depth_rating, new.depth_rating);
  end if;
  return new;
end
$$;
create trigger block_updating before update on public.blocks
  for each row execute function cloister.block_updating();

-- A Block can't be cancelled, by anyone. Deleting the account is the only way its Blocks go:
-- by the time the cascade from auth.users reaches them, the user row is gone.
create function cloister.block_deleting() returns trigger
language plpgsql
set search_path = ''
as $$
begin
  if exists (select 1 from auth.users where id = old.user_id) then
    perform cloister.reject('block_cannot_be_cancelled');
  end if;
  return old;
end
$$;
create trigger block_deleting before delete on public.blocks
  for each row execute function cloister.block_deleting();

-- The signed-in user's state, in the engine's JSON form.
create function public.state() returns jsonb
language sql stable
set search_path = ''
as $$
  select jsonb_build_object('blocks', coalesce(jsonb_agg(jsonb_build_object(
    'id', id,
    'starts_at', cloister.rfc3339(starts_at),
    'ends_at', cloister.rfc3339(ends_at),
    'blocklists', to_jsonb(blocklists),
    'deep', deep)
    || case when intent is null then '{}'::jsonb else jsonb_build_object('intent', intent) end
    || case when depth_rating is null then '{}'::jsonb
            else jsonb_build_object('depth_rating', depth_rating) end
    order by starts_at, id), '[]'::jsonb))
  from public.blocks
  where user_id = (select auth.uid())
$$;

-- What the signed-in user's Devices must enforce at server time (the engine's Enforcement).
create function public.enforcement() returns jsonb
language sql stable
set search_path = ''
as $$
  with active as (
    select distinct unnest(blocklists) as list
    from public.blocks
    where user_id = (select auth.uid())
      and starts_at <= cloister.server_now() and cloister.server_now() < ends_at
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

-- Validate a command's keys: unknown keys are an error, as in every engine.
create function cloister.check_keys(cmd jsonb, required text[], optional text[]) returns void
language plpgsql
set search_path = ''
as $$
begin
  if exists (select 1 from jsonb_object_keys(cmd) k
             where k <> 'type' and k <> all (required || optional))
     or exists (select 1 from unnest(required) r where not cmd ? r) then
    perform cloister.reject('invalid_command');
  end if;
end
$$;

-- Apply a command (shared fixture JSON form) at server time for the signed-in user.
-- Returns {"outcome": "accepted" | "rejected", "reason": <code or null>}.
create function public.command(cmd jsonb) returns jsonb
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
      perform cloister.reject('invalid_command');
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

revoke execute on function public.command(jsonb) from public, anon;
grant execute on function public.command(jsonb) to authenticated;
