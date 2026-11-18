-- Cloister schema: accounts, Devices, the Blocklist catalog and Blocks (incl. Deep Sessions).
-- Every user-owned row is visible only to its owner (row-level security). Blocks are written
-- only through public.command(); the triggers in the block_rules migration guard every write.

create schema if not exists cloister;  -- private helpers; not exposed through the API
grant usage on schema cloister to authenticated, anon, service_role;

-- Server time. The only clock the rules see. The database owner (the fixture runner, never an
-- API client) may pin it with the `cloister.server_time` setting.
create function cloister.server_now() returns timestamptz
language sql stable
set search_path = ''
as $$
  select case
    when session_user = 'postgres'
     and nullif(current_setting('cloister.server_time', true), '') is not null
    then current_setting('cloister.server_time', true)::timestamptz
    else now()
  end
$$;

create function public.server_now() returns timestamptz
language sql stable
set search_path = ''
as $$ select cloister.server_now() $$;

-- RFC 3339 in UTC, as the Rust engine serializes it (fraction only when there is one).
create function cloister.rfc3339(ts timestamptz) returns text
language sql immutable
set search_path = ''
as $$
  select case
    when extract(microseconds from ts)::bigint % 1000000 = 0
      then to_char(ts at time zone 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"')
    when extract(microseconds from ts)::bigint % 1000 = 0
      then to_char(ts at time zone 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"')
    else to_char(ts at time zone 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"')
  end
$$;

-- Accounts --------------------------------------------------------------------------------
create table public.accounts (
  user_id uuid primary key references auth.users (id) on delete cascade,
  email text,
  created_at timestamptz not null default now()
);
alter table public.accounts enable row level security;
create policy own_select on public.accounts for select to authenticated
  using ((select auth.uid()) = user_id);
grant select on public.accounts to authenticated;

create function cloister.create_account() returns trigger
language plpgsql security definer
set search_path = ''
as $$
begin
  insert into public.accounts (user_id, email) values (new.id, new.email)
  on conflict (user_id) do nothing;
  return new;
end
$$;
create trigger create_account after insert on auth.users
  for each row execute function cloister.create_account();

-- Devices ---------------------------------------------------------------------------------
-- A Device registers itself on sign-in (its id is generated and kept in its local mirror) and
-- reports whether it is enforcing. created_at and last_seen_at are always server time.
create table public.devices (
  id uuid primary key,
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  name text not null check (length(name) between 1 and 100),
  platform text not null check (platform in ('macos', 'windows', 'linux', 'android')),
  enforcing boolean not null default false,
  created_at timestamptz not null default now(),
  last_seen_at timestamptz not null default now()
);
create index devices_user_id on public.devices (user_id);
alter table public.devices enable row level security;
create policy own_select on public.devices for select to authenticated
  using ((select auth.uid()) = user_id);
create policy own_insert on public.devices for insert to authenticated
  with check ((select auth.uid()) = user_id);
create policy own_update on public.devices for update to authenticated
  using ((select auth.uid()) = user_id) with check ((select auth.uid()) = user_id);
create policy own_delete on public.devices for delete to authenticated
  using ((select auth.uid()) = user_id);
grant select, insert, update, delete on public.devices to authenticated;

create function cloister.device_seen() returns trigger
language plpgsql
set search_path = ''
as $$
begin
  if tg_op = 'UPDATE' then
    new.created_at := old.created_at;
    new.user_id := old.user_id;
  else
    new.created_at := cloister.server_now();
  end if;
  new.last_seen_at := cloister.server_now();
  return new;
end
$$;
create trigger device_seen before insert or update on public.devices
  for each row execute function cloister.device_seen();

-- Blocklist catalog -----------------------------------------------------------------------
-- Loaded from shared/blocklists.json (see the builtin_blocklists migration). Read-only.
create table public.blocklists (
  id text primary key,
  kind text not null check (kind in ('distraction', 'work')),
  domains text[] not null,
  apps text[] not null,
  position int not null
);
create table public.related_domains (
  domain text primary key,
  related text[] not null
);
alter table public.blocklists enable row level security;
alter table public.related_domains enable row level security;
create policy read_catalog on public.blocklists for select to authenticated, anon using (true);
create policy read_catalog on public.related_domains for select to authenticated, anon using (true);
grant select on public.blocklists, public.related_domains to authenticated, anon;

-- Replace the catalog with a document in the shared/blocklists.json format.
create function cloister.load_catalog(doc jsonb) returns void
language plpgsql
set search_path = ''
as $$
begin
  delete from public.blocklists where true;
  delete from public.related_domains where true;
  insert into public.blocklists (id, kind, domains, apps, position)
  select l->>'id', l->>'kind',
         array(select jsonb_array_elements_text(l->'domains')),
         array(select jsonb_array_elements_text(l->'apps')),
         ord::int
  from jsonb_array_elements(doc->'blocklists') with ordinality as t(l, ord);
  insert into public.related_domains (domain, related)
  select key, array(select jsonb_array_elements_text(value))
  from jsonb_each(doc->'related_domains');
end
$$;

-- The catalog in the shared/blocklists.json format.
create function public.catalog() returns jsonb
language sql stable
set search_path = ''
as $$
  select jsonb_build_object(
    'blocklists', coalesce((
      select jsonb_agg(jsonb_build_object('id', id, 'kind', kind, 'domains', to_jsonb(domains),
                                          'apps', to_jsonb(apps)) order by position)
      from public.blocklists), '[]'::jsonb),
    'related_domains', coalesce((
      select jsonb_object_agg(domain, to_jsonb(related)) from public.related_domains), '{}'::jsonb))
$$;

-- Blocks ----------------------------------------------------------------------------------
-- A Block is a Deep Session when `deep`; Intent and Depth Rating belong to Deep Sessions.
create table public.blocks (
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  id text not null check (length(id) between 1 and 64),
  starts_at timestamptz not null,
  ends_at timestamptz not null,
  blocklists text[] not null,
  deep boolean not null default true,
  -- Whether the write that set this length confirmed a Block longer than 4 hours.
  confirmed_long boolean not null default false,
  intent text check (length(intent) <= 200),
  depth_rating smallint check (depth_rating between 1 and 5),
  created_at timestamptz not null default now(),
  primary key (user_id, id),
  check (ends_at > starts_at)
);
alter table public.blocks enable row level security;
create policy own_select on public.blocks for select to authenticated
  using ((select auth.uid()) = user_id);
-- Read-only to clients: Blocks change only through public.command(), and never shrink.
revoke insert, update, delete, truncate on public.blocks from anon, authenticated;
grant select on public.blocks to authenticated;

-- Realtime pushes changes to the other Devices (each subscriber sees only its own rows).
alter publication supabase_realtime add table public.blocks, public.devices;
