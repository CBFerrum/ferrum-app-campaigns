//! Shared by the new suites: an SMTP sink, a booted workspace with the Mail, CRM, ERP and Campaigns packages, and the
//! helpers the older suites carry inline. Each suite uses its own database.
#![allow(dead_code)]

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

pub const PASSWORD: &str = "Correct-Horse-9";

pub fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

pub fn uid(d: &Doc) -> Uuid {
    d["id"].as_str().unwrap_or_else(|| panic!("no id in {d:?}")).parse().unwrap()
}

pub fn refused<T: std::fmt::Debug>(r: Result<T, Error>) {
    assert!(matches!(r, Err(Error::Invalid(_) | Error::Validation(_) | Error::Forbidden(_))), "expected a refusal, got {r:?}");
}

/// An SMTP server that accepts everything (the Mail app delivers to it).
#[derive(Clone, Default)]
pub struct Smtp(pub Arc<Mutex<usize>>);

impl Smtp {
    pub async fn start(&self) -> u16 {
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

    pub async fn session(&self, sock: tokio::net::TcpStream) -> std::io::Result<()> {
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

pub struct T {
    pub k: Arc<Kernel>,
    /// Campaigns\' own handle on the vault, as its registration was given it: what reads its secret fields.
    pub campaigns: ferrum_kernel::vault::Vault,
    pub tenant: Uuid,
    pub admin: Ctx,
}

#[allow(dead_code)]
impl T {
    pub async fn insert(&self, ctx: &Ctx, entity: &str, v: Value) -> Doc {
        self.k.insert(ctx, entity, obj(v)).await.unwrap_or_else(|e| panic!("insert {entity}: {e:?}"))
    }

    pub async fn update(&self, ctx: &Ctx, entity: &str, id: Uuid, v: Value) -> Doc {
        self.k.update(ctx, entity, id, obj(v), None).await.unwrap_or_else(|e| panic!("update {entity}: {e:?}"))
    }

    pub async fn get(&self, entity: &str, id: Uuid) -> Doc {
        self.k.get(&self.admin, entity, id, false).await.unwrap_or_else(|e| panic!("get {entity}: {e:?}"))
    }

    pub async fn action(&self, ctx: &Ctx, entity: &str, id: Uuid, action: &str, params: Value) -> Value {
        self.k.run_action(ctx, entity, id, action, params).await.unwrap_or_else(|e| panic!("{entity}.{action}: {e:?}"))
    }

    pub async fn list(&self, entity: &str, filters: Value) -> Vec<Doc> {
        self.list_as(&self.admin, entity, filters).await
    }

    pub async fn list_as(&self, ctx: &Ctx, entity: &str, filters: Value) -> Vec<Doc> {
        let q = ListQuery { filters: ferrum_kernel::query::parse_filters(&filters).unwrap(), limit: Some(500), ..Default::default() };
        self.k.list(ctx, entity, &q).await.unwrap().rows
    }

    pub async fn scalar<O: Send + Unpin + for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>>(&self, sql: &str, id: Option<Uuid>) -> O {
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

    pub async fn exec(&self, sql: &str, id: Option<Uuid>) {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        sqlx::query(sqlx::AssertSqlSafe(sql.to_string())).bind(self.tenant).bind(id).execute(&mut *tx).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
        tx.commit().await.unwrap();
    }

    pub async fn maint(&self) {
        ferrum_app_campaigns::maintenance(&self.k, self.tenant).await.expect("maintenance");
    }

    /// Delivers what the outbox holds: mail, and record events to this app's sink.
    pub async fn drain(&self) {
        while self.k.dispatch_outbox_once().await.expect("dispatch") > 0 {}
    }

    pub async fn site(&self, method: &str, path: &str, body: &str) -> SiteResponse {
        self.site_ua(method, path, body, "Mozilla/5.0 (Macintosh) Test").await
    }

    pub async fn site_ua(&self, method: &str, path: &str, body: &str, user_agent: &str) -> SiteResponse {
        let mut h = Map::new();
        h.insert("content-type".into(), json!("application/x-www-form-urlencoded"));
        h.insert("user-agent".into(), json!(user_agent));
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

    pub async fn key(&self) -> Vec<u8> {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        let sid: Uuid = sqlx::query_scalar("select id from e_campaign_settings where tenant_id = $1").bind(self.tenant).fetch_one(&mut *tx).await.unwrap();
        let key = self.campaigns.field(&self.k, &mut tx, self.tenant, "CampaignSettings", sid, "signing_key").await.unwrap().expect("signing key");
        tx.commit().await.unwrap();
        hex::decode(key.as_str()).unwrap()
    }

    /// The signed unsubscribe path of a recipient (as its email carries it).
    pub async fn unsubscribe_path(&self, recipient: Uuid) -> String {
        let key = self.key().await;
        let mut mac = Hmac::<Sha256>::new_from_slice(&key).unwrap();
        mac.update(format!("unsubscribe:{recipient}").as_bytes());
        format!("/u/{}.{}", recipient.simple(), hex::encode(&mac.finalize().into_bytes()[..16]))
    }

    /// A contact's recipient row of a campaign.
    pub async fn recipient_of(&self, campaign: Uuid, email: &str) -> Uuid {
        let rows = self.list("CampaignRecipient", json!({"campaign": campaign.to_string(), "email": email})).await;
        assert_eq!(rows.len(), 1, "{email} received {campaign}");
        uid(&rows[0])
    }

    /// Sends a campaign now and returns the addresses it went to.
    pub async fn send(&self, ctx: &Ctx, v: Value) -> (Uuid, Vec<String>) {
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

    pub async fn contact(&self, email: &str) -> Doc {
        let found = self.list("MarketingContact", json!({"email": email})).await;
        assert_eq!(found.len(), 1, "contact {email}");
        found[0].clone()
    }
}

pub fn body(r: &SiteResponse) -> String {
    String::from_utf8(r.body.clone()).unwrap()
}

/// What opens the test's vault: the database password of `url`, with Argon2id kept cheap for tests.
pub fn opener(url: &str) -> ferrum_kernel::vault::Opener {
    ferrum_kernel::vault::Opener {
        database_url: url.to_string(),
        file: "/nonexistent/root.key".into(),
        kdf: ferrum_kernel::vault::Kdf { m_kib: 64, t: 1, p: 1 },
        ..Default::default()
    }
}

/// For a caller who may not see the record or the action at all: the platform answers "not found" there, which is a
/// refusal too. Only for such assertions; every other refusal is checked with the strict `refused`.
pub fn hidden_or_refused<T: std::fmt::Debug>(r: Result<T, Error>) {
    assert!(
        matches!(r, Err(Error::Invalid(_) | Error::Validation(_) | Error::Forbidden(_) | Error::NotFound(_))),
        "expected a refusal or not found, got {r:?}"
    );
}

/// A workspace with Mail, ERP, CRM and Campaigns, the Marketing Manager `mgr` and a mail account that delivers to an
/// SMTP sink. `env_var` names the suite's database variable; `db_name` is the fallback.
pub async fn boot(env_var: &str, db_name: &str) -> (T, Ctx, u16) {
    let url = std::env::var(env_var).unwrap_or_else(|_| format!("postgres://ferrum:ferrum@localhost:5432/{db_name}"));
    let db = Db::connect(DbConfig { url: url.clone(), max_connections: 20, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists vault cascade; drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    ferrum_kernel::vault::open(&db, opener(&url), false).await.expect("the vault");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_comms::packages().expect("mail packages"));
    packages.extend(ferrum_app_erp::packages().expect("ERP's packages"));
    packages.extend(ferrum_app_crm::packages().expect("crm packages"));
    packages.extend(ferrum_app_campaigns::packages().expect("app packages"));
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    hooks.app("comms", &[], ferrum_app_comms::register);
    let comms_packages: Vec<String> = ferrum_app_comms::packages().unwrap().iter().map(|p| p.name().to_string()).collect();
    hooks.own("comms", &comms_packages);
    let erp_packages: Vec<String> = ferrum_app_erp::packages().unwrap().iter().map(|p| p.name().to_string()).collect();
    hooks.app("erp", &[], ferrum_app_erp::register);
    hooks.own("erp", &erp_packages);
    ferrum_app_crm::register(&mut hooks);
    let campaigns_packages: Vec<String> = ferrum_app_campaigns::packages().unwrap().iter().map(|p| p.name().to_string()).collect();
    let mut campaigns = None;
    hooks.app("campaigns", &campaigns_packages, |h| {
        ferrum_app_campaigns::register(h);
        campaigns = Some(h.vault());
    });
    hooks.own("campaigns", &campaigns_packages);
    for source in ferrum_app_campaigns::reports() {
        ferrum_domain::reports::add_source(source);
    }
    let k = Kernel::boot(db, catalog, hooks, KernelSettings::default(), None).await.expect("boot");
    ferrum_domain::maintenance(&k).await.expect("partitions");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "acme".into(),
            name: "Acme Ltd".into(),
            admin_email: "admin@acme.test".into(),
            admin_password: PASSWORD.into(),
            admin_name: None,
            packages: ["core", "comms", "campaigns", "selling", "crm", "campaigns_crm"].iter().map(|p| p.to_string()).collect(),
            settings: Some(json!({"timezone": "UTC"})),
        })
        .await
        .expect("tenant");
    let session = k.login("acme", "admin@acme.test", PASSWORD).await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("admin ctx");
    let mgr_id = k.create_user(&admin, "mgr@acme.test", "Mia", Some(PASSWORD), &["Marketing Manager".to_string()], false).await.expect("user");
    let mgr = k.load_ctx(tenant, mgr_id, None, "it".into()).await.unwrap();
    let t = T { k: k.clone(), campaigns: campaigns.expect("Campaigns' handle"), tenant, admin: admin.clone() };
    let port = Smtp::default().start().await;
    t.insert(&admin, "CommsSettings", json!({"public_url": "https://erp.acme.test/"})).await;
    t.insert(
        &admin,
        "EmailAccount",
        json!({"account_name": "Marketing", "email_address": "news@acme.test", "default_outgoing": true, "smtp_host": "127.0.0.1", "smtp_port": port,
               "smtp_security": "None"}),
    )
    .await;
    t.insert(&mgr, "CampaignSettings", json!({"postal_address": "Acme Ltd, 1 Main Street", "hourly_limit": 10000})).await;
    (t, mgr, port)
}

impl T {
    /// A user holding only `role`, whose rules are `perms` (each `{entity, read, write, ...}`), as its own context.
    pub async fn person(&self, email: &str, role: &str, perms: Value) -> Ctx {
        // The administrator's context as it is now: it knows entities defined since the workspace was made.
        let admin = self.reload(&self.admin).await;
        for p in perms.as_array().unwrap() {
            let def: ferrum_meta::PermissionDef = serde_json::from_value(p.clone()).unwrap();
            self.k.set_role_permission(&admin, role, &def).await.unwrap_or_else(|e| panic!("role {role}: {e:?}"));
        }
        let id = self.k.create_user(&self.admin, email, email, Some(PASSWORD), &[role.to_string()], false).await.expect("user");
        self.k.load_ctx(self.tenant, id, None, "it".into()).await.unwrap()
    }

    /// The same user's context loaded again (after its role changed).
    pub async fn reload(&self, ctx: &Ctx) -> Ctx {
        self.k.load_ctx(self.tenant, ctx.user, None, "it".into()).await.unwrap()
    }
}
