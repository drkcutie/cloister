-- Devices write their row only through public.device_heartbeat (B3). Before this, the owner had
-- insert/update/delete on public.devices, so a tampered client could write any row of its own
-- account (a fake "Enforcing" Device, a made-up last_seen_at, another Device deleted). Now a
-- client can only read the table; the heartbeat upserts the caller's own Device, keeps its
-- user_id, platform and created_at from the first heartbeat, and stamps last_seen_at with
-- server time (the device_seen trigger).

drop policy own_insert on public.devices;
drop policy own_update on public.devices;
drop policy own_delete on public.devices;
revoke insert, update, delete, truncate on public.devices from anon, authenticated;
grant select on public.devices to authenticated;

-- device = {"id": uuid, "name": text, "platform": "macos"|"windows"|"linux"|"android",
--           "enforcing": bool, "hardcore": bool (optional, default false)}.
-- Rejections raise CL001 with the reason: not_signed_in, invalid_command, device_not_yours.
-- The table's checks still apply (name length, platform, Hardcore Mode only on a phone).
create function public.device_heartbeat(device jsonb) returns void
language plpgsql
security definer
set search_path = ''
as $$
declare
  uid uuid := (select auth.uid());
  device_id uuid;
  written int;
begin
  if uid is null then
    perform cloister.reject('not_signed_in');
  end if;
  if jsonb_typeof(device) is distinct from 'object' then
    perform cloister.reject('invalid_command');
  end if;
  perform cloister.check_keys(device, array['id', 'name', 'platform', 'enforcing'], array['hardcore']);
  if device ? 'type'
     or jsonb_typeof(device->'id') <> 'string' or jsonb_typeof(device->'name') <> 'string'
     or jsonb_typeof(device->'platform') <> 'string' or jsonb_typeof(device->'enforcing') <> 'boolean'
     or coalesce(jsonb_typeof(device->'hardcore'), 'null') not in ('boolean', 'null') then
    perform cloister.reject('invalid_command');
  end if;
  begin
    device_id := (device->>'id')::uuid;
  exception when invalid_text_representation then
    perform cloister.reject('invalid_command');
  end;
  insert into public.devices as d (id, user_id, name, platform, enforcing, hardcore)
  values (device_id, uid, device->>'name', device->>'platform', (device->>'enforcing')::boolean,
          coalesce((device->>'hardcore')::boolean, false))
  on conflict (id) do update
    set name = excluded.name, enforcing = excluded.enforcing, hardcore = excluded.hardcore
    where d.user_id = uid;
  get diagnostics written = row_count;
  if written = 0 then
    perform cloister.reject('device_not_yours');
  end if;
end
$$;
revoke execute on function public.device_heartbeat(jsonb) from public, anon;
grant execute on function public.device_heartbeat(jsonb) to authenticated;
