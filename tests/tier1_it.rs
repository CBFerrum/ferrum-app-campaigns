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
        let q = ListQuery { filters: ferrum_kernel::query::parse_filters(&filters).unwrap(), limit: Some(500), ..Default::default() };
        self.k.list(&self.admin, entity, &q).await.unwrap().rows
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
            packages: vec!["core".into(), "comms".into(), "campaigns".into()],
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
