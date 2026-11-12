-- Record events a journey's event rules matched for people in it: what its Wait For Event and Branch steps read.
-- One row per journey, rule, event and person, so a repeated delivery of an event changes nothing.
create table campaigns_journey_event (
  tenant_id uuid not null,
  id bigserial,
  journey_id uuid not null,
  rule_no int not null,
  contact_id uuid not null,
  entity text not null,
  record_id uuid not null,
  event_id bigint not null,
  at timestamptz not null default now(),
  primary key (tenant_id, id),
  unique (tenant_id, journey_id, rule_no, event_id, contact_id),
  foreign key (tenant_id, journey_id) references e_marketing_journey (tenant_id, id) on delete cascade,
  foreign key (tenant_id, contact_id) references e_marketing_contact (tenant_id, id) on delete cascade
);
create index campaigns_journey_event_person on campaigns_journey_event (tenant_id, journey_id, contact_id, rule_no, at);

alter table campaigns_journey_event enable row level security;
alter table campaigns_journey_event force row level security;
create policy tenant_isolation on campaigns_journey_event using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));
