//! The leaders-gap items against a real PostgreSQL (`FERRUM_CAMPAIGNS_TIER1_TEST_DATABASE_URL`, default
//! `postgres://ferrum:ferrum@localhost:5432/ferrum_campaigns_tier1_test`), reset first: topics and the preference
//! page, text and chat-app messages through the platform, journeys on record events, segments from any data,
//! revenue, deliverability and the ready journeys.

use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings, ListQuery, SiteRequest, SiteResponse};
use ferrum_meta::Catalog;
use hmac::{Hmac, Mac};
use serde_json::{Map, Value, json};
use sha2::Sha256;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use uuid::Uuid;

const PASSWORD: &str = "Correct-Horse-9";

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

fn uid(d: &Doc) -> Uuid {
    d["id"].as_str().unwrap_or_else(|| panic!("no id in {d:?}")).parse().unwrap()
}

fn refused<T: std::fmt::Debug>(r: Result<T, Error>) {
    assert!(matches!(r, Err(Error::Invalid(_) | Error::Validation(_) | Error::Forbidden(_))), "expected a refusal, got {r:?}");
}

/// An SMTP server that accepts everything (the Mail app delivers to it).
#[derive(Clone, Default)]
struct Smtp(Arc<Mutex<usize>>);

impl Smtp {
    async fn start(&self) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let me = self.clone();
        tokio::spawn(async move {
            while let Ok((sock, _)) = listener.accept().await {
                let st = me.clone();
                tokio::spawn(async move {
                    let _ = st.session(sock).await;
                });
            }
        });
        port
    }

    async fn session(&self, sock: tokio::net::TcpStream) -> std::io::Result<()> {
        let (r, mut w) = sock.into_split();
        let mut r = BufReader::new(r);
        w.write_all(b"220 sink.test ESMTP\r\n").await?;
        loop {
            let mut line = String::new();
            if r.read_line(&mut line).await? == 0 {
                return Ok(());
            }
            let up = line.trim_end().to_ascii_uppercase();
            let reply: &[u8] = if up.starts_with("EHLO") || up.starts_with("HELO") {
                b"250-sink.test\r\n250 8BITMIME\r\n"
            } else if up == "DATA" {
                w.write_all(b"354 go\r\n").await?;
                loop {
                    let mut l = String::new();
                    if r.read_line(&mut l).await? == 0 || l == ".\r\n" {
                        break;
                    }
                }
                *self.0.lock().unwrap() += 1;
                b"250 2.0.0 Queued\r\n"
            } else if up == "QUIT" {
                w.write_all(b"221 Bye\r\n").await?;
                return Ok(());
            } else {
                b"250 OK\r\n"
            };
            w.write_all(reply).await?;
        }
    }
}

struct T {
    k: Arc<Kernel>,
    tenant: Uuid,
    admin: Ctx,
}

#[allow(dead_code)]
impl T {
    async fn insert(&self, ctx: &Ctx, entity: &str, v: Value) -> Doc {
        self.k.insert(ctx, entity, obj(v)).await.unwrap_or_else(|e| panic!("insert {entity}: {e:?}"))
    }

    async fn update(&self, ctx: &Ctx, entity: &str, id: Uuid, v: Value) -> Doc {
        self.k.update(ctx, entity, id, obj(v), None).await.unwrap_or_else(|e| panic!("update {entity}: {e:?}"))
    }

    async fn get(&self, entity: &str, id: Uuid) -> Doc {
        self.k.get(&self.admin, entity, id, false).await.unwrap_or_else(|e| panic!("get {entity}: {e:?}"))
    }

    async fn action(&self, ctx: &Ctx, entity: &str, id: Uuid, action: &str, params: Value) -> Value {
        self.k.run_action(ctx, entity, id, action, params).await.unwrap_or_else(|e| panic!("{entity}.{action}: {e:?}"))
    }

    async fn list(&self, entity: &str, filters: Value) -> Vec<Doc> {
        self.list_as(&self.admin, entity, filters).await
    }

    async fn list_as(&self, ctx: &Ctx, entity: &str, filters: Value) -> Vec<Doc> {
        let q = ListQuery { filters: ferrum_kernel::query::parse_filters(&filters).unwrap(), limit: Some(500), ..Default::default() };
        self.k.list(ctx, entity, &q).await.unwrap().rows
    }

    async fn scalar<O: Send + Unpin + for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>>(&self, sql: &str, id: Option<Uuid>) -> O {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        let v: O = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
            .bind(self.tenant)
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
        tx.commit().await.unwrap();
        v
    }

    async fn exec(&self, sql: &str, id: Option<Uuid>) {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        sqlx::query(sqlx::AssertSqlSafe(sql.to_string())).bind(self.tenant).bind(id).execute(&mut *tx).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
        tx.commit().await.unwrap();
    }

    async fn maint(&self) {
        ferrum_app_campaigns::maintenance(&self.k).await.expect("maintenance");
    }

    /// Delivers what the outbox holds: mail, and record events to this app's sink.
    async fn drain(&self) {
        while self.k.dispatch_outbox_once().await.expect("dispatch") > 0 {}
    }

    async fn site(&self, method: &str, path: &str, body: &str) -> SiteResponse {
        let mut h = Map::new();
        h.insert("content-type".into(), json!("application/x-www-form-urlencoded"));
        h.insert("user-agent".into(), json!("Mozilla/5.0 (Macintosh) Test"));
        let req = SiteRequest {
            tenant: self.tenant,
            user: None,
            method: method.into(),
            path: path.into(),
            query: Map::new(),
            headers: h,
            body: body.as_bytes().to_vec(),
            client_ip: "198.51.100.7".into(),
        };
        self.k.hooks.site("campaigns").expect("campaigns site").handle(&self.k, req).await.unwrap_or_else(|e| panic!("{method} {path}: {e:?}"))
    }

    async fn key(&self) -> Vec<u8> {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        let sid: Uuid = sqlx::query_scalar("select id from e_campaign_settings where tenant_id = $1").bind(self.tenant).fetch_one(&mut *tx).await.unwrap();
        let key = self.k.secret(&mut tx, self.tenant, "CampaignSettings", sid, "signing_key").await.unwrap().expect("signing key");
        tx.commit().await.unwrap();
        hex::decode(key).unwrap()
    }

    /// The signed unsubscribe path of a recipient (as its email carries it).
    async fn unsubscribe_path(&self, recipient: Uuid) -> String {
        let key = self.key().await;
        let mut mac = Hmac::<Sha256>::new_from_slice(&key).unwrap();
        mac.update(format!("unsubscribe:{recipient}").as_bytes());
        format!("/u/{}.{}", recipient.simple(), hex::encode(&mac.finalize().into_bytes()[..16]))
    }

    /// A contact's recipient row of a campaign.
    async fn recipient_of(&self, campaign: Uuid, email: &str) -> Uuid {
        let rows = self.list("CampaignRecipient", json!({"campaign": campaign.to_string(), "email": email})).await;
        assert_eq!(rows.len(), 1, "{email} received {campaign}");
        uid(&rows[0])
    }

    /// Sends a campaign now and returns the addresses it went to.
    async fn send(&self, ctx: &Ctx, v: Value) -> (Uuid, Vec<String>) {
        let c = uid(&self.insert(ctx, "EmailCampaign", v).await);
        self.k.submit(ctx, "EmailCampaign", c, None).await.unwrap_or_else(|e| panic!("submit: {e:?}"));
        self.maint().await;
        let mut to: Vec<String> = self
            .list("CampaignRecipient", json!({"campaign": c.to_string()}))
            .await
            .iter()
            .filter(|r| r["status"] == json!("Queued") || r["status"] == json!("Sent"))
            .map(|r| r["email"].as_str().unwrap().to_string())
            .collect();
        to.sort();
        (c, to)
    }

    async fn contact(&self, email: &str) -> Doc {
        let found = self.list("MarketingContact", json!({"email": email})).await;
        assert_eq!(found.len(), 1, "contact {email}");
        found[0].clone()
    }
}

fn body(r: &SiteResponse) -> String {
    String::from_utf8(r.body.clone()).unwrap()
}

#[test]
fn leaders_gap_items() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    #[rustfmt::skip]
    let url = std::env::var("FERRUM_CAMPAIGNS_TIER1_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_campaigns_tier1_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 20, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_comms::packages().expect("mail packages"));
    packages.extend(ferrum_app_erp::packages().expect("ERP's packages"));
    packages.extend(ferrum_app_crm::packages().expect("crm packages"));
    packages.extend(ferrum_app_campaigns::packages().expect("app packages"));
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_comms::register(&mut hooks);
    ferrum_app_erp::register(&mut hooks);
    ferrum_app_crm::register(&mut hooks);
    ferrum_app_campaigns::register(&mut hooks);
    for source in ferrum_app_campaigns::reports() {
        ferrum_domain::reports::add_source(source);
    }
    let settings = KernelSettings { secret_key: Some(ferrum_kernel::secret::SecretKey::new([9u8; 32])), ..Default::default() };
    let k = Kernel::boot(db, catalog, hooks, settings, None).await.expect("boot");
    ferrum_domain::maintenance(&k).await.expect("partitions");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "acme".into(),
            name: "Acme Ltd".into(),
            admin_email: "admin@acme.test".into(),
            admin_password: PASSWORD.into(),
            admin_name: None,
            packages: vec!["core".into(), "comms".into(), "campaigns".into(), "selling".into()],
            settings: Some(json!({"timezone": "UTC"})),
        })
        .await
        .expect("tenant");
    let session = k.login("acme", "admin@acme.test", PASSWORD).await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("admin ctx");
    let mgr_id = k.create_user(&admin, "mgr@acme.test", "Mia", Some(PASSWORD), &["Marketing Manager".to_string()], false).await.expect("user");
    let mgr = k.load_ctx(tenant, mgr_id, None, "it".into()).await.unwrap();
    let t = T { k: k.clone(), tenant, admin: admin.clone() };
    let smtp = Smtp::default();
    let port = smtp.start().await;
    t.insert(&admin, "CommsSettings", json!({"newsletter_hourly_limit": 10000, "public_url": "https://erp.acme.test/"})).await;
    t.insert(
        &admin,
        "EmailAccount",
        json!({"account_name": "Marketing", "email_address": "news@acme.test", "default_outgoing": true, "smtp_host": "127.0.0.1", "smtp_port": port,
               "smtp_security": "None"}),
    )
    .await;
    t.insert(&mgr, "CampaignSettings", json!({"postal_address": "Acme Ltd, 1 Main Street"})).await;
    // Contacts from before topics existed.
    for (email, first) in [("asha@x.test", "Asha"), ("ben@x.test", "Ben"), ("cy@x.test", "Cy"), ("dee@x.test", "Dee")] {
        t.insert(&mgr, "MarketingContact", json!({"email": email, "first_name": first})).await;
    }
    let (asha, ben, cy, dee) =
        (uid(&t.contact("asha@x.test").await), uid(&t.contact("ben@x.test").await), uid(&t.contact("cy@x.test").await), uid(&t.contact("dee@x.test").await));
    let everyone = uid(&t.insert(&mgr, "MarketingSegment", json!({"segment_name": "Everyone", "match_type": "All", "rules": []})).await);
    let audience = json!([{"segment": everyone.to_string()}]);

    topics(&t, &mgr, &admin, &audience, [asha, ben, cy, dee]).await;
    messages(&t, &mgr, &admin, &audience, [asha, ben]).await;
    let (admin, mgr) = journeys(&t, tenant, mgr_id, [asha, ben]).await;
    segments(&t, &admin, &mgr, ben).await;
    revenue(&t, &admin, &mgr, ben).await;
}

/// A recipient's signed click path for link `n` of their email.
async fn click_path(t: &T, recipient: Uuid, n: i32) -> String {
    let key = t.key().await;
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).unwrap();
    mac.update(format!("click:{recipient}:{n}").as_bytes());
    format!("/c/{}.{n}.{}", recipient.simple(), hex::encode(&mac.finalize().into_bytes()[..16]))
}

/// Item 5: orders credited to the last email touch within the window, once; tracking parameters on links.
async fn revenue(t: &T, admin: &Ctx, mgr: &Ctx, ben: Uuid) {
    let ben_doc = t.get("MarketingContact", ben).await;
    let customer: Uuid = ben_doc["record_id"].as_str().unwrap().parse().unwrap();
    let only_ben = uid(&t
        .insert(
            mgr,
            "MarketingSegment",
            json!({"segment_name": "Ben", "rules": [{"rule_type": "Contact Field", "field": "Email", "operator": "Equals", "value": "ben@x.test"}]}),
        )
        .await);
    let campaign = |name: &str| {
        json!({"campaign_name": name, "subject": "Sale", "content": "<p><a href=\"https://shop.test/sale?ref=1#top\">Shop</a></p>",
               "audience": [{"segment": only_ben.to_string()}], "utm_tags": true})
    };
    let (first, _) = t.send(mgr, campaign("Spring sale")).await;
    let (second, _) = t.send(mgr, campaign("Last chance")).await;
    let r1 = t.recipient_of(first, "ben@x.test").await;
    let go = t.site("GET", &click_path(t, r1, 1).await, "").await;
    let location = go.headers.iter().find(|(k, _)| k == "location").unwrap().1.clone();
    assert_eq!(location, "https://shop.test/sale?ref=1&utm_campaign=Spring+sale&utm_source=newsletter&utm_medium=email#top");
    assert_eq!(
        t.scalar::<Value>("select utm from campaigns_click where tenant_id = $1 and recipient_id = $2", Some(r1)).await,
        json!({"utm_campaign": "Spring sale", "utm_source": "newsletter", "utm_medium": "email"}),
        "kept on the click"
    );
    let r2 = t.recipient_of(second, "ben@x.test").await;
    t.site("GET", &click_path(t, r2, 1).await, "").await;
    // Ben buys: the last touch before it takes the order, once; the earlier campaign gets nothing.
    let so = sales_order(t, admin, customer, "200").await;
    t.maint().await;
    t.maint().await;
    let credit = |c: Uuid| async move {
        let d = t.get("EmailCampaign", c).await;
        (
            d["attributed_orders"].as_i64().unwrap_or(0),
            t.scalar::<String>("select coalesce(trim_scale(sum(amount)), 0)::text from campaigns_attribution where tenant_id = $1 and owner_id = $2", Some(c))
                .await,
        )
    };
    assert_eq!(credit(second).await, (1, "150".to_string()), "the order's total as ERP priced it");
    assert_eq!(credit(first).await, (0, "0".to_string()), "no double counting");
    assert_eq!(
        t.scalar::<i64>("select count(*) from campaigns_attribution where tenant_id = $1 and $2::uuid is null", None).await,
        1,
        "his earlier order came before any touch"
    );
    let report =
        ferrum_domain::reports::run(&t.k, mgr, "revenue_attribution", &[("campaign".to_string(), second.to_string())].into_iter().collect()).await.unwrap();
    assert_eq!(
        (report["rows"][0]["link_no"].clone(), report["rows"][0]["orders"].clone(), report["totals"]["revenue"].clone()),
        (json!(1), json!(1), json!("150"))
    );
    // Outside the window, nothing is credited.
    t.update(mgr, "CampaignSettings", uid(&t.list("CampaignSettings", json!({})).await[0]), json!({"attribution_days": 1})).await;
    t.exec("update campaigns_click set clicked_at = now() - interval '3 days' where tenant_id = $1 and recipient_id = $2", Some(r2)).await;
    t.exec("delete from campaigns_attribution where tenant_id = $1 and $2::uuid is null", None).await;
    t.maint().await;
    assert_eq!(credit(second).await.0, 0);
    let _ = so;
}

/// Item 4: segments over the contact's records and over any data, worked out when used and on a schedule.
async fn segments(t: &T, admin: &Ctx, mgr: &Ctx, ben: Uuid) {
    let seg = |name: &str, rules: Value| json!({"segment_name": name, "match_type": "All", "rules": rules});
    let count = |id: Uuid| async move {
        let p = t.action(mgr, "MarketingSegment", id, "preview", json!({})).await;
        let mut emails: Vec<String> = p["sample"].as_array().unwrap().iter().map(|c| c["email"].as_str().unwrap().to_string()).collect();
        emails.sort();
        (p["contact_count"].as_i64().unwrap(), p["mailable_count"].as_i64().unwrap(), emails)
    };
    // Orders in a period, summed, through the customer each contact came from.
    let big = uid(&t
        .insert(
            mgr,
            "MarketingSegment",
            seg(
                "Spent over 100 this month",
                json!([{"rule_type": "Related Records", "entity_name": "SalesOrder", "link_field": "customer",
            "aggregate": "Sum", "measure_field": "grand_total", "date_field": "posting_date", "within_days": 30, "operator": "Greater Than", "value": "100"}]),
            ),
        )
        .await);
    assert_eq!(count(big).await, (1, 1, vec!["ben@x.test".to_string()]));
    let recent = uid(&t
        .insert(
            mgr,
            "MarketingSegment",
            seg(
                "Bought lately",
                json!([{"rule_type": "Related Records", "entity_name": "SalesOrder", "link_field": "customer",
            "aggregate": "Latest", "measure_field": "posting_date", "operator": "Within Last Days", "value": "7"}]),
            ),
        )
        .await);
    assert_eq!(count(recent).await.0, 1);
    // A count with a condition, through an email field of the workspace's own entity.
    let open_carts = uid(&t
        .insert(
            mgr,
            "MarketingSegment",
            seg(
                "Open carts",
                json!([{"rule_type": "Related Records", "entity_name": "XCart", "link_field": "email",
            "match_field": "status", "match_value": "Open", "operator": "Greater Than", "value": "0"}]),
            ),
        )
        .await);
    assert_eq!(count(open_carts).await.2, ["asha@x.test", "ben@x.test"]);
    // A field of the record the contact came from, and one of the contact itself.
    let named = uid(&t
        .insert(
            mgr,
            "MarketingSegment",
            seg(
                "Named Ben",
                json!([{"rule_type": "Record Field", "entity_name": "Customer", "field_name": "customer_name",
            "operator": "Equals", "value": "ben ode"}, {"rule_type": "Record Field", "entity_name": "MarketingContact", "field_name": "phone",
            "operator": "Starts With", "value": "+1555"}]),
            ),
        )
        .await);
    assert_eq!(count(named).await, (1, 1, vec!["ben@x.test".to_string()]));
    for bad in [
        json!([{"rule_type": "Record Field", "entity_name": "Customer", "field_name": "nope", "operator": "Equals", "value": "x"}]),
        json!([{"rule_type": "Related Records", "entity_name": "SalesOrder", "link_field": "posting_date", "operator": "Greater Than", "value": "0"}]),
        json!([{"rule_type": "Related Records", "entity_name": "SalesOrder", "link_field": "customer", "aggregate": "Sum", "measure_field": "customer",
                "operator": "Greater Than", "value": "0"}]),
        json!([{"rule_type": "Related Records", "entity_name": "SalesOrder", "link_field": "customer", "match_field": "status",
                "match_value": "x'); drop table e_marketing_contact; --", "operator": "Greater Than", "value": "0"}]),
    ] {
        let r = t.k.insert(mgr, "MarketingSegment", obj(seg(&format!("Bad {}", Uuid::now_v7()), bad))).await;
        if let Ok(d) = r {
            assert_eq!(count(uid(&d)).await.0, 0, "a value is bound, never SQL");
        }
    }

    // A saved query: its rows' email column names the people. Who saves it must be able to read those rows.
    let query = json!({"segment_name": "Open carts (query)", "source": "Query", "query": {"entity": "XCart", "filters": {"status": "Open"}},
                       "query_person_field": "email"});
    refused(t.k.insert(mgr, "MarketingSegment", obj(query.clone())).await);
    let mut grouped = query.clone();
    grouped["query"]["group_by"] = json!([{"field": "status"}]);
    refused(t.k.insert(admin, "MarketingSegment", obj(grouped)).await);
    let q = uid(&t.insert(admin, "MarketingSegment", query).await);
    assert_eq!(count(q).await, (2, 1, vec!["asha@x.test".to_string(), "ben@x.test".to_string()]), "Asha unsubscribed: counted, not mailable");
    let changes = "select count(*) from campaigns_segment_change where tenant_id = $1 and segment_id = $2 and change = ";
    assert_eq!(t.scalar::<i64>(&format!("{changes}'entered'"), Some(q)).await, 2);
    // Ben orders his lamp: on the next scheduled refresh he has left, and that is recorded.
    let lamp = uid(&t.list_as(admin, "XCart", json!({"title": "Lamp"})).await[0]);
    t.update(admin, "XCart", lamp, json!({"status": "Ordered"})).await;
    t.update(admin, "XCart", uid(&t.list_as(admin, "XCart", json!({"title": "Nothing"})).await[0]), json!({"status": "Ordered"})).await;
    t.exec("update e_marketing_segment set refreshed_at = now() - interval '2 hours' where tenant_id = $1 and id = $2", Some(q)).await;
    t.maint().await;
    assert_eq!(t.scalar::<i64>(&format!("{changes}'left'"), Some(q)).await, 1);
    assert_eq!(t.scalar::<i64>("select count(*) from campaigns_segment_member where tenant_id = $1 and segment_id = $2", Some(q)).await, 1);
    // A campaign to it reads it again when it starts.
    t.update(admin, "XCart", lamp, json!({"status": "Open"})).await;
    let (_, to) = t.send(mgr, json!({"campaign_name": "Come back", "subject": "Hi", "content": "<p>Hi</p>", "audience": [{"segment": q.to_string()}]})).await;
    assert_eq!(to, ["ben@x.test"]);
    let _ = ben;
}

/// A sales order of ERP's for `customer`, submitted.
async fn sales_order(t: &T, admin: &Ctx, customer: Uuid, rate: &str) -> Uuid {
    let item: Vec<Doc> = t.list("Item", json!({"item_code": "WIDGET"})).await;
    let item = match item.first() {
        Some(i) => uid(i),
        None => {
            let products = &t.list("ItemGroup", json!({"group_name": "Products"})).await[0];
            let nos = &t.list("Uom", json!({"uom_name": "Nos"})).await[0];
            uid(&t
                .insert(
                    admin,
                    "Item",
                    json!({"item_code": "WIDGET", "item_name": "Widget", "item_group": products["id"], "stock_uom": nos["id"], "standard_rate": "150"}),
                )
                .await)
        }
    };
    let so = uid(&t
        .insert(admin, "SalesOrder", json!({"company": t.list("Company", json!({})).await[0]["id"], "customer": customer.to_string(), "delivery_date": chrono::Utc::now().date_naive().to_string(),
            "items": [{"item": item.to_string(), "qty": "1", "rate": rate}]}))
        .await);
    t.k.submit(admin, "SalesOrder", so, None).await.unwrap_or_else(|e| panic!("submit the sales order: {e:?}"));
    so
}

/// Item 3: journeys that start, wait for, branch on and end with record events of any entity.
async fn journeys(t: &T, tenant: Uuid, mgr_id: Uuid, [asha, ben]: [Uuid; 2]) -> (Ctx, Ctx) {
    // A made-up entity of the workspace's own: no other app needed.
    let cart = json!({"name": "XCart", "label": "Cart", "kind": "master", "title_field": "title", "fields": [
        {"name": "title", "type": "data", "required": true}, {"name": "email", "type": "email"},
        {"name": "status", "type": "select", "options": ["Open", "Ordered"], "default": "Open"}, {"name": "total", "type": "int"}]});
    let custom: ferrum_kernel::customize::NewCustomization =
        serde_json::from_value(json!({"entity": "XCart", "op": {"op": "define_entity", "entity": cart}})).unwrap();
    t.k.batch_customizations(&t.admin, vec![custom], false).await.map_err(|(_, e)| e).expect("the cart entity");
    let session = t.k.login("acme", "admin@acme.test", PASSWORD).await.expect("login");
    let admin = t.k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.unwrap();
    let mgr = t.k.load_ctx(tenant, mgr_id, None, "it".into()).await.unwrap();
    let (admin, mgr) = (&admin, &mgr);
    let inr = &t.list("Currency", json!({"code": "INR"})).await[0];
    t.insert(admin, "Company", json!({"company_name": "Acme Ltd", "abbr": "AC", "default_currency": inr["id"]})).await;
    let admin = &t.k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.unwrap();
    let customer = uid(&t.insert(admin, "Customer", json!({"customer_name": "Ben Ode"})).await);
    t.update(mgr, "MarketingContact", ben, json!({"record_entity": "Customer", "record_id": customer.to_string()})).await;

    let rules = json!([
        {"label": "Cart left open", "entity_name": "XCart", "event": "Created", "condition": "doc.status == 'Open' && doc.total > 0", "person_field": "email"},
        {"label": "Cart ordered", "entity_name": "XCart", "event": "Updated", "condition": "doc.status == 'Ordered' && old.status == 'Open'", "person_field": "email"},
        {"label": "Order placed", "entity_name": "SalesOrder", "event": "Submitted", "person_field": "customer"}]);
    let steps = json!([
        {"step_type": "Wait", "wait_amount": 30, "wait_unit": "Minutes"},
        {"step_type": "Branch", "branch_on": "Record Condition", "condition": "doc.status == 'Open'"},
        {"step_type": "Send Email", "subject": "You left {{ record.title }} behind", "content": "<p>Your {{ record.title }} waits</p>"},
        {"step_type": "Send Message", "channel": "Text Message", "message_template": "shop.flash_sale"},
        {"step_type": "Wait For Event", "event_rule": 2, "wait_amount": 2, "wait_unit": "Days", "next_step": 6, "else_step": 7},
        {"step_type": "Update Attribute", "attribute": "rescued", "value": "yes", "next_step": 8},
        {"step_type": "Update Attribute", "attribute": "rescued", "value": "no"},
        {"step_type": "Exit"}]);
    let journey = |rules: Value| {
        json!({"journey_name": format!("Cart rescue {}", Uuid::now_v7()), "trigger_type": "Record Event", "trigger_rule": 1,
        "goal_rule": 3, "event_rules": rules, "steps": steps})
    };
    // Rules are checked against the workspace's entities when saved.
    for bad in [
        json!([{"entity_name": "Nope", "event": "Created"}]),
        json!([{"entity_name": "XCart", "event": "Created", "condition": "doc.colour == 'red'"}]),
        json!([{"entity_name": "XCart", "event": "Created", "person_field": "title"}]),
        json!([{"entity_name": "XCart", "event": "Created", "condition": "doc.total >"}]),
    ] {
        refused(t.k.insert(mgr, "MarketingJourney", obj(journey(bad))).await);
    }
    let j = uid(&t.insert(mgr, "MarketingJourney", journey(rules)).await);
    t.action(mgr, "MarketingJourney", j, "activate", json!({})).await;
    let enrollments = |status: &'static str| async move {
        t.list("JourneyEnrollment", json!({"journey": j.to_string(), "status": status}))
            .await
            .iter()
            .map(|e| e["email"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };

    // Asha leaves a cart: she enters, once; a cart of an unknown address makes no contact, an empty one does not count.
    let cart1 = uid(&t.insert(admin, "XCart", json!({"title": "Blue shoes", "email": "Asha@X.test", "total": 90})).await);
    t.insert(admin, "XCart", json!({"title": "Hat", "email": "stranger@y.test", "total": 5})).await;
    t.insert(admin, "XCart", json!({"title": "Nothing", "email": "ben@x.test", "total": 0})).await;
    t.drain().await;
    assert_eq!(enrollments("Active").await, ["asha@x.test"]);
    assert!(t.list("MarketingContact", json!({"email": "stranger@y.test"})).await.is_empty(), "a record names a person; it does not give consent");
    let e = &t.list("JourneyEnrollment", json!({"journey": j.to_string()})).await[0];
    assert_eq!((e["trigger_entity"].clone(), e["trigger_record"].clone()), (json!("XCart"), json!(cart1.to_string())));
    t.insert(admin, "XCart", json!({"title": "Red shoes", "email": "asha@x.test", "total": 40})).await;
    t.drain().await;
    assert_eq!(t.list("JourneyEnrollment", json!({"journey": j.to_string()})).await.len(), 1, "once per person, journey and entry rule");

    // Thirty minutes later the cart is still open: one email and one text message, however many rounds run at once.
    t.maint().await;
    assert!(t.list("JourneyEnrollment", json!({"journey": j.to_string()})).await[0]["waiting"] == json!(true));
    t.exec("update e_journey_enrollment set next_run_at = now() - interval '1 second' where tenant_id = $1 and journey = $2", Some(j)).await;
    let mut rounds = Vec::new();
    for _ in 0..4 {
        let k = t.k.clone();
        rounds.push(tokio::spawn(async move { ferrum_app_campaigns::maintenance(&k).await }));
    }
    for r in rounds {
        r.await.unwrap().expect("concurrent maintenance");
    }
    let sent = t.list("CampaignRecipient", json!({"journey": j.to_string()})).await;
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(
        t.scalar::<i64>("select count(*) from e_communication where tenant_id = $1 and subject = 'You left Blue shoes behind' and $2::uuid is null", None)
            .await,
        1
    );
    let e = &t.list("JourneyEnrollment", json!({"journey": j.to_string()})).await[0];
    assert_eq!((e["current_step"].clone(), e["waiting"].clone()), (json!(5), json!(true)), "waiting for the order, two days at most");
    // She orders: the wait ends at once, on the event.
    t.update(admin, "XCart", cart1, json!({"status": "Ordered"})).await;
    t.drain().await;
    t.maint().await;
    assert_eq!(enrollments("Completed").await, ["asha@x.test"]);
    assert_eq!(t.get("MarketingContact", asha).await["attributes"]["rescued"], json!("yes"));

    // Re-entry is a setting: not within the hours it names, then again.
    t.update(mgr, "MarketingJourney", j, json!({"allow_reentry": true, "reentry_hours": 24})).await;
    t.insert(admin, "XCart", json!({"title": "Socks", "email": "asha@x.test", "total": 9})).await;
    t.drain().await;
    assert!(enrollments("Active").await.is_empty(), "she entered less than 24 hours ago");
    t.update(mgr, "MarketingJourney", j, json!({"reentry_hours": 0})).await;
    t.insert(admin, "XCart", json!({"title": "Scarf", "email": "asha@x.test", "total": 12})).await;
    t.drain().await;
    assert_eq!(enrollments("Active").await, ["asha@x.test"]);

    // Ben's cart, then his order in ERP: the goal ends his run.
    t.insert(admin, "XCart", json!({"title": "Lamp", "email": "ben@x.test", "total": 30})).await;
    t.drain().await;
    assert_eq!(enrollments("Active").await, ["asha@x.test", "ben@x.test"]);
    sales_order(t, admin, customer, "150").await;
    t.drain().await;
    assert_eq!(enrollments("Goal Reached").await, ["ben@x.test"]);

    // Someone who opts out, or is erased, leaves every journey at once.
    t.update(mgr, "MarketingContact", asha, json!({"consent_status": "Unsubscribed"})).await;
    assert!(enrollments("Active").await.is_empty());
    let eve = uid(&t.insert(mgr, "MarketingContact", json!({"email": "eve@x.test"})).await);
    t.insert(admin, "XCart", json!({"title": "Kettle", "email": "eve@x.test", "total": 20})).await;
    t.drain().await;
    assert_eq!(enrollments("Active").await, ["eve@x.test"]);
    t.k.erase_personal_data(admin, "eve@x.test", "her request").await.expect("erasure");
    assert!(enrollments("Active").await.is_empty(), "erased");
    assert_ne!(t.get("MarketingContact", eve).await["email"], json!("eve@x.test"));
    (admin.clone(), mgr.clone())
}

/// Item 2: text and chat-app messages through the platform's templates and channels.
async fn messages(t: &T, mgr: &Ctx, admin: &Ctx, audience: &Value, [asha, ben]: [Uuid; 2]) {
    t.update(mgr, "MarketingContact", asha, json!({"phone": "+15550101"})).await;
    t.update(mgr, "MarketingContact", ben, json!({"phone": "+15550102"})).await;
    let template = |key: &str, category: &str| -> ferrum_meta::MessageDef {
        serde_json::from_value(json!({"key": key, "entity": "MarketingContact", "category": category,
            "languages": {"en": {"text": {"text": "Hi {{ doc.first_name }}, 20% off today"}, "chat": {"text": "Hi {{ doc.first_name }}"}}}}))
        .unwrap()
    };
    t.k.save_message_template(admin, &template("shop.flash_sale", "promotional")).await.expect("a promotional template");
    t.k.save_message_template(admin, &template("shop.notice", "operational")).await.expect("an operational template");
    let channel: ferrum_kernel::messages::ChannelInput = serde_json::from_value(json!({"kind": "text", "name": "Test", "secrets": {"hook": "h00k"},
        "description": {"test": true, "inbound": {"auth": "token", "secret": "hook", "from_at": "/from", "text_at": "/text", "stop": ["STOP"], "start": ["START"]}}}))
    .unwrap();
    let channel = t.k.save_message_channel(admin, None, &channel).await.expect("the test channel");
    // Marketing goes out on promotional templates only: an operational one would pass every opt-out.
    let wrong = uid(&t
        .insert(mgr, "EmailCampaign", json!({"campaign_name": "Wrong", "channel": "Text Message", "message_template": "shop.notice", "audience": audience}))
        .await);
    refused(t.k.submit(mgr, "EmailCampaign", wrong, None).await);
    let missing = uid(&t
        .insert(mgr, "EmailCampaign", json!({"campaign_name": "Missing", "channel": "Text Message", "message_template": "shop.nope", "audience": audience}))
        .await);
    refused(t.k.submit(mgr, "EmailCampaign", missing, None).await);
    refused(t.k.insert(mgr, "EmailCampaign", obj(json!({"campaign_name": "No template", "channel": "Chat App", "audience": audience}))).await);

    let (sale, to) =
        t.send(mgr, json!({"campaign_name": "Flash", "channel": "Text Message", "message_template": "shop.flash_sale", "audience": audience})).await;
    assert_eq!(to, ["asha@x.test", "ben@x.test"], "the subscribed contacts with a number");
    while t.k.run_jobs_once(4).await.unwrap() > 0 {}
    t.maint().await;
    let rows = t.list("CampaignRecipient", json!({"campaign": sale.to_string(), "status": "Sent"})).await;
    assert_eq!(rows.len(), 2, "read back from the platform's delivery log");
    assert!(rows.iter().all(|r| r["channel"] == json!("Text Message") && r["delivered_at"].is_string() && r["message_send"].is_string()), "{rows:?}");
    let said: String =
        t.scalar("select content->'message'->>'text' from message_send where tenant_id = $1 and recipient = '+15550101' and $2::uuid is null", None).await;
    assert_eq!(said, "Hi Asha, 20% off today");
    // Ben replies STOP: the platform records his number's opt-out on that channel, and the next campaign skips him.
    let none = std::collections::BTreeMap::new();
    let query = std::collections::BTreeMap::from([("token".to_string(), "h00k".to_string())]);
    t.k.message_inbound(channel, &none, &query, br#"{"from": "+15550102", "text": "stop"}"#).await.expect("a reply");
    let (again, to) =
        t.send(mgr, json!({"campaign_name": "Flash 2", "channel": "Text Message", "message_template": "shop.flash_sale", "audience": audience})).await;
    assert_eq!(to, ["asha@x.test"]);
    let ben_row = t.list("CampaignRecipient", json!({"campaign": again.to_string(), "email": "ben@x.test"})).await;
    assert_eq!((ben_row[0]["status"].clone(), ben_row[0]["skip_reason"].clone()), (json!("Skipped"), json!("opted out")));
    // Email is another channel: his email marketing is untouched.
    assert_eq!(t.get("MarketingContact", ben).await["consent_status"], json!("Subscribed"));
}

/// Item 1: topics a workspace defines, the preference page, "stop everything" as the platform's one fact.
async fn topics(t: &T, mgr: &Ctx, admin: &Ctx, audience: &Value, [asha, ben, cy, _dee]: [Uuid; 4]) {
    // Without topics the unsubscribe link asks the plain question, as before.
    let (plain, to) = t.send(mgr, json!({"campaign_name": "Before topics", "subject": "Hi", "content": "<p>Hi</p>", "audience": audience})).await;
    assert_eq!(to, ["asha@x.test", "ben@x.test", "cy@x.test", "dee@x.test"]);
    let page = t.site("GET", &t.unsubscribe_path(t.recipient_of(plain, "asha@x.test").await).await, "").await;
    assert!(body(&page).contains("Stop sending marketing emails") && !body(&page).contains("stop_all"), "{}", body(&page));

    let news = uid(&t.insert(mgr, "MarketingTopic", json!({"topic_name": "News", "description": "Our monthly letter", "sort_order": 1})).await);
    let offers = uid(&t.insert(mgr, "MarketingTopic", json!({"topic_name": "Offers", "default_in": false, "sort_order": 2})).await);
    let service = uid(&t.insert(mgr, "MarketingTopic", json!({"topic_name": "Service notices", "category": "Operational", "sort_order": 3})).await);
    // Existing contacts are in the default topics; only who chose it is in Offers.
    t.insert(mgr, "TopicSubscription", json!({"contact": asha.to_string(), "topic": offers.to_string(), "status": "Subscribed"})).await;
    let ben_offers = uid(&t.insert(mgr, "TopicSubscription", json!({"contact": ben.to_string(), "topic": offers.to_string(), "status": "Unsubscribed"})).await);
    let row = t.get("TopicSubscription", ben_offers).await;
    assert_eq!((row["email"].clone(), row["method"].clone(), row["source"].clone()), (json!("ben@x.test"), json!("Staff"), json!("user: mgr@acme.test")));
    assert!(row["changed_at"].is_string(), "the evidence says when");
    refused(t.k.update(mgr, "TopicSubscription", ben_offers, obj(json!({"status": "Subscribed"})), None).await);
    let (_, to) =
        t.send(mgr, json!({"campaign_name": "Offer", "subject": "Deal", "content": "<p>Deal</p>", "audience": audience, "topic": offers.to_string()})).await;
    assert_eq!(to, ["asha@x.test"], "Offers is out by default: only Asha chose it");
    let (news_mail, to) =
        t.send(mgr, json!({"campaign_name": "News 1", "subject": "News", "content": "<p>News</p>", "audience": audience, "topic": news.to_string()})).await;
    assert_eq!(to.len(), 4, "everyone is in News by default");

    // The signed link opens the preference page: her topics as they stand, and "stop everything".
    let asha_link = t.unsubscribe_path(t.recipient_of(news_mail, "asha@x.test").await).await;
    let page = body(&t.site("GET", &asha_link, "").await);
    let checked = |id: Uuid| page.contains(&format!("name=\"t_{}\" checked", id.simple()));
    assert!(checked(news) && checked(offers) && checked(service) && !page.contains("name=\"stop_all\" checked"), "{page}");
    // She leaves News and keeps Offers, without signing in.
    let form = format!("prefs=1&t_{}=on&t_{}=on", offers.simple(), service.simple());
    assert!(body(&t.site("POST", &asha_link, &form).await).contains("saved"));
    let mine = t.list("TopicSubscription", json!({"contact": asha.to_string(), "topic": news.to_string()})).await;
    assert_eq!((mine[0]["status"].clone(), mine[0]["method"].clone()), (json!("Unsubscribed"), json!("Preference Page")));
    assert!(mine[0]["source"].as_str().unwrap().starts_with("preference page: EC-"), "{}", mine[0]["source"]);
    assert_eq!(t.get("MarketingContact", asha).await["consent_status"], json!("Subscribed"), "one topic is not everything");
    let (_, to) =
        t.send(mgr, json!({"campaign_name": "News 2", "subject": "News", "content": "<p>News</p>", "audience": audience, "topic": news.to_string()})).await;
    assert_eq!(to, ["ben@x.test", "cy@x.test", "dee@x.test"]);

    // Stop everything: the platform's opt-out, which every app reads, is the one fact.
    t.site("POST", &asha_link, &format!("prefs=1&stop_all=on&t_{}=on&t_{}=on", offers.simple(), service.simple())).await;
    assert_eq!(t.get("MarketingContact", asha).await["consent_status"], json!("Unsubscribed"));
    let withdrawn = "select not granted from contact_consent where tenant_id = $1 and email = 'asha@x.test' and channel = 'email' and $2::uuid is null";
    assert!(t.scalar::<bool>(withdrawn, None).await);
    assert!(body(&t.site("GET", &asha_link, "").await).contains("name=\"stop_all\" checked"));
    // And back, on her own word: her topics are as she left them.
    t.site("POST", &asha_link, &format!("prefs=1&t_{}=on&t_{}=on", offers.simple(), service.simple())).await;
    assert_eq!(t.get("MarketingContact", asha).await["consent_status"], json!("Subscribed"));
    assert!(!t.scalar::<bool>(withdrawn, None).await, "the platform's record says granted again");
    assert_eq!(t.scalar::<i64>("select count(*) from e_email_suppression where tenant_id = $1 and email = 'asha@x.test' and $2::uuid is null", None).await, 0);

    // An opt-out recorded elsewhere in the workspace counts here: it is the same fact.
    t.k.set_message_opt_out(admin, "dee@x.test", "email", true).await.expect("staff record an opt-out");
    let probe = uid(&t.insert(mgr, "EmailCampaign", json!({"campaign_name": "Probe", "subject": "P", "content": "<p>P</p>", "audience": audience})).await);
    let check = t.action(mgr, "EmailCampaign", probe, "audience_check", json!({})).await;
    assert_eq!((check["contacts"].clone(), check["mailable"].clone()), (json!(4), json!(3)), "{check}");
    let dee_link = t.unsubscribe_path(t.recipient_of(news_mail, "dee@x.test").await).await;
    assert!(body(&t.site("GET", &dee_link, "").await).contains("name=\"stop_all\" checked"), "her page agrees");
    let topic_check = t
        .action(
            mgr,
            "EmailCampaign",
            uid(&t
                .insert(
                    mgr,
                    "EmailCampaign",
                    json!({"campaign_name": "P2", "subject": "P", "content": "<p>P</p>", "audience": audience, "topic": offers.to_string()}),
                )
                .await),
            "audience_check",
            json!({}),
        )
        .await;
    assert_eq!((topic_check["mailable"].clone(), topic_check["left_out"]["not_in_topic"].clone()), (json!(1), json!(2)), "{topic_check}");

    // The one-click request stays "everything"; an operational topic's mail still reaches the person, until they untick it.
    let cy_link = t.unsubscribe_path(t.recipient_of(news_mail, "cy@x.test").await).await;
    t.site("POST", &cy_link, "List-Unsubscribe=One-Click").await;
    assert_eq!(t.get("MarketingContact", cy).await["consent_status"], json!("Unsubscribed"));
    let (notice, to) = t
        .send(
            mgr,
            json!({"campaign_name": "Planned downtime", "subject": "Downtime", "content": "<p>Sunday</p>", "audience": audience, "topic": service.to_string()}),
        )
        .await;
    assert_eq!(to, ["asha@x.test", "ben@x.test", "cy@x.test", "dee@x.test"], "operational: opted-out people still get service notices");
    assert_eq!(
        t.scalar::<i64>("select count(*) from e_communication where tenant_id = $1 and reference_entity = 'MarketingContact' and purpose = 'Transactional' and $2::uuid is null", None).await,
        4
    );
    t.site("POST", &t.unsubscribe_path(t.recipient_of(notice, "cy@x.test").await).await, "List-Unsubscribe=One-Click").await;
    let cy_service = t.list("TopicSubscription", json!({"contact": cy.to_string(), "topic": service.to_string()})).await;
    assert_eq!((cy_service[0]["status"].clone(), cy_service[0]["method"].clone()), (json!("Unsubscribed"), json!("Unsubscribe Link")));
    let (_, to) = t
        .send(
            mgr,
            json!({"campaign_name": "More downtime", "subject": "Downtime", "content": "<p>Monday</p>", "audience": audience, "topic": service.to_string()}),
        )
        .await;
    assert_eq!(to, ["asha@x.test", "ben@x.test", "dee@x.test"]);
    let _ = ben;

    // A signup form for a topic asks the confirmation for that topic, also of a subscribed contact.
    let form = uid(&t.insert(mgr, "SubscriptionForm", json!({"form_name": "Deals", "topic": offers.to_string()})).await);
    let r = t.site("POST", &format!("/subscribe/{form}"), "email=dee%40x.test").await;
    assert_eq!(r.status, 200);
    assert_eq!(
        t.scalar::<i64>("select count(*) from e_communication where tenant_id = $1 and subject = 'Please confirm your subscription' and recipients = 'dee@x.test' and $2::uuid is null", None).await,
        1,
        "Dee is subscribed but not to Offers: she is asked"
    );
}
