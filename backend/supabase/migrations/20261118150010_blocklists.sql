-- Blocklist edits under "tighten now, loosen tomorrow" (ADR 0001), mirroring the Rust rules
-- engine and pinned by fixtures/rules (blocklist-*, distraction-opt-out, deep-session-extras,
-- quit-social-block). Additions apply now; removals and Distraction List opt-outs are rows
-- with an effective_at that the triggers force to the next day boundary.

-- The account's time zone (IANA) for day boundaries. Setting it comes with the settings work.
alter table public.accounts add column time_zone text not null default 'UTC';

-- The first local midnight after `at` in time zone `tz` (unknown zones count as UTC).
create function cloister.next_day_boundary(tz text, at timestamptz) returns timestamptz
language plpgsql stable
set search_path = ''
as $$
declare
  z text := coalesce(tz, 'UTC');
begin
  begin
    perform at at time zone z;
  exception when others then
    z := 'UTC';
  end;
  return ((at at time zone z)::date + 1)::timestamp at time zone z;
end
$$;

create function cloister.user_time_zone(uid uuid) returns text
language sql stable
set search_path = ''
as $$ select coalesce((select time_zone from public.accounts where user_id = uid), 'UTC') $$;

-- Normalize a Blocklist entry, or null when it isn't valid (same rules as the engine).
create function cloister.normalize_entry(kind text, input text) returns text
language plpgsql immutable
set search_path = ''
as $$
declare
  v text := regexp_replace(coalesce(input, ''), '^\s+|\s+$', '', 'g');
begin
  if kind = 'domain' then
    if v !~ '^[\x01-\x7f]*$' then
      return null;
    end if;
    v := regexp_replace(lower(v), '\.$', '');
    if length(v) <= 253
       and v ~ '^([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$' then
      return v;
    end if;
    return null;
  elsif kind = 'app' then
    if char_length(v) between 1 and 200 then
      return v;
    end if;
    return null;
  end if;
  return null;
end
$$;

-- Normalize a list of entries: deduplicated, byte-order sorted; rejects any invalid one.
create function cloister.normalize_entries(kind text, inputs text[]) returns text[]
language plpgsql immutable
set search_path = ''
as $$
begin
  if exists (select 1 from unnest(inputs) as i(x) where cloister.normalize_entry(kind, x) is null) then
    perform cloister.reject('invalid_entry');
  end if;
  return array(select distinct cloister.normalize_entry(kind, x) collate "C" as e
               from unnest(inputs) as i(x) order by e);
end
$$;

create function cloister.check_blocklist(list_id text) returns void
language plpgsql stable
set search_path = ''
as $$
begin
  if not exists (select 1 from public.blocklists b where b.id = list_id) then
    perform cloister.reject('unknown_blocklist');
  end if;
end
$$;

-- Tables ----------------------------------------------------------------------------------
-- No foreign key to the catalog: cloister.load_catalog replaces it wholesale.
create table public.blocklist_entries (
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  blocklist text not null,
  kind text not null check (kind in ('app', 'domain')),
  entry text not null,
  created_at timestamptz not null default now(),
  primary key (user_id, blocklist, kind, entry)
);
create table public.blocklist_removals (
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  blocklist text not null,
  kind text not null check (kind in ('app', 'domain')),
  entry text not null,
  effective_at timestamptz not null,
  created_at timestamptz not null default now(),
  primary key (user_id, blocklist, kind, entry)
);
create table public.blocklist_opt_outs (
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  blocklist text not null,
  effective_at timestamptz not null,
  created_at timestamptz not null default now(),
  primary key (user_id, blocklist)
);

do $$
declare t text;
begin
  foreach t in array array['blocklist_entries', 'blocklist_removals', 'blocklist_opt_outs'] loop
    execute format('alter table public.%I enable row level security', t);
    execute format('create policy own_select on public.%I for select to authenticated
                    using ((select auth.uid()) = user_id)', t);
    -- Read-only to clients: edits go through public.command().
    execute format('revoke insert, update, delete, truncate on public.%I from anon, authenticated', t);
    execute format('grant select on public.%I to authenticated', t);
  end loop;
end
$$;

alter publication supabase_realtime
  add table public.blocklist_entries, public.blocklist_removals, public.blocklist_opt_outs;

-- Whether `entry` is in the user's Blocklist at `at` (built-in or added, and not removed).
create function cloister.entry_listed(uid uuid, blocklist text, kind text, entry text, at timestamptz)
returns boolean
language sql stable
set search_path = ''
as $$
  -- Positional parameters: the tables have columns with the parameters' names.
  select (exists (select 1 from public.blocklists b
                  where b.id = $2
                    and $4 = any (case $3 when 'domain' then b.domains else b.apps end))
          or exists (select 1 from public.blocklist_entries e
                     where e.user_id = $1 and e.blocklist = $2 and e.kind = $3 and e.entry = $4))
     and not exists (select 1 from public.blocklist_removals r
                     where r.user_id = $1 and r.blocklist = $2 and r.kind = $3 and r.entry = $4
                       and r.effective_at <= $5)
$$;

-- Distraction List categories in force for the user at `at` (not opted out), sorted.
create function cloister.distraction_ids_in_force(uid uuid, at timestamptz) returns text[]
language sql stable
set search_path = ''
as $$
  select array(select b.id from public.blocklists b
               where b.kind = 'distraction'
                 and not exists (select 1 from public.blocklist_opt_outs o
                                 where o.user_id = uid and o.blocklist = b.id and o.effective_at <= at)
               order by b.id collate "C")
$$;

-- Guards: they decide on every write, whoever makes it -------------------------------------

-- A user's Blocklist rows only go when the account does (see block_deleting).
create function cloister.account_gone(uid uuid) returns boolean
language sql stable
set search_path = ''
as $$ select not exists (select 1 from auth.users where id = uid) $$;

create function cloister.blocklist_entry_guard() returns trigger
language plpgsql
set search_path = ''
as $$
begin
  if tg_op = 'INSERT' then
    perform cloister.check_blocklist(new.blocklist);
    new.entry := cloister.normalize_entry(new.kind, new.entry);
    if new.entry is null then
      perform cloister.reject('invalid_entry');
    end if;
    new.created_at := cloister.server_now();
    return new;
  elsif tg_op = 'DELETE' and cloister.account_gone(old.user_id) then
    return old;
  end if;
  -- An addition is only undone by a removal, which waits until tomorrow.
  perform cloister.reject('blocklist_cannot_be_loosened');
  return null;
end
$$;
create trigger blocklist_entry_guard before insert or update or delete on public.blocklist_entries
  for each row execute function cloister.blocklist_entry_guard();

create function cloister.blocklist_removal_guard() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
begin
  if tg_op = 'INSERT' then
    perform cloister.check_blocklist(new.blocklist);
    new.entry := cloister.normalize_entry(new.kind, new.entry);
    if new.entry is null then
      perform cloister.reject('invalid_entry');
    end if;
    if not cloister.entry_listed(new.user_id, new.blocklist, new.kind, new.entry, now) then
      perform cloister.reject('unknown_entry');
    end if;
    new.effective_at := cloister.next_day_boundary(cloister.user_time_zone(new.user_id), now);
    new.created_at := now;
    return new;
  elsif tg_op = 'DELETE' then
    return old;  -- dropping a removal puts the entry back: that only tightens
  end if;
  perform cloister.reject('blocklist_cannot_be_loosened');
  return null;
end
$$;
create trigger blocklist_removal_guard before insert or update or delete on public.blocklist_removals
  for each row execute function cloister.blocklist_removal_guard();

create function cloister.blocklist_opt_out_guard() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
begin
  if tg_op = 'INSERT' then
    perform cloister.check_blocklist(new.blocklist);
    if not exists (select 1 from public.blocklists b where b.id = new.blocklist and b.kind = 'distraction') then
      perform cloister.reject('cannot_opt_out');
    end if;
    new.effective_at := cloister.next_day_boundary(cloister.user_time_zone(new.user_id), now);
    new.created_at := now;
    return new;
  elsif tg_op = 'DELETE' then
    return old;  -- opting back in only tightens
  end if;
  perform cloister.reject('blocklist_cannot_be_loosened');
  return null;
end
$$;
create trigger blocklist_opt_out_guard before insert or update or delete on public.blocklist_opt_outs
  for each row execute function cloister.blocklist_opt_out_guard();

-- Blocks: extras for one Block and the Quit-Social Block ----------------------------------
alter table public.blocks
  add column extra_domains text[] not null default '{}',
  add column extra_apps text[] not null default '{}',
  add column quit_social boolean not null default false;

-- Replaces the block_rules version: adds the extras, the opted-out categories and the
-- Quit-Social Block (30 days, Social only, confirmed: the one exception to the 24-hour cap).
create or replace function cloister.block_inserting() returns trigger
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
  length interval := new.ends_at - new.starts_at;
begin
  if new.quit_social then
    if not new.confirmed_long then
      perform cloister.reject('confirmation_required');
    end if;
    length := interval '30 days';
    new.blocklists := array['social'];
    new.deep := false;
    new.intent := null;
    new.extra_domains := '{}';
    new.extra_apps := '{}';
  else
    perform cloister.check_length(length, new.confirmed_long);
    if exists (select 1 from unnest(new.blocklists) as l(id)
               where l.id not in (select b.id from public.blocklists b)) then
      perform cloister.reject('unknown_blocklist');
    end if;
    new.extra_domains := cloister.normalize_entries('domain', new.extra_domains);
    new.extra_apps := cloister.normalize_entries('app', new.extra_apps);
    new.blocklists := array(select distinct x collate "C" as x
                            from unnest(new.blocklists || cloister.distraction_ids_in_force(new.user_id, now)) as t(x)
                            order by x);
    new.intent := nullif(regexp_replace(new.intent, '^\s+|\s+$', '', 'g'), '');
  end if;
  if exists (select 1 from public.blocks b where b.user_id = new.user_id and b.id = new.id) then
    perform cloister.reject('duplicate_block');
  end if;
  new.starts_at := now;
  new.ends_at := now + length;
  new.depth_rating := null;
  new.created_at := now;
  return new;
end
$$;

-- Extras and the Quit-Social flag never change after the start (block_updating checks the rest).
create function cloister.block_extras_fixed() returns trigger
language plpgsql
set search_path = ''
as $$
begin
  if new.extra_domains <> old.extra_domains or new.extra_apps <> old.extra_apps
     or new.quit_social <> old.quit_social then
    perform cloister.reject('block_cannot_be_changed');
  end if;
  return new;
end
$$;
create trigger block_extras_fixed before update on public.blocks
  for each row execute function cloister.block_extras_fixed();

-- Commands (dispatched by public.command; see the extensible_command migration) -------------

create function cloister.entry_kind(cmd jsonb) returns text
language plpgsql immutable
set search_path = ''
as $$
begin
  if jsonb_typeof(cmd->'kind') <> 'string' or cmd->>'kind' not in ('app', 'domain')
     or jsonb_typeof(cmd->'entry') <> 'string' or jsonb_typeof(cmd->'blocklist') <> 'string' then
    perform cloister.reject('invalid_command');
  end if;
  return cmd->>'kind';
end
$$;

create function cloister.command_add_blocklist_entry(uid uuid, cmd jsonb) returns void
language plpgsql
set search_path = ''
as $$
declare
  v_kind text;
  v_list text := cmd->>'blocklist';
  v_entry text;
begin
  perform cloister.check_keys(cmd, array['blocklist', 'kind', 'entry'], array[]::text[]);
  v_kind := cloister.entry_kind(cmd);
  perform cloister.check_blocklist(v_list);
  v_entry := cloister.normalize_entry(v_kind, cmd->>'entry');
  if v_entry is null then
    perform cloister.reject('invalid_entry');
  end if;
  delete from public.blocklist_removals r
  where r.user_id = uid and r.blocklist = v_list and r.kind = v_kind and r.entry = v_entry;
  -- Listed before any removal (built-in or added already): nothing to add.
  if not cloister.entry_listed(uid, v_list, v_kind, v_entry, '-infinity') then
    insert into public.blocklist_entries (user_id, blocklist, kind, entry)
    values (uid, v_list, v_kind, v_entry);
  end if;
end
$$;

create function cloister.command_remove_blocklist_entry(uid uuid, cmd jsonb) returns void
language plpgsql
set search_path = ''
as $$
declare
  v_kind text;
  v_list text := cmd->>'blocklist';
  v_entry text;
begin
  perform cloister.check_keys(cmd, array['blocklist', 'kind', 'entry'], array[]::text[]);
  v_kind := cloister.entry_kind(cmd);
  perform cloister.check_blocklist(v_list);
  v_entry := cloister.normalize_entry(v_kind, cmd->>'entry');
  if v_entry is null then
    perform cloister.reject('invalid_entry');
  end if;
  if not cloister.entry_listed(uid, v_list, v_kind, v_entry, cloister.server_now()) then
    perform cloister.reject('unknown_entry');
  end if;
  insert into public.blocklist_removals (user_id, blocklist, kind, entry, effective_at)
  values (uid, v_list, v_kind, v_entry, 'infinity')
  on conflict do nothing;  -- already pending: its date stays
end
$$;

create function cloister.command_opt_out_of_blocklist(uid uuid, cmd jsonb) returns void
language plpgsql
set search_path = ''
as $$
begin
  perform cloister.check_keys(cmd, array['blocklist'], array[]::text[]);
  if jsonb_typeof(cmd->'blocklist') <> 'string' then
    perform cloister.reject('invalid_command');
  end if;
  perform cloister.check_blocklist(cmd->>'blocklist');
  if not exists (select 1 from public.blocklists b where b.id = cmd->>'blocklist' and b.kind = 'distraction') then
    perform cloister.reject('cannot_opt_out');
  end if;
  insert into public.blocklist_opt_outs (user_id, blocklist, effective_at)
  values (uid, cmd->>'blocklist', 'infinity')
  on conflict do nothing;
end
$$;

create function cloister.command_opt_in_to_blocklist(uid uuid, cmd jsonb) returns void
language plpgsql
set search_path = ''
as $$
begin
  perform cloister.check_keys(cmd, array['blocklist'], array[]::text[]);
  if jsonb_typeof(cmd->'blocklist') <> 'string' then
    perform cloister.reject('invalid_command');
  end if;
  perform cloister.check_blocklist(cmd->>'blocklist');
  delete from public.blocklist_opt_outs o where o.user_id = uid and o.blocklist = cmd->>'blocklist';
end
$$;

create function cloister.command_start_quit_social_block(uid uuid, cmd jsonb) returns void
language plpgsql
set search_path = ''
as $$
declare
  now timestamptz := cloister.server_now();
begin
  perform cloister.check_keys(cmd, array['block_id'], array['confirm_long']);
  if jsonb_typeof(cmd->'block_id') <> 'string' then
    perform cloister.reject('invalid_command');
  end if;
  insert into public.blocks (user_id, id, starts_at, ends_at, blocklists, deep, confirmed_long, quit_social)
  values (uid, cmd->>'block_id', now, now + interval '30 days', array['social'], false,
          coalesce((cmd->>'confirm_long')::boolean, false), true);
end
$$;

-- State and Enforcement -------------------------------------------------------------------

-- The Blocklist part of the user's state, in the engine's JSON form (empty parts omitted).
create function cloister.state_part_blocklists(uid uuid) returns jsonb
language sql stable
set search_path = ''
as $$
  select jsonb_strip_nulls(jsonb_build_object(
    'time_zone', nullif(cloister.user_time_zone(uid), 'UTC'),
    'blocklist_entries', (
      select jsonb_agg(jsonb_build_object('blocklist', blocklist, 'kind', kind, 'entry', entry)
                       order by blocklist collate "C", kind collate "C", entry collate "C")
      from public.blocklist_entries where user_id = uid),
    'blocklist_removals', (
      select jsonb_agg(jsonb_build_object('blocklist', blocklist, 'kind', kind, 'entry', entry,
                                          'effective_at', cloister.rfc3339(effective_at))
                       order by blocklist collate "C", kind collate "C", entry collate "C")
      from public.blocklist_removals where user_id = uid),
    'blocklist_opt_outs', (
      select jsonb_agg(jsonb_build_object('blocklist', blocklist,
                                          'effective_at', cloister.rfc3339(effective_at))
                       order by blocklist collate "C")
      from public.blocklist_opt_outs where user_id = uid)))
$$;

-- Replaces the block_rules version. Per active Block: its Blocklists, plus the Distraction
-- List categories in force now (except for the Quit-Social Block), plus its extras. Each
-- Blocklist counts with the account's additions and without removals already in force.
create or replace function public.enforcement() returns jsonb
language sql stable
set search_path = ''
as $$
  with me as (
    select (select auth.uid()) as uid, cloister.server_now() as now
  ), active as (
    select b.* from public.blocks b, me
    where b.user_id = me.uid and b.starts_at <= me.now and me.now < b.ends_at
  ), lists as (
    select unnest(blocklists) as list from active
    union
    select unnest(cloister.distraction_ids_in_force(me.uid, me.now)) from me
    where exists (select 1 from active where not quit_social)
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
