//! End-to-end email marketing test against a real PostgreSQL (`FERRUM_CAMPAIGNS_TEST_DATABASE_URL`,
//! default `postgres://ferrum:ferrum@localhost:5432/ferrum_campaigns_test`). The database is reset first.
//!
//! A marketing team builds an audience (contacts, imports from a Mail email group, customers, CRM
//! contacts and leads), segments it (with injection attempts), sends a throttled campaign under
//! concurrent maintenance through the Mail app to an in-process SMTP server, tracks opens
//! (automatic ones apart) and clicks (no open redirect), takes unsubscribes, bounces and
//! complaints from the signed webhook, honours them in the next campaign, runs an A/B test to its
//! winner, signs people up with double opt-in, and runs journeys with waits, branches, goals and
//! exits — with the refusal paths and report numbers along the way.

use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings, ListQuery, SiteRequest, SiteResponse};
use ferrum_meta::Catalog;
use hmac::{Hmac, Mac};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use uuid::Uuid;

const PASSWORD: &str = "Correct-Horse-9";
const BASE: &str = "https://erp.acme.test/api/site/acme/campaigns";

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

fn uid(d: &Doc) -> Uuid {
    d["id"].as_str().unwrap_or_else(|| panic!("no id in {d:?}")).parse().unwrap()
}

/// A refusal of the expected kind.
fn refused<T: std::fmt::Debug>(r: Result<T, Error>, kind: &str) {
    let ok = matches!(
        (&r, kind),
        (Err(Error::NotFound(_)), "not found")
            | (Err(Error::Forbidden(_)), "forbidden")
            | (Err(Error::Invalid(_) | Error::Validation(_)), "invalid")
            | (Err(Error::Conflict(_)), "conflict")
            | (Err(Error::Unauthorized(_)), "unauthorized")
    );
    assert!(ok, "expected {kind}, got {r:?}");
}

// ───────────────────────────── an SMTP server that records what it receives ─────────────────────────────

#[derive(Clone, Debug)]
struct Received {
    to: Vec<String>,
    data: String,
}

#[derive(Clone, Default)]
struct Smtp(Arc<Mutex<Vec<Received>>>);

impl Smtp {
    fn mails(&self) -> Vec<Received> {
        self.0.lock().unwrap().clone()
    }

    fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }

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
        let mut to = Vec::new();
        loop {
            let mut line = String::new();
            if r.read_line(&mut line).await? == 0 {
                return Ok(());
            }
            let cmd = line.trim_end().to_string();
            let up = cmd.to_ascii_uppercase();
            let reply: &[u8] = if up.starts_with("EHLO") || up.starts_with("HELO") {
                b"250-sink.test\r\n250 8BITMIME\r\n"
            } else if up.starts_with("RCPT TO") {
                to.push(cmd[8..].trim().trim_start_matches(':').trim().trim_matches(['<', '>']).to_lowercase());
                b"250 OK\r\n"
            } else if up == "DATA" {
                w.write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n").await?;
                let mut data = String::new();
                loop {
                    let mut l = String::new();
                    if r.read_line(&mut l).await? == 0 {
                        return Ok(());
                    }
                    if l == ".\r\n" {
                        break;
                    }
                    data.push_str(l.strip_prefix('.').filter(|_| l.starts_with("..")).unwrap_or(&l));
                }
                self.0.lock().unwrap().push(Received { to: std::mem::take(&mut to), data });
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

struct Mail {
    to: String,
    subject: String,
    html: String,
    text: String,
}

fn parse(m: &Received) -> Mail {
    let p = ferrum_app_comms::inbound::parse_mime(m.data.as_bytes()).expect("a parsable mail");
    Mail { to: m.to.join(","), subject: p.subject, html: p.html.unwrap_or_default(), text: p.text.unwrap_or_default() }
}

/// The site paths of this app's links of one kind (`o`, `c` or `u`) in a text.
fn links(text: &str, kind: &str) -> Vec<String> {
    let re = regex::Regex::new(&format!(r#"{}(/{kind}/[A-Za-z0-9.]+)"#, regex::escape(BASE))).unwrap();
    re.captures_iter(text).map(|c| c[1].to_string()).collect()
}

// ───────────────────────────── harness ─────────────────────────────

struct T {
    k: Arc<Kernel>,
    tenant: Uuid,
    admin: Ctx,
    smtp: Smtp,
}

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

    async fn contact(&self, email: &str) -> Doc {
        let found = self.list("MarketingContact", json!({"email": email})).await;
        assert_eq!(found.len(), 1, "contact {email}");
        found[0].clone()
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

    /// Lets the Mail app deliver everything queued.
    async fn drain(&self) {
        while self.k.dispatch_outbox_once().await.expect("dispatch") > 0 {}
    }

    /// Several maintenance rounds at once (as a cluster with a stuck lock would run them).
    async fn concurrent_maint(&self, n: usize) {
        let mut tasks = Vec::new();
        for _ in 0..n {
            let k = self.k.clone();
            tasks.push(tokio::spawn(async move { ferrum_app_campaigns::maintenance(&k).await }));
        }
        for t in tasks {
            t.await.unwrap().expect("concurrent maintenance");
        }
    }

    async fn site(&self, method: &str, path: &str, body: Vec<u8>, headers: &[(&str, &str)], ip: &str) -> Result<SiteResponse, Error> {
        let mut h = Map::new();
        for (k, v) in headers {
            h.insert(k.to_string(), json!(v));
        }
        let (path, query) = match path.split_once('?') {
            Some((p, q)) => (p.to_string(), form_urlencoded::parse(q.as_bytes()).map(|(k, v)| (k.into_owned(), json!(v.into_owned()))).collect()),
            None => (path.to_string(), Map::new()),
        };
        let req = SiteRequest { tenant: self.tenant, user: None, method: method.into(), path, query, headers: h, body, client_ip: ip.into() };
        self.k.hooks.site("campaigns").expect("campaigns site").handle(&self.k, req).await
    }

    async fn get_page(&self, path: &str, ua: &str) -> SiteResponse {
        self.site("GET", path, vec![], &[("user-agent", ua)], "198.51.100.7").await.unwrap_or_else(|e| panic!("GET {path}: {e:?}"))
    }

    async fn post_page(&self, path: &str) -> SiteResponse {
        self.site("POST", path, vec![], &[("content-type", "application/x-www-form-urlencoded")], "203.0.113.9")
            .await
            .unwrap_or_else(|e| panic!("POST {path}: {e:?}"))
    }

    async fn report(&self, ctx: &Ctx, name: &str, params: &[(&str, String)]) -> Value {
        let p: HashMap<String, String> = params.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        ferrum_domain::reports::run(&self.k, ctx, name, &p).await.unwrap_or_else(|e| panic!("report {name}: {e:?}"))
    }

    async fn signing_key(&self) -> Vec<u8> {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        let sid: Uuid = sqlx::query_scalar("select id from e_campaign_settings where tenant_id = $1").bind(self.tenant).fetch_one(&mut *tx).await.unwrap();
        let key = self.k.secret(&mut tx, self.tenant, "CampaignSettings", sid, "signing_key").await.unwrap().expect("signing key");
        tx.commit().await.unwrap();
        hex::decode(key).unwrap()
    }

    /// Mails received for one subject (in arrival order).
    fn received(&self, subject_prefix: &str) -> Vec<Mail> {
        self.smtp.mails().iter().map(parse).filter(|m| m.subject.starts_with(subject_prefix)).collect()
    }

    async fn recipients(&self, campaign: Uuid) -> Vec<Doc> {
        self.list("CampaignRecipient", json!({"campaign": campaign.to_string()})).await
    }
}

fn tag(key: &[u8], message: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
    mac.update(message.as_bytes());
    hex::encode(&mac.finalize().into_bytes()[..16])
}

fn webhook_signature(secret: &str, t: i64, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{t}.").as_bytes());
    mac.update(body);
    format!("t={t},v1={}", hex::encode(mac.finalize().into_bytes()))
}

fn status_of(d: &Doc) -> &str {
    d["status"].as_str().unwrap_or_default()
}

fn int(v: &Value) -> i64 {
    v.as_i64().unwrap_or_else(|| panic!("not an integer: {v}"))
}

#[test]
fn campaigns_end_to_end() {
    // Same runtime shape as the server: unoptimized async hook chains need more than 2 MiB.
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_CAMPAIGNS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_campaigns_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 20, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_comms::packages().expect("mail packages"));
    packages.extend(ferrum_app_crm::packages().expect("crm packages"));
    packages.extend(ferrum_app_campaigns::packages().expect("app packages"));
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_comms::register(&mut hooks);
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
            packages: vec!["core".into(), "comms".into(), "crm".into(), "campaigns".into()],
            settings: Some(json!({"timezone": "America/New_York"})),
        })
        .await
        .expect("tenant");
    let session = k.login("acme", "admin@acme.test", PASSWORD).await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("admin ctx");
    let mut users = HashMap::new();
    for (email, roles) in [
        ("mgr@acme.test", vec!["Marketing Manager"]),
        ("uma@acme.test", vec!["Marketing User"]),
        ("ola@acme.test", vec!["Marketing User"]),
        ("sam@acme.test", vec!["Sales User"]),
    ] {
        let roles: Vec<String> = roles.into_iter().map(String::from).collect();
        let id = k.create_user(&admin, email, email, Some(PASSWORD), &roles, false).await.expect("user");
        users.insert(email.split('@').next().unwrap().to_string(), k.load_ctx(tenant, id, None, "it".into()).await.unwrap());
    }
    let (mgr, uma, ola, sam) = (&users["mgr"], &users["uma"], &users["ola"], &users["sam"]);
    let smtp = Smtp::default();
    let port = smtp.start().await;
    let t = T { k: k.clone(), tenant, admin: admin.clone(), smtp: smtp.clone() };

    // ───────────── Mail setup: an account, settings without a public URL yet ─────────────
    let comms_settings = uid(&t.insert(&admin, "CommsSettings", json!({"newsletter_hourly_limit": 1000})).await);
    t.insert(
        &admin,
        "EmailAccount",
        json!({"account_name": "Marketing", "email_address": "news@acme.test", "display_name": "Acme News", "default_outgoing": true,
               "smtp_host": "127.0.0.1", "smtp_port": port, "smtp_security": "None"}),
    )
    .await;
    let settings = t.insert(mgr, "CampaignSettings", json!({"postal_address": "Acme Ltd\n1 Main Street, Springfield", "soft_bounce_limit": 2})).await;
    let rotated = t.action(mgr, "CampaignSettings", uid(&settings), "rotate_webhook_secret", json!({})).await;
    let webhook_secret = rotated["secret"].as_str().unwrap().to_string();
    assert_eq!(webhook_secret.len(), 64);
    assert_eq!(rotated["path"], json!("/api/site/acme/campaigns/webhooks/bounces"));
    assert!(rotated["url"].is_null(), "no public URL yet");
    assert_eq!(t.get("CampaignSettings", uid(&settings)).await["webhook_secret"], json!(true), "write-only");
    refused(k.get(uma, "CampaignSettings", uid(&settings), false).await, "forbidden");

    // ───────────── contacts: one per address, consent only the person can restore ─────────────
    let people = [
        ("asha@x.test", "Asha", "Rao", json!({"city": "Pune", "score": 42, "plan": "free", "born": "1990-03-10"})),
        ("ben@x.test", "Ben", "Ode", json!({"city": "Mumbai", "score": 7, "plan": "free", "born": "1996-11-02"})),
        ("chen@x.test", "Chen", "Li", json!({"city": "pune", "score": 15, "born": "1992-05-01"})),
        ("dan@x.test", "Dan", "Fox", json!({})),
        ("eve@x.test", "Eve", "Ray", json!({})),
        ("fay@x.test", "Fay", "Kim", json!({"city": "50% off_city"})),
        ("gus@x.test", "Gus", "Lee", json!({})),
        ("hal@x.test", "Hal", "Ng", json!({})),
    ];
    for (email, first, last, attrs) in &people {
        let c = t.insert(mgr, "MarketingContact", json!({"email": email, "first_name": first, "last_name": last, "attributes": attrs})).await;
        assert_eq!((c["consent_status"].clone(), c["full_name"].clone()), (json!("Subscribed"), json!(format!("{first} {last}"))));
        assert_eq!(c["consent_source"], json!("user: mgr@acme.test"));
    }
    refused(k.insert(uma, "MarketingContact", obj(json!({"email": " ASHA@X.test "}))).await, "conflict");
    refused(k.insert(uma, "MarketingContact", obj(json!({"email": "new@x.test", "attributes": {"Bad Key": 1}}))).await, "invalid");
    refused(k.insert(uma, "MarketingContact", obj(json!({"email": "new@x.test", "attributes": {"nested": {"a": 1}}}))).await, "invalid");
    refused(k.insert(uma, "MarketingContact", obj(json!({"email": "new@x.test", "consent_status": "Complained"}))).await, "invalid");
    refused(
        k.insert(uma, "MarketingContact", obj(json!({"email": "new@x.test", "record_entity": "Customer", "record_id": Uuid::now_v7().to_string()}))).await,
        "invalid",
    );
    refused(k.insert(sam, "MarketingContact", obj(json!({"email": "sam@x.test"}))).await, "forbidden");
    // An opted-out contact stays out: people cannot opt others back in.
    let gus = uid(&t.contact("gus@x.test").await);
    let tmp = uid(&t.insert(uma, "MarketingContact", json!({"email": "tmp@x.test", "consent_status": "Unsubscribed"})).await);
    refused(k.update(uma, "MarketingContact", tmp, obj(json!({"consent_status": "Subscribed"})), None).await, "invalid");
    refused(k.update(mgr, "MarketingContact", tmp, obj(json!({"consent_status": "Subscribed"})), None).await, "invalid");
    assert_eq!(
        t.scalar::<i64>("select count(*) from e_email_suppression where tenant_id = $1 and email = 'tmp@x.test' and reason = 'Unsubscribed'", None).await,
        1
    );
    k.delete(mgr, "MarketingContact", tmp, None).await.unwrap();
    // A manual block: never mailed, whatever the consent.
    let block = t.insert(uma, "EmailSuppression", json!({"email": "GUS@x.test", "note": "asked by phone"})).await;
    assert_eq!(
        (block["email"].clone(), block["reason"].clone(), block["source"].clone()),
        (json!("gus@x.test"), json!("Manual"), json!("user: uma@acme.test"))
    );
    assert_eq!(t.get("MarketingContact", gus).await["consent_status"], json!("Subscribed"), "consent is the person's; the block is ours");

    // ───────────── imports: a Mail email group, customers, CRM contacts and leads ─────────────
    let newsletter = uid(&t.insert(mgr, "EmailGroup", json!({"group_name": "Newsletter"})).await);
    for (email, name, unsub) in [
        ("Asha@X.test", "Asha R", false),
        ("ivy@news.test", "Ivy Moss", false),
        ("jon@news.test", "Jon", false),
        ("kai@news.test", "Kai Ito", true),
        ("lou@news.test", "Lou Park Jr", false),
    ] {
        t.insert(mgr, "EmailGroupMember", json!({"email_group": newsletter.to_string(), "email": email, "member_name": name, "unsubscribed": unsub})).await;
    }
    let imp =
        uid(&t.insert(uma, "AudienceImport", json!({"import_name": "Newsletter list", "source": "Email Group", "email_group": newsletter.to_string()})).await);
    let out = t.action(uma, "AudienceImport", imp, "run_import", json!({})).await;
    assert_eq!((int(&out["created_count"]), int(&out["existing_count"]), int(&out["unsubscribed_count"]), int(&out["invalid_count"])), (4, 1, 1, 0));
    let lou = t.contact("lou@news.test").await;
    assert_eq!((lou["first_name"].clone(), lou["last_name"].clone(), lou["source"].clone()), (json!("Lou"), json!("Park Jr"), json!("Email Group")));
    assert_eq!(t.contact("kai@news.test").await["consent_status"], json!("Unsubscribed"), "an opt-out arrives as one");
    assert_eq!(t.scalar::<String>("select reason from e_email_suppression where tenant_id = $1 and email = 'kai@news.test'", None).await, "Unsubscribed");
    let again = t.action(uma, "AudienceImport", imp, "run_import", json!({})).await;
    assert_eq!((int(&again["created_count"]), int(&again["existing_count"])), (0, 5), "deduplicated on a second run");
    assert_eq!(t.get("AudienceImport", imp).await["status"], json!("Completed"));
    refused(k.run_action(ola, "AudienceImport", imp, "run_import", json!({})).await, "not found");

    let retail = t.insert(&admin, "CustomerGroup", json!({"group_name": "Retail Buyers", "is_group": false})).await;
    for (name, email, kind) in [
        ("Mia Cole", Some("mia@shop.test"), "Individual"),
        ("Noah Diaz", Some("NOAH@shop.test"), "Individual"),
        ("Globex", Some("buy@globex.test"), "Company"),
        ("No Mail", None, "Individual"),
    ] {
        t.insert(&admin, "Customer", json!({"customer_name": name, "email": email, "customer_type": kind, "customer_group": retail["id"]})).await;
    }
    let cimp = uid(&t
        .insert(mgr, "AudienceImport", json!({"import_name": "Individual customers", "source": "Customer", "filters": {"customer_type": "Individual"}}))
        .await);
    let out = t.action(mgr, "AudienceImport", cimp, "run_import", json!({})).await;
    assert_eq!((int(&out["created_count"]), int(&out["existing_count"]), int(&out["invalid_count"])), (2, 0, 0), "{out}");
    let mia = t.contact("mia@shop.test").await;
    assert_eq!((mia["record_entity"].clone(), mia["first_name"].clone(), mia["source"].clone()), (json!("Customer"), json!("Mia Cole"), json!("Customer")));
    // Users import only what they may read.
    let uimp = uid(&t.insert(uma, "AudienceImport", json!({"import_name": "Mine", "source": "Customer"})).await);
    refused(k.run_action(uma, "AudienceImport", uimp, "run_import", json!({})).await, "forbidden");
    // Unknown and unreadable fields get the same answer (filters must not reveal hidden values).
    refused(k.insert(mgr, "AudienceImport", obj(json!({"import_name": "Bad", "source": "Customer", "filters": {"no_such_field": 1}}))).await, "forbidden");
    refused(
        k.insert(mgr, "AudienceImport", obj(json!({"import_name": "Bad", "source": "Customer", "filters": [["customer_type", "~", "x"]]}))).await,
        "invalid",
    );

    let inr: Value = Value::Object(t.list("Currency", json!({"code": "INR"})).await[0].clone());
    let company = uid(&t.insert(&admin, "Company", json!({"company_name": "Acme Ltd", "abbr": "AL", "default_currency": inr["id"]})).await);
    let lia = uid(&t
        .insert(
            &admin,
            "Contact",
            json!({"first_name": "Lia", "last_name": "Ng", "emails": [{"email": "lia@crm.test", "is_primary": true}], "unsubscribed": true}),
        )
        .await);
    // Ben, already a subscribed marketing contact, asked the sales team to stop mailing him.
    let crm_ben = uid(&t
        .insert(
            &admin,
            "Contact",
            json!({"first_name": "Ben", "last_name": "Ode", "emails": [{"email": "ben@x.test", "is_primary": true}], "unsubscribed": true}),
        )
        .await);
    let crm_chen = uid(&t.insert(&admin, "Contact", json!({"first_name": "Chen", "emails": [{"email": "chen@x.test", "is_primary": true}]})).await);
    t.insert(&admin, "Lead", json!({"first_name": "Leo", "last_name": "Park", "email": "leo@crm.test", "company": company.to_string()})).await;
    // Marketing users cannot read CRM leads; the CRM app's own "Marketing Manager" role (merged
    // with ours by name) can.
    let limp = uid(&t.insert(uma, "AudienceImport", json!({"import_name": "Leads", "source": "Lead"})).await);
    refused(k.run_action(uma, "AudienceImport", limp, "run_import", json!({})).await, "forbidden");
    let out = t.action(mgr, "AudienceImport", limp, "run_import", json!({})).await;
    assert_eq!(int(&out["created_count"]), 1);
    assert_eq!(t.contact("leo@crm.test").await["record_entity"], json!("Lead"));
    let coimp = uid(&t.insert(&admin, "AudienceImport", json!({"import_name": "CRM contacts", "source": "Contact"})).await);
    let out = t.action(&admin, "AudienceImport", coimp, "run_import", json!({})).await;
    assert_eq!((int(&out["created_count"]), int(&out["existing_count"]), int(&out["unsubscribed_count"])), (1, 2, 2), "{out}");
    assert_eq!(t.contact("lia@crm.test").await["consent_status"], json!("Unsubscribed"));
    let ben = t.contact("ben@x.test").await;
    assert_eq!(
        (ben["consent_status"].clone(), ben["record_entity"].clone(), ben["record_id"].clone()),
        (json!("Unsubscribed"), json!("Contact"), json!(crm_ben.to_string()))
    );
    assert_eq!(ben["consent_source"], json!("import: CRM contacts"));
    let chen = t.contact("chen@x.test").await;
    assert_eq!(
        (chen["consent_status"].clone(), chen["record_id"].clone()),
        (json!("Subscribed"), json!(crm_chen.to_string())),
        "opt-ins never override; links do"
    );
    let _ = lia;

    // ───────────── segments: rules compiled to parameters, injection-proof ─────────────
    let seg = |name: &str, all: bool, rules: Value| json!({"segment_name": name, "match_type": if all { "All" } else { "Any" }, "rules": rules});
    let pune = uid(&t
        .insert(
            uma,
            "MarketingSegment",
            seg(
                "Pune regulars",
                true,
                json!([
        {"rule_type": "Attribute", "attribute": "city", "operator": "Equals", "value": "PUNE"},
        {"rule_type": "Attribute", "attribute": "score", "operator": "Greater Than", "value": "10"}]),
            ),
        )
        .await);
    let preview = t.action(uma, "MarketingSegment", pune, "preview", json!({})).await;
    assert_eq!((int(&preview["contact_count"]), int(&preview["mailable_count"])), (2, 2), "{preview}");
    let emails: Vec<&str> = preview["sample"].as_array().unwrap().iter().map(|r| r["email"].as_str().unwrap()).collect();
    assert_eq!(emails, vec!["asha@x.test", "chen@x.test"]);
    assert_eq!(int(&t.get("MarketingSegment", pune).await["contact_count"]), 2);
    let count = |rules: Value, all: bool| {
        let t = &t;
        async move {
            let s = uid(&t.insert(&t.admin, "MarketingSegment", seg(&format!("s{}", Uuid::now_v7()), all, rules)).await);
            int(&t.action(&t.admin, "MarketingSegment", s, "preview", json!({})).await["contact_count"])
        }
    };
    assert_eq!(count(json!([{"rule_type": "Attribute", "attribute": "born", "operator": "Before", "value": "1995-01-01"}]), true).await, 2);
    assert_eq!(count(json!([{"rule_type": "Contact Field", "field": "Created On", "operator": "Within Last Days", "value": "7"}]), true).await, 16);
    assert_eq!(
        count(
            json!([{"rule_type": "Contact Field", "field": "Email", "operator": "Ends With", "value": "@news.test"},
                            {"rule_type": "Email Group", "email_group": newsletter.to_string(), "operator": "Is Member"}]),
            true
        )
        .await,
        3,
        "kai left the group"
    );
    assert_eq!(
        count(
            json!([{"rule_type": "Attribute", "attribute": "score", "operator": "Less Than", "value": "8"},
                            {"rule_type": "Contact Field", "field": "Source", "operator": "Equals", "value": "Lead"}]),
            false
        )
        .await,
        2
    );
    // Wildcards are literal; quotes and SQL are values like any other.
    assert_eq!(count(json!([{"rule_type": "Attribute", "attribute": "city", "operator": "Contains", "value": "50% off_"}]), true).await, 1);
    assert_eq!(count(json!([{"rule_type": "Attribute", "attribute": "city", "operator": "Contains", "value": "%"}]), true).await, 1);
    assert_eq!(count(json!([{"rule_type": "Contact Field", "field": "First Name", "operator": "Equals", "value": "x' or '1'='1"}]), true).await, 0);
    assert_eq!(
        count(json!([{"rule_type": "Contact Field", "field": "Email", "operator": "Contains", "value": "'); drop table e_marketing_contact; --"}]), true).await,
        0
    );
    refused(
        k.insert(
            &admin,
            "MarketingSegment",
            obj(seg("evil key", true, json!([{"rule_type": "Attribute", "attribute": "x') or true --", "operator": "Equals", "value": "1"}]))),
        )
        .await,
        "invalid",
    );
    refused(
        k.insert(
            &admin,
            "MarketingSegment",
            obj(seg("evil field", true, json!([{"rule_type": "Contact Field", "field": "email; drop table x", "operator": "Equals", "value": "1"}]))),
        )
        .await,
        "invalid",
    );
    refused(
        k.insert(
            &admin,
            "MarketingSegment",
            obj(seg("bad op", true, json!([{"rule_type": "Email Group", "email_group": newsletter.to_string(), "operator": "Contains"}]))),
        )
        .await,
        "invalid",
    );
    assert_eq!(t.scalar::<i64>("select count(*) from e_marketing_contact where tenant_id = $1", None).await, 16, "the table is intact");

    // ───────────── campaign 1: scheduled, throttled, claimed exactly once under concurrency ─────────────
    let x_list = uid(&t
        .insert(
            uma,
            "MarketingSegment",
            seg("x.test", true, json!([{"rule_type": "Contact Field", "field": "Email", "operator": "Ends With", "value": "@x.test"}])),
        )
        .await);
    let no_hal = uid(&t
        .insert(
            uma,
            "MarketingSegment",
            seg("Hal", true, json!([{"rule_type": "Contact Field", "field": "Email", "operator": "Starts With", "value": "hal@"}])),
        )
        .await);
    let content = "<html><body><p>Hello {{ contact.first_name }},</p><p>Our <a href=\"https://shop.acme.test/sale?utm=1&amp;x=2\">spring sale</a> starts \
                   today. <a href='https://acme.test/about'>About us</a> · <a href=\"mailto:hi@acme.test\">write to us</a></p></body></html>";
    let draft = t
        .insert(
            uma,
            "EmailCampaign",
            json!({"campaign_name": "Spring sale", "subject": "Hello {{ contact.first_name }}", "preheader": "Up to 50% off for {{ contact.first_name }}",
                   "content": content, "hourly_limit": 4,
                   "audience": [{"segment": x_list.to_string()}, {"email_group": newsletter.to_string()}, {"segment": no_hal.to_string(), "exclude": true}]}),
        )
        .await;
    let c1 = uid(&draft);
    assert!(draft["number"].as_str().unwrap().starts_with("EC-"));
    assert_eq!(status_of(&draft), "Draft");
    refused(k.get(ola, "EmailCampaign", c1, false).await, "not found");
    refused(k.update(ola, "EmailCampaign", c1, obj(json!({"subject": "Mine now"})), None).await, "not found");
    refused(
        k.insert(
            uma,
            "EmailCampaign",
            obj(json!({"campaign_name": "No audience", "subject": "x", "content": "x", "audience": [{"segment": no_hal.to_string(), "exclude": true}]})),
        )
        .await,
        "invalid",
    );
    refused(
        k.insert(
            uma,
            "EmailCampaign",
            obj(json!({"campaign_name": "Broken", "subject": "x", "content": "{% for %}", "audience": [{"segment": x_list.to_string()}]})),
        )
        .await,
        "invalid",
    );
    refused(k.insert(uma, "EmailCampaign", obj(json!({"campaign_name": "Both", "subject": "x", "content": "x", "audience": [{"segment": x_list.to_string(), "email_group": newsletter.to_string()}]}))).await, "invalid");
    // Preview and test emails while drafting.
    let asha = uid(&t.contact("asha@x.test").await);
    let pv = t.action(uma, "EmailCampaign", c1, "preview", json!({"contact": asha.to_string()})).await;
    assert_eq!(pv["subject"], json!("Hello Asha"));
    assert!(pv["html"].as_str().unwrap().contains("Up to 50% off for Asha") && pv["html"].as_str().unwrap().contains("1 Main Street, Springfield"));
    let test_mail = t.action(uma, "EmailCampaign", c1, "send_test", json!({"emails": ["uma@acme.test", "boss@acme.test"]})).await;
    assert_eq!(int(&test_mail["sent"]), 2);
    t.drain().await;
    let tests = t.received("[Test] Hello Jane");
    assert_eq!(tests.len(), 2, "test emails use a stand-in contact");
    assert!(tests.iter().all(|m| links(&m.html, "o").is_empty() && links(&m.html, "c").is_empty()), "test emails are not tracked");
    refused(k.run_action(uma, "EmailCampaign", c1, "send_test", json!({"emails": "not-an-address"})).await, "invalid");
    // Scheduling: managers only, and only once links can reach this server.
    refused(k.submit(uma, "EmailCampaign", c1, None).await, "forbidden");
    refused(k.submit(mgr, "EmailCampaign", c1, None).await, "invalid");
    t.update(&admin, "CommsSettings", comms_settings, json!({"public_url": "https://erp.acme.test/"})).await;
    let scheduled = k.submit(mgr, "EmailCampaign", c1, None).await.expect("schedule");
    assert_eq!(status_of(&scheduled), "Scheduled");
    refused(k.update(mgr, "EmailCampaign", c1, obj(json!({"subject": "Changed"})), None).await, "invalid");
    assert_eq!(t.scalar::<i64>("select count(*) from campaigns_link where tenant_id = $1 and owner_id = $2", Some(c1)).await, 2, "two static links");

    t.concurrent_maint(4).await;
    let rs = t.recipients(c1).await;
    let mut addresses: Vec<String> = rs.iter().map(|r| r["email"].as_str().unwrap().to_string()).collect();
    addresses.sort();
    assert_eq!(
        addresses,
        vec!["asha@x.test", "chen@x.test", "dan@x.test", "eve@x.test", "fay@x.test", "ivy@news.test", "jon@news.test", "lou@news.test"],
        "the audience once each: no Hal (excluded), Gus (blocked), Kai (left) or Ben (opted out in the CRM)"
    );
    let queued = rs.iter().filter(|r| status_of(r) == "Queued").count();
    assert_eq!(queued, 4, "the campaign's own limit of 4 per hour holds under concurrent maintenance");
    t.maint().await;
    assert_eq!(t.recipients(c1).await.iter().filter(|r| status_of(r) == "Queued").count(), 4, "nothing more this hour");
    for _ in 0..2 {
        t.exec(
            "update e_campaign_recipient set queued_at = queued_at - interval '2 hours' where tenant_id = $1 and campaign = $2 and queued_at is not null",
            Some(c1),
        )
        .await;
        t.concurrent_maint(3).await;
    }
    let rs = t.recipients(c1).await;
    assert!(rs.iter().all(|r| status_of(r) == "Queued" && r["communication"].is_string()), "{rs:?}");
    let comms: i64 = t
        .scalar("select count(*) from e_communication c join e_campaign_recipient r on r.tenant_id = c.tenant_id and r.communication = c.id where c.tenant_id = $1 and r.campaign = $2", Some(c1))
        .await;
    let per_address: i64 = t
        .scalar("select max(n) from (select count(*) n from e_communication where tenant_id = $1 and reference_entity = 'MarketingContact' and $2::uuid is not null group by recipients) x", Some(c1))
        .await;
    assert_eq!((comms, per_address), (8, 1), "one email per recipient, never two");
    t.drain().await;
    let spring = t.received("Hello ");
    assert_eq!(spring.len(), 8);
    let mut to: Vec<&str> = spring.iter().map(|m| m.to.as_str()).collect();
    to.sort();
    to.dedup();
    assert_eq!(to.len(), 8);
    let asha_mail = spring.iter().find(|m| m.to == "asha@x.test").unwrap();
    assert_eq!(asha_mail.subject, "Hello Asha");
    assert!(asha_mail.html.contains("Up to 50% off for Asha") && asha_mail.html.contains("Acme Ltd<br>1 Main Street, Springfield"));
    assert_eq!(links(&asha_mail.html, "o").len(), 1, "one open pixel");
    assert_eq!(links(&asha_mail.html, "c").len(), 2, "both static links tracked");
    assert!(asha_mail.html.contains("mailto:hi@acme.test"), "other links stay as written");
    assert_eq!(links(&asha_mail.html, "u").len(), 1, "the unsubscribe footer");
    assert!(
        asha_mail.text.contains("Hello Asha") && links(&asha_mail.text, "u").len() == 1,
        "the plain-text part carries the unsubscribe link: {}",
        asha_mail.text
    );
    // The Mail app's verdict comes back.
    t.maint().await;
    let done = t.get("EmailCampaign", c1).await;
    assert_eq!((status_of(&done), int(&done["sent_count"]), int(&done["total_recipients"])), ("Sent", 8, 8));
    assert!(t.recipients(c1).await.iter().all(|r| status_of(r) == "Sent" && r["sent_at"].is_string()));

    // ───────────── tracking: opens (automatic ones apart), clicks without an open redirect ─────────────
    t.exec("update e_communication set sent_at = sent_at - interval '1 hour' where tenant_id = $1 and $2::uuid is not null", Some(c1)).await;
    let mail_of = |addr: &str| spring.iter().find(|m| m.to == addr).unwrap();
    let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";
    let pixel = links(&mail_of("asha@x.test").html, "o")[0].clone();
    for _ in 0..2 {
        let r = t.get_page(&pixel, iphone).await;
        assert_eq!((r.status, r.content_type.as_str(), r.body.len()), (200, "image/gif", 43));
    }
    t.get_page(&links(&mail_of("dan@x.test").html, "o")[0], "Mozilla/5.0").await;
    let forged = t.get_page(&pixel.replace(&pixel[pixel.len() - 8..pixel.len() - 4], "0000"), iphone).await;
    assert_eq!((forged.status, forged.body.len()), (200, 43), "a forged pixel looks the same and counts nothing");
    let chen_links = links(&mail_of("chen@x.test").html, "c");
    let r = t.get_page(&format!("{}?url=https://evil.test", chen_links[0]), iphone).await;
    assert_eq!(r.status, 302);
    let location = r.headers.iter().find(|(k, _)| k == "location").map(|(_, v)| v.clone());
    assert_eq!(location.as_deref(), Some("https://shop.acme.test/sale?utm=1&x=2"), "only the campaign's own URL");
    let r = t.get_page(&links(&mail_of("asha@x.test").html, "c")[1], iphone).await;
    assert_eq!(r.headers.iter().find(|(k, _)| k == "location").map(|(_, v)| v.as_str()), Some("https://acme.test/about"));
    // A validly signed link number that is not in the content goes nowhere.
    let key = t.signing_key().await;
    let chen_rid = t.recipients(c1).await.into_iter().find(|r| r["email"] == json!("chen@x.test")).map(|r| uid(&r)).unwrap();
    let rid_simple = chen_rid.simple().to_string();
    let r = t.get_page(&format!("/c/{rid_simple}.99.{}", tag(&key, &format!("click:{chen_rid}:99"))), iphone).await;
    assert_eq!(r.status, 404, "no open redirect");
    assert!(!r.headers.iter().any(|(k, _)| k == "location"));
    let r = t.get_page(&format!("/c/{rid_simple}.2.{}", tag(&key, &format!("click:{chen_rid}:1"))), iphone).await;
    assert_eq!(r.status, 404, "the tag covers the link number");
    let r = t.get_page(&format!("/c/{rid_simple}.1.{}", tag(b"another key, another workspace!!", &format!("click:{chen_rid}:1"))), iphone).await;
    assert_eq!(r.status, 404);
    let by_email: HashMap<String, Doc> = t.recipients(c1).await.into_iter().map(|r| (r["email"].as_str().unwrap().to_string(), r)).collect();
    let a = &by_email["asha@x.test"];
    assert_eq!((int(&a["open_count"]), int(&a["machine_open_count"]), int(&a["click_count"])), (2, 0, 1));
    let d = &by_email["dan@x.test"];
    assert_eq!((int(&d["open_count"]), int(&d["machine_open_count"]), d["opened_at"].is_null()), (0, 1, true), "Apple's prefetch is not an open");
    let c = &by_email["chen@x.test"];
    assert_eq!((int(&c["open_count"]), int(&c["click_count"]), c["opened_at"].is_string()), (1, 1, true), "a click proves an open");
    assert_eq!(t.scalar::<i64>("select count(*) from campaigns_click where tenant_id = $1 and owner_id = $2", Some(c1)).await, 2);

    // ───────────── unsubscribe: the signed link updates consent, suppression and Mail groups ─────────────
    let unsub = links(&mail_of("ivy@news.test").html, "u")[0].clone();
    let page = t.get_page(&unsub, iphone).await;
    assert_eq!(page.status, 200);
    assert!(String::from_utf8_lossy(&page.body).contains("<form method=\"post\">"));
    assert_eq!(t.contact("ivy@news.test").await["consent_status"], json!("Subscribed"), "a GET (link scanners) changes nothing");
    let flipped = if unsub.ends_with('0') { '1' } else { '0' };
    let bad = t.post_page(&format!("{}{flipped}", &unsub[..unsub.len() - 1])).await;
    assert_eq!(bad.status, 404);
    let done_page = t.post_page(&unsub).await;
    assert_eq!(done_page.status, 200);
    assert!(String::from_utf8_lossy(&done_page.body).contains("will not receive our marketing emails"));
    let ivy = t.contact("ivy@news.test").await;
    assert_eq!(ivy["consent_status"], json!("Unsubscribed"));
    assert!(ivy["consent_source"].as_str().unwrap().starts_with("unsubscribe link: EC-"), "{}", ivy["consent_source"]);
    assert_eq!(ivy["consent_ip_hash"].as_str().map(str::len), Some(64), "evidence without the address");
    assert_eq!(t.scalar::<String>("select reason from e_email_suppression where tenant_id = $1 and email = 'ivy@news.test'", None).await, "Unsubscribed");
    assert!(
        t.scalar::<bool>("select unsubscribed from e_email_group_member where tenant_id = $1 and email = 'ivy@news.test' and $2::uuid is not null", Some(c1))
            .await,
        "Mail newsletters honour it too"
    );
    t.post_page(&unsub).await;
    assert_eq!(
        t.scalar::<i64>(
            "select count(*) from campaigns_contact_change where tenant_id = $1 and new_value = 'Unsubscribed' and contact_id = $2",
            Some(uid(&ivy))
        )
        .await,
        1,
        "idempotent"
    );
    // Chen is linked to a CRM contact: unsubscribing sets the CRM flag as well.
    t.post_page(&links(&mail_of("chen@x.test").html, "u")[0]).await;
    assert_eq!(t.get("Contact", crm_chen).await["unsubscribed"], json!(true));

    // ───────────── bounces and complaints through the signed webhook ─────────────
    let hook = |body: Value, signature: Option<String>| {
        let t = &t;
        async move {
            let bytes = body.to_string().into_bytes();
            let mut headers = vec![("content-type", "application/json".to_string())];
            if let Some(s) = signature {
                headers.push(("x-campaigns-signature", s));
            }
            let h: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
            t.site("POST", "/webhooks/bounces", bytes, &h, "192.0.2.1").await
        }
    };
    let now = chrono::Utc::now().timestamp();
    let sign = |body: &Value, at: i64| Some(webhook_signature(&webhook_secret, at, body.to_string().as_bytes()));
    let dan_mid: String = t
        .scalar("select c.message_id from e_communication c join e_campaign_recipient r on r.tenant_id = c.tenant_id and r.communication = c.id where c.tenant_id = $1 and r.campaign = $2 and r.email = 'dan@x.test'", Some(c1))
        .await;
    let hard = json!({"type": "hard", "email": "dan@x.test", "message_id": format!("<{dan_mid}>"), "reason": "550 5.1.1 user unknown"});
    refused(hook(hard.clone(), None).await, "unauthorized");
    refused(hook(hard.clone(), Some(webhook_signature("wrong secret wrong secret wrong!", now, hard.to_string().as_bytes()))).await, "unauthorized");
    refused(hook(hard.clone(), sign(&hard, now - 600)).await, "unauthorized");
    refused(hook(json!({"type": "hard", "email": "asha@x.test"}), sign(&hard, now)).await, "unauthorized");
    let r = hook(hard.clone(), sign(&hard, now)).await.expect("signed webhook");
    assert_eq!(serde_json::from_slice::<Value>(&r.body).unwrap(), json!({"processed": 1, "duplicates": 0, "ignored": 0}));
    let dan = t.contact("dan@x.test").await;
    assert_eq!((dan["consent_status"].clone(), dan["consent_source"].as_str().unwrap().starts_with("hard bounce")), (json!("Bounced"), true));
    assert_eq!(t.scalar::<String>("select reason from e_email_suppression where tenant_id = $1 and email = 'dan@x.test'", None).await, "Hard Bounce");
    let eve_rid = t.recipients(c1).await.into_iter().find(|r| r["email"] == json!("eve@x.test")).map(|r| uid(&r)).unwrap();
    let soft1 = json!({"events": [{"id": "evt-1", "type": "bounce", "bounce_type": "Transient", "recipient": eve_rid.to_string()}]});
    hook(soft1.clone(), sign(&soft1, now)).await.unwrap();
    let r = hook(soft1.clone(), sign(&soft1, now)).await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&r.body).unwrap()["duplicates"], json!(1), "a provider retry is applied once");
    let eve = t.contact("eve@x.test").await;
    assert_eq!((eve["consent_status"].clone(), int(&eve["soft_bounce_count"])), (json!("Subscribed"), 1));
    let soft2 = json!([{"id": "evt-2", "type": "soft", "email": "EVE@x.test"}, {"type": "delivered", "email": "eve@x.test"}, {"type": "hard"}]);
    let r = hook(soft2.clone(), sign(&soft2, now)).await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&r.body).unwrap(), json!({"processed": 1, "duplicates": 0, "ignored": 2}));
    let eve = t.contact("eve@x.test").await;
    assert_eq!((eve["consent_status"].clone(), int(&eve["soft_bounce_count"])), (json!("Bounced"), 2), "suppressed at the limit");
    assert_eq!(t.scalar::<String>("select reason from e_email_suppression where tenant_id = $1 and email = 'eve@x.test'", None).await, "Soft Bounces");
    let fay_rid = t.recipients(c1).await.into_iter().find(|r| r["email"] == json!("fay@x.test")).map(|r| uid(&r)).unwrap();
    let complaint = json!({"type": "complaint", "recipient": fay_rid.to_string(), "campaign": c1.to_string()});
    hook(complaint.clone(), sign(&complaint, now)).await.unwrap();
    assert_eq!(t.contact("fay@x.test").await["consent_status"], json!("Complained"));
    assert_eq!(t.scalar::<String>("select reason from e_email_suppression where tenant_id = $1 and email = 'fay@x.test'", None).await, "Complaint");
    let stranger = json!({"type": "hard", "email": "nobody@else.test"});
    hook(stranger.clone(), sign(&stranger, now)).await.unwrap();
    assert_eq!(t.scalar::<String>("select reason from e_email_suppression where tenant_id = $1 and email = 'nobody@else.test'", None).await, "Hard Bounce");
    t.maint().await;
    let perf = t.report(mgr, "campaign_performance", &[("campaign", c1.to_string())]).await;
    let row = &perf["rows"][0];
    let numbers: Vec<i64> = [
        "sent",
        "delivered",
        "unique_opens",
        "total_opens",
        "machine_opens",
        "unique_clicks",
        "total_clicks",
        "unsubscribes",
        "bounces",
        "hard_bounces",
        "soft_bounces",
        "complaints",
    ]
    .iter()
    .map(|k| int(&row[*k]))
    .collect();
    assert_eq!(numbers, vec![8, 6, 2, 3, 1, 2, 2, 2, 2, 1, 1, 1], "{row}");
    assert_eq!(
        (
            row["open_rate"].clone(),
            row["click_rate"].clone(),
            row["click_to_open"].clone(),
            row["unsubscribe_rate"].clone(),
            row["bounce_rate"].clone(),
            row["complaint_rate"].clone()
        ),
        (json!("33.33"), json!("33.33"), json!("100.00"), json!("33.33"), json!("25.00"), json!("16.67"))
    );
    let done = t.get("EmailCampaign", c1).await;
    assert_eq!(
        (int(&done["unique_opens"]), int(&done["unique_clicks"]), int(&done["unsubscribe_count"]), int(&done["bounce_count"]), int(&done["complaint_count"])),
        (2, 2, 2, 2, 1)
    );
    let clicks = t.report(uma, "link_clicks", &[("campaign", c1.to_string())]).await;
    assert_eq!(
        clicks["rows"].as_array().unwrap().iter().map(|r| (r["url"].as_str().unwrap().to_string(), int(&r["unique_clicks"]))).collect::<Vec<_>>(),
        vec![("https://shop.acme.test/sale?utm=1&x=2".to_string(), 1), ("https://acme.test/about".to_string(), 1)]
    );
    refused(k.cancel(mgr, "EmailCampaign", c1, None).await, "invalid");

    // ───────────── campaign 2: every opt-out, bounce, complaint and block is honoured ─────────────
    let c2 = uid(&t
        .insert(
            mgr,
            "EmailCampaign",
            json!({"campaign_name": "Summer", "subject": "Summer news", "content": "<p>Hi {{ contact.first_name }}, see <a href=\"https://acme.test/summer\">summer</a>.</p>",
                   "track_opens": false, "track_clicks": false, "hourly_limit": 2,
                   "audience": [{"segment": x_list.to_string()}, {"email_group": newsletter.to_string()}, {"segment": no_hal.to_string(), "exclude": true}]}),
        )
        .await);
    k.submit(mgr, "EmailCampaign", c2, None).await.expect("schedule");
    t.maint().await;
    let mut r2: Vec<String> = t.recipients(c2).await.iter().map(|r| r["email"].as_str().unwrap().to_string()).collect();
    r2.sort();
    assert_eq!(
        r2,
        vec!["asha@x.test", "jon@news.test", "lou@news.test"],
        "Ben, Chen and Ivy unsubscribed; Dan and Eve bounced; Fay complained; Gus is blocked"
    );
    let pending: Vec<Doc> = t.recipients(c2).await.into_iter().filter(|r| status_of(r) == "Pending").collect();
    assert_eq!(pending.len(), 1);
    let late = pending[0]["email"].as_str().unwrap().to_string();
    let late_block = t.insert(mgr, "EmailSuppression", json!({"email": late, "note": "asked to stop just now"})).await;
    t.exec(
        "update e_campaign_recipient set queued_at = queued_at - interval '2 hours' where tenant_id = $1 and campaign = $2 and queued_at is not null",
        Some(c2),
    )
    .await;
    t.maint().await;
    let skipped = t.recipients(c2).await.into_iter().find(|r| r["email"] == json!(late)).unwrap();
    assert_eq!((status_of(&skipped), skipped["communication"].is_null()), ("Skipped", true));
    assert!(skipped["skip_reason"].as_str().unwrap().contains("suppressed"));
    // A manager lifts that block again (it was a mistake): later sends may mail them.
    k.delete(mgr, "EmailSuppression", uid(&late_block), None).await.unwrap();
    refused(k.delete(uma, "EmailSuppression", uid(&block), None).await, "forbidden");
    t.drain().await;
    t.maint().await;
    let summer = t.received("Summer news");
    assert_eq!(summer.len(), 2);
    assert!(summer.iter().all(|m| m.to != late && links(&m.html, "o").is_empty() && links(&m.html, "c").is_empty() && links(&m.html, "u").len() == 1));
    assert!(summer.iter().all(|m| m.html.contains("href=\"https://acme.test/summer\"")), "untracked links stay as written");
    assert_eq!(status_of(&t.get("EmailCampaign", c2).await), "Sent");

    // ───────────── campaign 3: an A/B test decided by opens ─────────────
    let mut ab_contacts = Vec::new();
    for i in 1..=20 {
        ab_contacts.push(uid(&t
            .insert(mgr, "MarketingContact", json!({"email": format!("ab{i:02}@ab.test"), "first_name": format!("P{i}"), "attributes": {"plan": "free"}}))
            .await));
    }
    let ab_seg = uid(&t
        .insert(
            mgr,
            "MarketingSegment",
            seg("ab", true, json!([{"rule_type": "Contact Field", "field": "Email", "operator": "Ends With", "value": "@ab.test"}])),
        )
        .await);
    refused(
        k.insert(
            mgr,
            "EmailCampaign",
            obj(json!({"campaign_name": "One variant", "subject": "x", "content": "x", "ab_test": true, "variants": [{"subject": "A"}],
                                                       "audience": [{"segment": ab_seg.to_string()}]})),
        )
        .await,
        "invalid",
    );
    refused(
        k.insert(
            mgr,
            "EmailCampaign",
            obj(json!({"campaign_name": "Blind", "subject": "x", "content": "x", "ab_test": true, "track_opens": false,
                                                       "variants": [{"subject": "A"}, {"subject": "B"}], "audience": [{"segment": ab_seg.to_string()}]})),
        )
        .await,
        "invalid",
    );
    let c3doc = t
        .insert(
            mgr,
            "EmailCampaign",
            json!({"campaign_name": "Subject test", "subject": "Base subject", "content": "<p>Offer for {{ contact.first_name }}</p>", "ab_test": true, "test_percent": 50,
                   "winner_metric": "Opens", "winner_after_hours": 2, "variants": [{"subject": "Subject A"}, {"subject": "Subject B"}],
                   "audience": [{"segment": ab_seg.to_string()}]}),
        )
        .await;
    let c3 = uid(&c3doc);
    assert_eq!(c3doc["variants"].as_array().unwrap().iter().map(|v| v["variant"].as_str().unwrap()).collect::<Vec<_>>(), vec!["A", "B"]);
    k.submit(mgr, "EmailCampaign", c3, None).await.expect("schedule");
    t.concurrent_maint(2).await;
    let rs = t.recipients(c3).await;
    assert_eq!(rs.len(), 20);
    let assigned: HashMap<Uuid, Option<String>> =
        rs.iter().map(|r| (r["contact"].as_str().unwrap().parse().unwrap(), r["variant"].as_str().map(str::to_string))).collect();
    // The split is a pure function of campaign and contact: rank by SHA-256(campaign ‖ contact).
    let mut ranked = ab_contacts.clone();
    ranked.sort_by_key(|c| (Sha256::digest(format!("{c3}{c}").as_bytes()).to_vec(), *c));
    for (i, c) in ranked.iter().enumerate() {
        let expected = if i < 10 { Some(if i % 2 == 0 { "A" } else { "B" }.to_string()) } else { None };
        assert_eq!(assigned[c], expected, "rank {i}");
    }
    assert_eq!(rs.iter().filter(|r| status_of(r) == "Held").count(), 10, "the rest wait for the winner");
    t.drain().await;
    t.maint().await;
    let tested = t.get("EmailCampaign", c3).await;
    assert!(tested["test_sent_at"].is_string() && tested["winner_variant"].is_null());
    t.exec("update e_communication set sent_at = sent_at - interval '1 hour' where tenant_id = $1 and $2::uuid is not null", Some(c3)).await;
    let ab_mails = t.received("Subject ");
    assert_eq!(ab_mails.len(), 10);
    let open_by = |variant: &str, n: usize| {
        let t = &t;
        let mails: Vec<&Mail> = ab_mails.iter().filter(|m| m.subject == format!("Subject {variant}")).take(n).collect();
        let pixels: Vec<String> = mails.iter().map(|m| links(&m.html, "o")[0].clone()).collect();
        async move {
            for p in pixels {
                t.get_page(&p, iphone).await;
            }
        }
    };
    open_by("A", 1).await;
    open_by("B", 3).await;
    t.maint().await;
    assert!(t.get("EmailCampaign", c3).await["winner_variant"].is_null(), "not before winner_after_hours");
    t.exec("update e_email_campaign set test_sent_at = test_sent_at - interval '3 hours' where tenant_id = $1 and id = $2", Some(c3)).await;
    t.maint().await;
    let decided = t.get("EmailCampaign", c3).await;
    assert_eq!(decided["winner_variant"], json!("B"), "3 of 5 beats 1 of 5");
    t.drain().await;
    t.maint().await;
    assert_eq!(t.received("Subject B").len(), 15, "5 in the test, 10 winners");
    assert_eq!(status_of(&t.get("EmailCampaign", c3).await), "Sent");
    let ab = t.report(mgr, "ab_test_results", &[("campaign", c3.to_string())]).await;
    let rows = ab["rows"].as_array().unwrap();
    assert_eq!(
        (rows[0]["variant"].clone(), int(&rows[0]["recipients"]), int(&rows[0]["unique_opens"]), rows[0]["winner"].clone()),
        (json!("A"), 5, 1, json!(false))
    );
    assert_eq!(
        (rows[1]["variant"].clone(), int(&rows[1]["recipients"]), int(&rows[1]["unique_opens"]), rows[1]["winner"].clone()),
        (json!("B"), 15, 3, json!(true))
    );
    assert_eq!((rows[0]["open_rate"].clone(), ab["meta"]["winner"].clone()), (json!("20.00"), json!("B")));
    // Cancelling a scheduled campaign stops it before anyone is mailed.
    let c4 = uid(&t
        .insert(
            mgr,
            "EmailCampaign",
            json!({"campaign_name": "Later", "subject": "Later", "content": "x", "schedule_at": "2099-01-01T00:00:00Z",
                                                          "audience": [{"segment": ab_seg.to_string()}]}),
        )
        .await);
    k.submit(mgr, "EmailCampaign", c4, None).await.unwrap();
    t.maint().await;
    assert_eq!(t.recipients(c4).await.len(), 0);
    refused(k.cancel(uma, "EmailCampaign", c4, None).await, "forbidden");
    let cancelled = k.cancel(mgr, "EmailCampaign", c4, None).await.unwrap();
    assert_eq!(status_of(&cancelled), "Cancelled");
    refused(k.delete(mgr, "EmailCampaign", c1, None).await, "invalid");
    k.delete(mgr, "EmailCampaign", c4, None).await.expect("a cancelled campaign can be deleted");

    // ───────────── double opt-in signup ─────────────
    let website = uid(&t.insert(mgr, "EmailGroup", json!({"group_name": "Website signups"})).await);
    let form = t
        .insert(
            mgr,
            "SubscriptionForm",
            json!({"form_name": "Website", "email_group": website.to_string(), "allowed_attributes": "city, interests",
                                                  "thank_you_url": "https://acme.test/thanks"}),
        )
        .await;
    let fid = uid(&form);
    assert_eq!(t.get("SubscriptionForm", fid).await["endpoint"], json!(format!("/api/site/acme/campaigns/subscribe/{fid}")));
    refused(k.insert(mgr, "SubscriptionForm", obj(json!({"form_name": "Bad", "allowed_attributes": "website"}))).await, "invalid");
    let post_json = |body: Value| {
        let t = &t;
        async move { t.site("POST", &format!("/subscribe/{fid}"), body.to_string().into_bytes(), &[("content-type", "application/json")], "203.0.113.9").await }
    };
    let bot = post_json(json!({"email": "bot@spam.test", "website": "http://spam.test"})).await.unwrap();
    let human = post_json(json!({"email": " Nia@Site.test ", "first_name": "Nia", "city": "Goa", "plan": "gold"})).await.unwrap();
    assert_eq!((bot.status, bot.body.clone()), (human.status, human.body.clone()), "the honeypot gets the same answer");
    assert_eq!(human.status, 202);
    assert_eq!(t.scalar::<i64>("select count(*) from e_marketing_contact where tenant_id = $1 and email = 'bot@spam.test'", None).await, 0);
    let nia = t.contact("nia@site.test").await;
    assert_eq!(
        (nia["consent_status"].clone(), nia["attributes"].clone(), nia["source"].clone()),
        (json!("Pending"), json!({"city": "Goa"}), json!("Subscription Form"))
    );
    post_json(json!({"email": "nia@site.test"})).await.unwrap();
    refused(post_json(json!({"email": "not an address"})).await, "invalid");
    refused(
        t.site("POST", &format!("/subscribe/{}", Uuid::now_v7()), b"{}".to_vec(), &[("content-type", "application/json")], "203.0.113.9").await,
        "not found",
    );
    t.drain().await;
    let confirmations = t.received("Please confirm your subscription");
    assert_eq!(confirmations.len(), 1, "one confirmation, however often the form is sent");
    assert_eq!(confirmations[0].to, "nia@site.test");
    let confirm_path =
        regex::Regex::new(&format!(r"{}(/confirm/[A-Za-z0-9.]+)", regex::escape(BASE))).unwrap().captures(&confirmations[0].html).unwrap()[1].to_string();
    let page = t.get_page(&confirm_path, iphone).await;
    assert!(page.status == 200 && String::from_utf8_lossy(&page.body).contains("Yes, subscribe me"));
    assert_eq!(t.contact("nia@site.test").await["consent_status"], json!("Pending"));
    let parts: Vec<&str> = confirm_path.trim_start_matches("/confirm/").split('.').collect();
    let forged = format!("/confirm/{}.{}.{}.{}", parts[0], parts[1], parts[2].parse::<i64>().unwrap() + 3600, parts[3]);
    assert_eq!(t.post_page(&forged).await.status, 404, "the expiry is signed");
    let (cid, form_part) = (parts[0].to_string(), parts[1].to_string());
    let past = chrono::Utc::now().timestamp() - 60;
    let nia_id = uid(&nia);
    let expired = format!("/confirm/{cid}.{form_part}.{past}.{}", tag(&key, &format!("confirm:{nia_id}:{fid}:{past}")));
    assert_eq!(t.post_page(&expired).await.status, 404, "expired links do nothing");
    let ok = t.post_page(&confirm_path).await;
    assert_eq!(ok.status, 200);
    let nia = t.contact("nia@site.test").await;
    assert_eq!((nia["consent_status"].clone(), nia["consent_source"].clone()), (json!("Subscribed"), json!("double opt-in: Website")));
    assert_eq!(nia["consent_ip_hash"].as_str().map(str::len), Some(64));
    assert_eq!(
        t.scalar::<i64>(
            "select count(*) from e_email_group_member where tenant_id = $1 and email = 'nia@site.test' and email_group = $2 and not unsubscribed",
            Some(website)
        )
        .await,
        1
    );
    let form_now = t.get("SubscriptionForm", fid).await;
    assert_eq!((int(&form_now["submission_count"]), int(&form_now["confirmed_count"])), (2, 1), "bots and invalid addresses are not submissions");
    // Asha (unsubscribed through a link) comes back through a browser form; Fay (complained) and
    // Gus (blocked) get nothing — with the very same answer.
    let browser = |email: &str| {
        let t = &t;
        let body = format!("email={}&first_name=Someone", email.replace('@', "%40"));
        async move {
            t.site("POST", &format!("/subscribe/{fid}"), body.into_bytes(), &[("content-type", "application/x-www-form-urlencoded")], "203.0.113.10")
                .await
                .unwrap()
        }
    };
    t.post_page(&links(&mail_of("asha@x.test").html, "u")[0]).await;
    assert_eq!(t.contact("asha@x.test").await["consent_status"], json!("Unsubscribed"));
    let before = t.smtp.count();
    for who in ["asha@x.test", "fay@x.test", "gus@x.test"] {
        let r = browser(who).await;
        assert_eq!((r.status, r.headers.iter().find(|(k, _)| k == "location").map(|(_, v)| v.as_str())), (303, Some("https://acme.test/thanks")));
    }
    t.drain().await;
    let fresh: Vec<Mail> = t.smtp.mails()[before..].iter().map(parse).collect();
    assert_eq!(fresh.iter().map(|m| m.to.as_str()).collect::<Vec<_>>(), vec!["asha@x.test"]);
    let asha_confirm =
        regex::Regex::new(&format!(r"{}(/confirm/[A-Za-z0-9.]+)", regex::escape(BASE))).unwrap().captures(&fresh[0].html).unwrap()[1].to_string();
    t.post_page(&asha_confirm).await;
    assert_eq!(t.contact("asha@x.test").await["consent_status"], json!("Subscribed"));
    assert_eq!(
        t.scalar::<i64>("select count(*) from e_email_suppression where tenant_id = $1 and email = 'asha@x.test'", None).await,
        0,
        "her own opt-in lifts it"
    );
    assert_eq!(t.scalar::<i64>("select count(*) from e_email_suppression where tenant_id = $1 and email = 'gus@x.test'", None).await, 1, "manual blocks stay");

    // ───────────── journeys: welcome series with a wait and a branch on opens ─────────────
    let reengage = uid(&t.insert(mgr, "EmailGroup", json!({"group_name": "Re-engage"})).await);
    let welcome = uid(&t
        .insert(
            mgr,
            "MarketingJourney",
            json!({"journey_name": "Welcome", "trigger_type": "Joined Email Group", "trigger_group": website.to_string(), "steps": [
                {"step_type": "Send Email", "label": "Welcome", "subject": "Welcome {{ contact.first_name }}", "content": "<p>Start <a href=\"https://acme.test/start\">here</a></p>"},
                {"step_type": "Wait", "wait_amount": 1, "wait_unit": "Days"},
                {"step_type": "Branch", "branch_on": "Opened Last Email", "next_step": 4, "else_step": 6},
                {"step_type": "Update Attribute", "attribute": "engaged", "value": "yes"},
                {"step_type": "Exit"},
                {"step_type": "Send Email", "subject": "We miss you, {{ contact.first_name }}", "content": "<p>Come back</p>"},
                {"step_type": "Add To Group", "email_group": reengage.to_string()}]}),
        )
        .await);
    refused(
        k.insert(
            mgr,
            "MarketingJourney",
            obj(json!({"journey_name": "Broken", "trigger_type": "Joined Email Group", "trigger_group": website.to_string(),
                                                          "steps": [{"step_type": "Branch", "branch_on": "Opened Last Email", "next_step": 9}]})),
        )
        .await,
        "invalid",
    );
    refused(k.run_action(uma, "MarketingJourney", welcome, "activate", json!({})).await, "forbidden");
    t.action(mgr, "MarketingJourney", welcome, "activate", json!({})).await;
    refused(k.update(mgr, "MarketingJourney", welcome, obj(json!({"steps": [{"step_type": "Exit"}]})), None).await, "invalid");
    t.update(mgr, "MarketingJourney", welcome, json!({"journey_name": "Welcome series"})).await;
    for (email, name) in [("jay@site.test", "Jay"), ("kim@site.test", "Kim")] {
        t.insert(mgr, "EmailGroupMember", json!({"email_group": website.to_string(), "email": email, "member_name": name})).await;
    }
    t.concurrent_maint(3).await;
    t.maint().await;
    let enrollments = t.list("JourneyEnrollment", json!({"journey": welcome.to_string()})).await;
    let mut who: Vec<&str> = enrollments.iter().map(|e| e["email"].as_str().unwrap()).collect();
    who.sort();
    assert_eq!(who, vec!["jay@site.test", "kim@site.test"], "Nia was already in the group when it started");
    for e in &enrollments {
        assert_eq!((status_of(e), int(&e["current_step"]), e["waiting"].clone()), ("Active", 2, json!(true)));
        let next = chrono::DateTime::parse_from_rfc3339(e["next_run_at"].as_str().unwrap()).unwrap();
        let hours = (next.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_minutes() as f64 / 60.0;
        assert!((22.9..=25.1).contains(&hours), "a day in New York: {hours}h");
    }
    // Kim leaves the group and joins again while still in the journey (re-entry allowed): she is
    // not enrolled a second time while the first run is active.
    t.exec("update e_marketing_journey set allow_reentry = true where tenant_id = $1 and id = $2", Some(welcome)).await;
    let kim_member: Uuid =
        t.scalar("select id from e_email_group_member where tenant_id = $1 and email = 'kim@site.test' and email_group = $2", Some(website)).await;
    k.delete(mgr, "EmailGroupMember", kim_member, None).await.unwrap();
    t.maint().await;
    t.insert(mgr, "EmailGroupMember", json!({"email_group": website.to_string(), "email": "kim@site.test", "member_name": "Kim"})).await;
    t.maint().await;
    assert_eq!(t.list("JourneyEnrollment", json!({"journey": welcome.to_string()})).await.len(), 2);
    t.exec("update e_marketing_journey set allow_reentry = false where tenant_id = $1 and id = $2", Some(welcome)).await;
    t.drain().await;
    let welcomes = t.received("Welcome ");
    assert_eq!(welcomes.len(), 2, "each welcome sent once under concurrent maintenance");
    refused(k.update(mgr, "JourneyEnrollment", uid(&enrollments[0]), obj(json!({"current_step": 5})), None).await, "forbidden");
    t.exec("update e_communication set sent_at = sent_at - interval '1 hour' where tenant_id = $1 and $2::uuid is not null", Some(welcome)).await;
    let jay_welcome = welcomes.iter().find(|m| m.to == "jay@site.test").unwrap();
    t.get_page(&links(&jay_welcome.html, "o")[0], iphone).await;
    t.maint().await;
    assert_eq!(t.received("We miss you").len(), 0, "waiting still");
    t.exec("update e_journey_enrollment set next_run_at = now() - interval '1 minute' where tenant_id = $1 and journey = $2", Some(welcome)).await;
    t.concurrent_maint(2).await;
    t.drain().await;
    let jay = t.contact("jay@site.test").await;
    assert_eq!((jay["attributes"]["engaged"].clone(), jay["source"].clone()), (json!("yes"), json!("Email Group")));
    let misses = t.received("We miss you");
    assert_eq!(misses.iter().map(|m| m.to.as_str()).collect::<Vec<_>>(), vec!["kim@site.test"]);
    assert_eq!(
        t.scalar::<i64>("select count(*) from e_email_group_member where tenant_id = $1 and email = 'kim@site.test' and email_group = $2", Some(reengage))
            .await,
        1
    );
    let enrollments = t.list("JourneyEnrollment", json!({"journey": welcome.to_string()})).await;
    assert!(enrollments.iter().all(|e| status_of(e) == "Completed"), "{enrollments:?}");
    let funnel = t.report(mgr, "journey_funnel", &[("journey", welcome.to_string())]).await;
    let reach: Vec<(i64, i64, i64)> =
        funnel["rows"].as_array().unwrap().iter().map(|r| (int(&r["reached"]), int(&r["emails_sent"]), int(&r["unique_opens"]))).collect();
    assert_eq!(reach, vec![(2, 2, 1), (2, 0, 0), (2, 0, 0), (1, 0, 0), (1, 0, 0), (1, 1, 0), (1, 0, 0)]);
    assert_eq!((int(&funnel["totals"]["enrolled"]), int(&funnel["totals"]["completed"])), (2, 2));
    assert_eq!(int(&t.get("MarketingJourney", welcome).await["completed_count"]), 2);

    // ───────────── journeys: an attribute change with a goal, and an exit on unsubscribe ─────────────
    let vips = uid(&t
        .insert(mgr, "MarketingSegment", seg("VIPs", true, json!([{"rule_type": "Attribute", "attribute": "vip", "operator": "Equals", "value": "yes"}])))
        .await);
    let upgrade = uid(&t
        .insert(
            mgr,
            "MarketingJourney",
            json!({"journey_name": "Upgrade", "trigger_type": "Attribute Changed", "trigger_attribute": "plan", "trigger_value": "Paid", "goal_segment": vips.to_string(),
                   "steps": [{"step_type": "Wait", "wait_amount": 2, "wait_unit": "Hours"}, {"step_type": "Send Email", "subject": "Thanks for upgrading", "content": "<p>Thanks</p>"}]}),
        )
        .await);
    let (p1, p2, p3) = (ab_contacts[0], ab_contacts[1], ab_contacts[2]);
    t.update(mgr, "MarketingContact", p3, json!({"attributes": {"plan": "paid"}})).await;
    t.action(mgr, "MarketingJourney", upgrade, "activate", json!({})).await;
    for p in [p1, p2] {
        t.update(mgr, "MarketingContact", p, json!({"attributes": {"plan": "paid"}})).await;
    }
    t.maint().await;
    let up = t.list("JourneyEnrollment", json!({"journey": upgrade.to_string()})).await;
    assert_eq!(up.len(), 2, "changes before activation do not count");
    t.update(mgr, "MarketingContact", p2, json!({"consent_status": "Unsubscribed"})).await;
    t.update(mgr, "MarketingContact", p1, json!({"attributes": {"plan": "paid", "vip": "yes"}})).await;
    t.maint().await;
    let by_contact: HashMap<String, String> = t
        .list("JourneyEnrollment", json!({"journey": upgrade.to_string()}))
        .await
        .iter()
        .map(|e| (e["contact"].as_str().unwrap().to_string(), status_of(e).to_string()))
        .collect();
    assert_eq!((by_contact[&p1.to_string()].as_str(), by_contact[&p2.to_string()].as_str()), ("Goal Reached", "Exited"));
    t.update(mgr, "MarketingContact", p1, json!({"attributes": {"plan": "free", "vip": "yes"}})).await;
    t.update(mgr, "MarketingContact", p1, json!({"attributes": {"plan": "paid", "vip": "yes"}})).await;
    t.maint().await;
    assert_eq!(t.list("JourneyEnrollment", json!({"journey": upgrade.to_string()})).await.len(), 2, "no re-entry");
    assert_eq!(t.received("Thanks for upgrading").len(), 0);
    t.action(mgr, "MarketingJourney", upgrade, "stop", json!({})).await;
    refused(k.run_action(mgr, "MarketingJourney", upgrade, "activate", json!({})).await, "invalid");

    // ───────────── journeys: a birthday, once a year, in the workspace's time zone ─────────────
    let today = ferrum_kernel::ctx::local_now("America/New_York").date_naive();
    let tomorrow = today + chrono::Duration::days(1);
    t.update(mgr, "MarketingContact", ab_contacts[3], json!({"attributes": {"birthday": format!("1990-{}", today.format("%m-%d"))}})).await;
    t.update(mgr, "MarketingContact", ab_contacts[4], json!({"attributes": {"birthday": format!("1991-{}", tomorrow.format("%m-%d"))}})).await;
    let birthday = uid(&t
        .insert(mgr, "MarketingJourney", json!({"journey_name": "Birthday", "trigger_type": "Date Anniversary", "trigger_attribute": "birthday",
                                                 "steps": [{"step_type": "Send Email", "subject": "Happy birthday {{ contact.first_name }}", "content": "<p>Cake!</p>"}]}))
        .await);
    t.action(mgr, "MarketingJourney", birthday, "activate", json!({})).await;
    t.maint().await;
    t.maint().await;
    t.drain().await;
    assert_eq!(t.received("Happy birthday").iter().map(|m| m.to.as_str()).collect::<Vec<_>>(), vec!["ab04@ab.test"], "today's only, once");
    let bday = t.list("JourneyEnrollment", json!({"journey": birthday.to_string()})).await;
    assert_eq!((bday.len(), status_of(&bday[0])), (1, "Completed"));

    // ───────────── engine records stay the engine's ─────────────
    refused(k.insert(&admin, "CampaignRecipient", obj(json!({"email": "x@y.test", "campaign": c1.to_string()}))).await, "forbidden");
    let any_recipient = uid(&t.recipients(c1).await[0]);
    refused(k.update(&admin, "CampaignRecipient", any_recipient, obj(json!({"click_count": 99})), None).await, "forbidden");
    refused(k.delete(&admin, "CampaignRecipient", any_recipient, None).await, "forbidden");
    refused(k.list(sam, "MarketingContact", &ListQuery::default()).await, "forbidden");

    // ───────────── audience growth, today ─────────────
    let growth = t.report(uma, "audience_growth", &[("from_date", today.to_string()), ("to_date", today.to_string())]).await;
    let g = &growth["rows"][0];
    let contacts_now: i64 = t.scalar("select count(*) from e_marketing_contact where tenant_id = $1", None).await;
    assert_eq!(int(&g["new_contacts"]), contacts_now, "everyone was created today");
    let changes: i64 = t.scalar("select count(*) from campaigns_contact_change where tenant_id = $1 and field = 'consent_status' and old_value = 'Subscribed' and new_value = 'Unsubscribed'", None).await;
    assert_eq!(int(&g["unsubscribed"]), changes);
    assert_eq!((int(&g["bounced"]), int(&g["complained"])), (2, 1), "Dan and Eve; Fay");
    let subscribed_now: i64 = t.scalar("select count(*) from e_marketing_contact where tenant_id = $1 and consent_status = 'Subscribed'", None).await;
    assert_eq!(int(&g["net_change"]), subscribed_now, "today's net change is today's subscribed audience");
    assert_eq!(int(&growth["meta"]["subscribed_now"]), subscribed_now);
}
