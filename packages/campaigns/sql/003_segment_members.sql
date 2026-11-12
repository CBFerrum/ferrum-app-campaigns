-- Who is in a segment as last worked out (a query segment's members are what campaigns and journeys read), and every
-- entry and exit since: what "entered the segment" means.
create table campaigns_segment_member (
  tenant_id uuid not null,
  segment_id uuid not null,
  contact_id uuid not null,
  since timestamptz not null default now(),
  primary key (tenant_id, segment_id, contact_id),
  foreign key (tenant_id, segment_id) references e_marketing_segment (tenant_id, id) on delete cascade,
  foreign key (tenant_id, contact_id) references e_marketing_contact (tenant_id, id) on delete cascade
);

create table campaigns_segment_change (
  tenant_id uuid not null,
  id bigserial,
  segment_id uuid not null,
  contact_id uuid not null,
  change text not null check (change in ('entered', 'left')),
  at timestamptz not null default now(),
  primary key (tenant_id, id),
  foreign key (tenant_id, segment_id) references e_marketing_segment (tenant_id, id) on delete cascade,
  foreign key (tenant_id, contact_id) references e_marketing_contact (tenant_id, id) on delete cascade
);
create index campaigns_segment_change_segment on campaigns_segment_change (tenant_id, segment_id, id);

do $$
declare t text;
begin
  foreach t in array array['campaigns_segment_member', 'campaigns_segment_change'] loop
    execute format('alter table %I enable row level security', t);
    execute format('alter table %I force row level security', t);
    execute format(
      'create policy tenant_isolation on %I using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system())) '
      'with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))', t);
  end loop;
end $$;
