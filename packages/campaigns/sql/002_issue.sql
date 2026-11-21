-- A newsletter issue another app asked Campaigns to send under a topic (`send_issue`): once per topic and reference.
-- The row is the guard: it is made first, in the caller's transaction, and a repeat finds it and answers what was done.
create table campaigns_issue (
  tenant_id uuid not null,
  topic_id uuid not null,
  reference text not null,
  campaign_id uuid,
  accepted int not null default 0,
  skipped int not null default 0,
  created_at timestamptz not null default now(),
  primary key (tenant_id, topic_id, reference)
);
alter table campaigns_issue enable row level security;
alter table campaigns_issue force row level security;
create policy tenant_isolation on campaigns_issue using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));
