-- Private bookkeeping of the campaigns package (not entities, so never exposed through the
-- generic API): tracked links and clicks, the contact change log (journey triggers, audience
-- growth), journey membership snapshots, the journey step log and processed webhook events.

-- Addresses are stored lower-case (the kernel lower-cases email fields), so the unique indexes
-- on these emails are case-insensitive; the checks keep raw SQL paths honest too.
alter table e_marketing_contact add constraint campaigns_contact_email_lower check (email = lower(email));
alter table e_email_suppression add constraint campaigns_suppression_email_lower check (email = lower(email));

-- The static links of a campaign's (or journey's) content, numbered. Click links carry only the
-- number, so a click can only ever redirect to one of these URLs.
create table campaigns_link (
  tenant_id uuid not null,
  owner_id uuid not null,
  link_no int not null check (link_no > 0),
  url text not null,
  primary key (tenant_id, owner_id, link_no),
  unique (tenant_id, owner_id, url)
);

create table campaigns_click (
  tenant_id uuid not null,
  recipient_id uuid not null,
  owner_id uuid not null,
  link_no int not null,
  clicked_at timestamptz not null default now(),
  foreign key (tenant_id, recipient_id) references e_campaign_recipient (tenant_id, id) on delete cascade
);
create index campaigns_click_owner on campaigns_click (tenant_id, owner_id, link_no);
create index campaigns_click_recipient on campaigns_click (tenant_id, recipient_id);

-- Consent and attribute changes: `field` is 'consent_status' or 'attr:<key>'.
create table campaigns_contact_change (
  tenant_id uuid not null,
  id bigserial,
  contact_id uuid not null,
  field text not null,
  old_value text,
  new_value text,
  source text,
  changed_at timestamptz not null default now(),
  primary key (tenant_id, id),
  foreign key (tenant_id, contact_id) references e_marketing_contact (tenant_id, id) on delete cascade
);
create index campaigns_contact_change_field on campaigns_contact_change (tenant_id, field, id);
create index campaigns_contact_change_time on campaigns_contact_change (tenant_id, changed_at);

-- Who a journey has already seen in its email group or segment ("joined" = not seen before).
create table campaigns_journey_member (
  tenant_id uuid not null,
  journey_id uuid not null,
  contact_id uuid not null,
  seen_at timestamptz not null default now(),
  primary key (tenant_id, journey_id, contact_id),
  foreign key (tenant_id, journey_id) references e_marketing_journey (tenant_id, id) on delete cascade,
  foreign key (tenant_id, contact_id) references e_marketing_contact (tenant_id, id) on delete cascade
);

-- Every step an enrollment went through (the journey funnel).
create table campaigns_journey_log (
  tenant_id uuid not null,
  id bigserial,
  journey_id uuid not null,
  enrollment_id uuid not null,
  step int not null,
  outcome text not null,
  at timestamptz not null default now(),
  primary key (tenant_id, id),
  foreign key (tenant_id, enrollment_id) references e_journey_enrollment (tenant_id, id) on delete cascade
);
create index campaigns_journey_log_journey on campaigns_journey_log (tenant_id, journey_id, step);

-- Bounce and complaint events already applied (providers retry deliveries).
create table campaigns_webhook_event (
  tenant_id uuid not null,
  event_key text not null,
  received_at timestamptz not null default now(),
  primary key (tenant_id, event_key)
);
create index campaigns_webhook_event_time on campaigns_webhook_event (received_at);

-- Hot paths: a campaign's due recipients, delivery sync, journey work.
create index campaigns_recipient_due on e_campaign_recipient (tenant_id, campaign, status) where status in ('Pending', 'Held');
create index campaigns_recipient_queued on e_campaign_recipient (tenant_id, communication) where status = 'Queued';
create index campaigns_enrollment_due on e_journey_enrollment (tenant_id, next_run_at) where status = 'Active';
-- A contact is in a journey at most once at a time (re-entries wait until the previous run ends).
create unique index campaigns_enrollment_active on e_journey_enrollment (tenant_id, journey, contact) where status = 'Active';

do $$
declare t text;
begin
  foreach t in array array['campaigns_link', 'campaigns_click', 'campaigns_contact_change', 'campaigns_journey_member', 'campaigns_journey_log',
                           'campaigns_webhook_event'] loop
    execute format('alter table %I enable row level security', t);
    execute format('alter table %I force row level security', t);
    execute format(
      'create policy tenant_isolation on %I using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system())) '
      'with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))', t);
  end loop;
end $$;
