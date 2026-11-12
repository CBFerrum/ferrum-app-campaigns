-- Orders credited to the last email touch before them (docs in src/attribution.rs): one row per order, so two
-- campaigns that touched the same person never both count it.
create table campaigns_attribution (
  tenant_id uuid not null,
  order_entity text not null,
  order_id uuid not null,
  recipient_id uuid not null,
  owner_id uuid not null,
  link_no int,
  contact_id uuid not null,
  amount numeric not null,
  touched_at timestamptz not null,
  ordered_at timestamptz not null,
  primary key (tenant_id, order_entity, order_id),
  foreign key (tenant_id, recipient_id) references e_campaign_recipient (tenant_id, id) on delete cascade
);
create index campaigns_attribution_owner on campaigns_attribution (tenant_id, owner_id);

-- The link's tracking parameters as the click was sent on (campaign, source, medium).
alter table campaigns_click add column utm jsonb;

alter table campaigns_attribution enable row level security;
alter table campaigns_attribution force row level security;
create policy tenant_isolation on campaigns_attribution using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));
