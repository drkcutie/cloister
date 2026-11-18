-- Hardcore Mode: an Android phone set up as Device Owner reports it, and the Devices screen
-- shows "with Hardcore Mode". Only a phone can claim it.
alter table public.devices
  add column hardcore boolean not null default false,
  add constraint hardcore_only_on_android check (not hardcore or platform = 'android');
