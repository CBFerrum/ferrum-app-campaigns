//! The gap-closing items against a real PostgreSQL (`FERRUM_CAMPAIGNS_GC_TEST_DATABASE_URL`, default
//! `postgres://ferrum:ferrum@localhost:5432/ferrum_campaigns_gc_test`), reset first. Each item is one section of
//! one scenario, written before its code.

mod common;

use common::*;
use ferrum_kernel::Ctx;
use ferrum_kernel::capability::Invocation;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use uuid::Uuid;

#[test]
fn gap_closing_items() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let (t, mgr, _smtp) = boot("FERRUM_CAMPAIGNS_GC_TEST_DATABASE_URL", "ferrum_campaigns_gc_test").await;
    for (email, first) in [("asha@x.test", "Asha"), ("ben@x.test", "Ben")] {
        t.insert(&mgr, "MarketingContact", json!({"email": email, "first_name": first})).await;
    }
    segments_k0(&t, &mgr).await;
    journey_email_refused_by_the_platform_is_skipped(&t, &mgr).await;
    frequency_cap(&t, &mgr).await;
    pause_and_resume(&t, &mgr).await;
    send_time(&t, &mgr).await;
    provider_notices(&t, &mgr).await;
    designs(&t, &mgr).await;
}

async fn settings_of(t: &T) -> Uuid {
    uid(&t.list("CampaignSettings", json!({})).await[0])
}

async fn people(t: &T, mgr: &Ctx, domain: &str, n: usize) -> Vec<Uuid> {
    let mut out = Vec::new();
    for i in 0..n {
        out.push(uid(&t.insert(mgr, "MarketingContact", json!({"email": format!("p{i:02}@{domain}")})).await));
    }
    out
}

/// A segment of everyone at `domain`.
async fn at_domain(t: &T, mgr: &Ctx, domain: &str) -> Value {
    let rule = json!({"rule_type": "Contact Field", "field": "Email", "operator": "Ends With", "value": format!("@{domain}")});
    let s = uid(&t.insert(mgr, "MarketingSegment", json!({"segment_name": domain, "match_type": "All", "rules": [rule]})).await);
    json!([{"segment": s.to_string()}])
}

fn campaign(name: &str, audience: &Value) -> Value {
    json!({"campaign_name": name, "subject": "Hello", "content": "<p>Hello</p>", "audience": audience})
}

/// A journey that mails on a change of the attribute `plan`, activated.
async fn plan_journey(t: &T, mgr: &Ctx, name: &str) -> Uuid {
    let j = uid(&t
        .insert(
            mgr,
            "MarketingJourney",
            json!({"journey_name": name, "trigger_type": "Attribute Changed", "trigger_attribute": "plan",
                   "steps": [{"step_type": "Send Email", "subject": "Your plan", "content": "<p>Plan</p>"}]}),
        )
        .await);
    t.action(mgr, "MarketingJourney", j, "activate", json!({})).await;
    j
}

/// campaigns-1: a journey email the platform refuses is Skipped at once, in the same transaction.
async fn journey_email_refused_by_the_platform_is_skipped(t: &T, mgr: &Ctx) {
    let rex = uid(&t.insert(mgr, "MarketingContact", json!({"email": "rex@j.test"})).await);
    t.k.restrict_processing(&t.admin, "rex@j.test", "asked").await.expect("restricted");
    let j = plan_journey(t, mgr, "Skip journey").await;
    t.update(mgr, "MarketingContact", rex, json!({"attributes": {"plan": "pro"}})).await;
    t.maint().await;
    let rows = t.list("CampaignRecipient", json!({"journey": j.to_string()})).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let r = &rows[0];
    assert_eq!(r["status"], json!("Skipped"), "{r:?}");
    assert!(r["skip_reason"].as_str().is_some_and(|w| !w.is_empty()) && r["communication"].is_null() && r["queued_at"].is_null(), "{r:?}");
    let logged: i64 = t
        .scalar("select count(*) from campaigns_journey_log where tenant_id = $1 and journey_id = (select id from e_marketing_journey where tenant_id = $1 and journey_name = 'Skip journey') and outcome like 'skipped%' and $2::uuid is null", None)
        .await;
    assert_eq!(logged, 1);
    assert_eq!(t.list("JourneyEnrollment", json!({"journey": j.to_string()})).await[0]["status"], json!("Completed"), "it moved past the step");
}

async fn mailed(t: &T, campaign: Uuid) -> Vec<String> {
    let mut to: Vec<String> = t
        .list("CampaignRecipient", json!({"campaign": campaign.to_string()}))
        .await
        .iter()
        .filter(|r| !r["communication"].is_null())
        .map(|r| r["email"].as_str().unwrap().to_string())
        .collect();
    to.sort();
    to
}

/// campaigns-2: the frequency cap, for campaigns and journeys, with the operational exemption.
async fn frequency_cap(t: &T, mgr: &Ctx) {
    let settings = settings_of(t).await;
    t.update(mgr, "CampaignSettings", settings, json!({"frequency_cap": 1, "frequency_days": 7})).await;
    let ana = uid(&t.insert(mgr, "MarketingContact", json!({"email": "ana@f.test"})).await);
    let bo = uid(&t.insert(mgr, "MarketingContact", json!({"email": "bo@f.test"})).await);
    let only_ana = {
        let rule = json!({"rule_type": "Contact Field", "field": "Email", "operator": "Equals", "value": "ana@f.test"});
        let s = uid(&t.insert(mgr, "MarketingSegment", json!({"segment_name": "ana", "match_type": "All", "rules": [rule]})).await);
        json!([{"segment": s.to_string()}])
    };
    let both = at_domain(t, mgr, "f.test").await;
    // the_cap_leaves_a_person_out_of_a_campaign
    let (a, to) = t.send(mgr, campaign("A", &only_ana)).await;
    assert_eq!(to, ["ana@f.test"]);
    let (b, to) = t.send(mgr, campaign("B", &both)).await;
    assert_eq!(to, ["bo@f.test"], "Ana already had her one this week");
    let row = &t.list("CampaignRecipient", json!({"campaign": b.to_string(), "email": "ana@f.test"})).await[0];
    assert_eq!((row["status"].clone(), row["skip_reason"].clone()), (json!("Skipped"), json!("frequency cap")), "{row:?}");
    let _ = a;
    // the review counts them the way sending does
    let c = uid(&t.insert(mgr, "EmailCampaign", campaign("C", &both)).await);
    let check = t.action(mgr, "EmailCampaign", c, "audience_check", json!({})).await;
    assert_eq!(check["capped"], json!(2), "both have their one now: {check}");
    // an_operational_topic_passes_the_cap, and its mail does not count toward the next promotional one
    let service = uid(&t.insert(mgr, "MarketingTopic", json!({"topic_name": "Service", "category": "Operational"})).await);
    let mut notice = campaign("Notice", &only_ana);
    notice["topic"] = json!(service.to_string());
    let (_, to) = t.send(mgr, notice).await;
    assert_eq!(to, ["ana@f.test"], "an operational topic is not held back");
    let cap_after: i64 = t
        .scalar("select count(*) from e_campaign_recipient r join e_marketing_topic tp on tp.id = r.topic where r.tenant_id = $1 and tp.category = 'Operational' and r.communication is not null and $2::uuid is null", None)
        .await;
    assert_eq!(cap_after, 1);
    t.exec("update e_campaign_recipient set queued_at = now() - interval '8 days' where tenant_id = $1 and contact = (select id from e_marketing_contact where tenant_id = $1 and email = 'bo@f.test') and $2::uuid is null", None).await;
    let (d, to) = t.send(mgr, campaign("D", &both)).await;
    assert_eq!(to, ["bo@f.test"], "Bo's old email left the window; Ana's operational one never counted but her promotional A did");
    let _ = (d, ana, bo);

    // a_journey_step_waits_for_the_cap
    let jay = uid(&t.insert(mgr, "MarketingContact", json!({"email": "jay@g.test"})).await);
    let seg = at_domain(t, mgr, "g.test").await;
    let (_, to) = t.send(mgr, campaign("Jay's first", &seg)).await;
    assert_eq!(to, ["jay@g.test"]);
    let j = plan_journey(t, mgr, "Capped journey").await;
    t.update(mgr, "MarketingContact", jay, json!({"attributes": {"plan": "pro"}})).await;
    t.maint().await;
    assert!(t.list("CampaignRecipient", json!({"journey": j.to_string()})).await.is_empty(), "no row is made while the cap holds");
    let e = &t.list("JourneyEnrollment", json!({"journey": j.to_string()})).await[0];
    assert_eq!((e["status"].clone(), e["current_step"].clone()), (json!("Active"), json!(1)), "{e:?}");
    let waits: i64 =
        t.scalar("select count(*) from campaigns_journey_log where tenant_id = $1 and outcome = 'waiting: frequency cap' and $2::uuid is null", None).await;
    assert_eq!(waits, 1);
    let later: bool = t
        .scalar(
            &format!("select next_run_at > now() + interval '1 day' from e_journey_enrollment where tenant_id = $1 and journey = '{j}' and $2::uuid is null"),
            None,
        )
        .await;
    assert!(later, "the step waits until the counted email leaves the window");
    t.exec("update e_campaign_recipient set queued_at = now() - interval '8 days' where tenant_id = $1 and contact = $2", Some(jay)).await;
    t.exec(
        &format!("update e_journey_enrollment set next_run_at = now() - interval '1 second' where tenant_id = $1 and journey = '{j}' and $2::uuid is null"),
        None,
    )
    .await;
    t.maint().await;
    let sent = t.list("CampaignRecipient", json!({"journey": j.to_string()})).await;
    assert!(sent.len() == 1 && !sent[0]["communication"].is_null(), "{sent:?}");

    // two_senders_never_pass_the_cap
    let crowd = people(t, mgr, "h.test", 20).await;
    let all = at_domain(t, mgr, "h.test").await;
    for name in ["Two A", "Two B"] {
        let c = uid(&t.insert(mgr, "EmailCampaign", campaign(name, &all)).await);
        t.k.submit(mgr, "EmailCampaign", c, None).await.expect("submit");
    }
    for _ in 0..3 {
        tokio::join!(t.maint(), t.maint());
    }
    let over: i64 = t
        .scalar("select count(*) from (select contact from e_campaign_recipient where tenant_id = $1 and communication is not null and $2::uuid is null group by contact having count(*) > 1) x", None)
        .await;
    assert_eq!(over, 0, "nobody went past the cap");
    let once: i64 = t
        .scalar("select count(distinct contact) from e_campaign_recipient r join e_marketing_contact c on c.id = r.contact where r.tenant_id = $1 and c.email like '%@h.test' and r.communication is not null and $2::uuid is null", None)
        .await;
    assert_eq!(once as usize, crowd.len(), "and each got exactly one");
    t.update(mgr, "CampaignSettings", settings, json!({"frequency_cap": 0})).await;
}

/// campaigns-2: a campaign pauses itself on complaints, a manager resumes it, and the pause does not repeat.
async fn pause_and_resume(t: &T, mgr: &Ctx) {
    let settings = settings_of(t).await;
    t.update(mgr, "CampaignSettings", settings, json!({"pause_complaint_percent": "1", "pause_after_sent": 50})).await;
    people(t, mgr, "z.test", 60).await;
    let aud = at_domain(t, mgr, "z.test").await;
    let mut v = campaign("Big", &aud);
    v["hourly_limit"] = json!(55);
    let (c, to) = t.send(mgr, v).await;
    assert_eq!(to.len(), 55);
    t.exec("update e_campaign_recipient set complained_at = now() where tenant_id = $1 and campaign = (select id from e_email_campaign where tenant_id = $1 and campaign_name = 'Big') and communication is not null and email = 'p00@z.test' and $2::uuid is null", None).await;
    t.maint().await;
    let doc = t.get("EmailCampaign", c).await;
    assert_eq!((doc["status"].clone(), doc["pause_cause"].clone()), (json!("Paused"), json!("Complaints")), "{doc:?}");
    assert!(doc["paused_at"].is_string() && doc["pause_rate"].as_str().is_some_and(|r| r.parse::<f64>().unwrap() >= 1.0), "{doc:?}");
    let pending = t.list("CampaignRecipient", json!({"campaign": c.to_string(), "status": "Pending"})).await.len();
    assert_eq!(pending, 5, "what waits stays Pending");
    let notices: i64 = t
        .scalar(
            "select count(*) from message_send where tenant_id = $1 and template = 'campaigns.campaign_paused' and channel = 'inapp' and $2::uuid is null",
            None,
        )
        .await;
    assert_eq!(notices, 1, "the maker is told");
    t.maint().await;
    assert_eq!(t.list("CampaignRecipient", json!({"campaign": c.to_string(), "status": "Pending"})).await.len(), 5, "a paused campaign sends nothing");
    // resume_needs_submit, and only a paused campaign resumes
    let uma = t.person("uma@acme.test", "Campaign Writer", json!([{"entity": "EmailCampaign", "read": true, "write": true}])).await;
    refused(t.k.run_action(&uma, "EmailCampaign", c, "resume", json!({})).await);
    // resume_repeats_once
    let inv = || Invocation::new("campaigns.email_campaign.resume", json!({"id": c.to_string()})).key("resume-1");
    let first = t.k.invoke(mgr, inv()).await.expect("resumed");
    assert!(!first.repeated);
    assert!(t.k.invoke(mgr, inv()).await.expect("repeat").repeated, "the same key answers repeated");
    refused(t.k.run_action(mgr, "EmailCampaign", c, "resume", json!({})).await);
    // a_resumed_campaign_is_not_paused_again
    t.exec("update e_campaign_recipient set queued_at = now() - interval '2 hours' where tenant_id = $1 and $2::uuid is null", None).await;
    t.maint().await;
    t.maint().await;
    let doc = t.get("EmailCampaign", c).await;
    assert_eq!((doc["status"].clone(), doc["resumed_by"].clone()), (json!("Sent"), json!("mgr@acme.test")), "{doc:?}");
    // off until a rate is set: complaints stop nothing
    t.update(mgr, "CampaignSettings", settings, json!({"pause_complaint_percent": null})).await;
    people(t, mgr, "y.test", 60).await;
    let aud = at_domain(t, mgr, "y.test").await;
    let (c2, _) = t.send(mgr, campaign("Unjudged", &aud)).await;
    t.exec("update e_campaign_recipient set complained_at = now() where tenant_id = $1 and campaign = (select id from e_email_campaign where tenant_id = $1 and campaign_name = 'Unjudged') and email like 'p0%' and $2::uuid is null", None).await;
    t.maint().await;
    assert_eq!(t.get("EmailCampaign", c2).await["status"], json!("Sent"));
}

fn hour_of(hours: &[i32]) -> String {
    serde_json::to_string(hours).unwrap()
}

/// campaigns-3: each person's own send time.
async fn send_time(t: &T, mgr: &Ctx) {
    use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};
    use chrono_tz::Tz;
    // a_zone_must_be_one_the_database_knows
    let sol = uid(&t.insert(mgr, "MarketingContact", json!({"email": "sol@t.test"})).await);
    refused(t.k.update(mgr, "MarketingContact", sol, obj(json!({"time_zone": "Mars/Olympus"})), None).await);
    t.update(mgr, "MarketingContact", sol, json!({"time_zone": "Asia/Kolkata"})).await;
    assert_eq!(t.get("MarketingContact", sol).await["time_zone"], json!("Asia/Kolkata"));
    // send_time_is_refused_with_an_ab_test_and_for_text
    let aud = at_domain(t, mgr, "t.test").await;
    let mut bad = campaign("Bad A/B", &aud);
    bad["send_time"] = json!("Local Time");
    bad["ab_test"] = json!(true);
    bad["variants"] = json!([{"variant": "A", "subject": "a", "content": "<p>a</p>"}, {"variant": "B", "subject": "b", "content": "<p>b</p>"}]);
    refused(t.k.insert(mgr, "EmailCampaign", obj(bad)).await);
    let mut text = campaign("Bad text", &aud);
    text["send_time"] = json!("Best Hour");
    text["channel"] = json!("Text Message");
    refused(t.k.insert(mgr, "EmailCampaign", obj(text)).await);
    // opens_are_counted_in_the_persons_hour
    let (c0, _) = t.send(mgr, campaign("First", &aud)).await;
    let rid = t.recipient_of(c0, "sol@t.test").await;
    let open = |rid: Uuid| async move {
        let mut mac = Hmac::<Sha256>::new_from_slice(&t.key().await).unwrap();
        mac.update(format!("open:{rid}").as_bytes());
        format!("/o/{}.{}.gif", rid.simple(), hex::encode(&mac.finalize().into_bytes()[..16]))
    };
    let path = open(rid).await;
    let hour_in = |zone: &str| Utc::now().with_timezone(&zone.parse::<Tz>().unwrap()).format("%H").to_string().parse::<usize>().unwrap();
    let before = hour_in("Asia/Kolkata");
    t.site_ua("GET", &path, "", "Mozilla/5.0").await;
    let machine = t.get("MarketingContact", sol).await;
    assert!(machine["open_hours"].is_null(), "a machine open changes nothing: {machine:?}");
    t.site_ua("GET", &path, "", "Mozilla/5.0 (Macintosh) Test").await;
    let after = hour_in("Asia/Kolkata");
    let hours: Vec<i64> = t.get("MarketingContact", sol).await["open_hours"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
    assert_eq!((hours.len(), hours.iter().sum::<i64>()), (24, 1), "{hours:?}");
    assert!(hours[before] == 1 || hours[after] == 1, "{hours:?} at {before}..{after}");
    // a person's own update of open_hours changes nothing; a new zone clears the counts
    t.update(mgr, "MarketingContact", sol, json!({"open_hours": [9, 9]})).await;
    assert_eq!(t.get("MarketingContact", sol).await["open_hours"].as_array().unwrap().len(), 24);
    t.update(mgr, "MarketingContact", sol, json!({"time_zone": "Europe/Paris"})).await;
    assert!(t.get("MarketingContact", sol).await["open_hours"].is_null());

    // local_time_goes_when_each_clock_shows_it, nobody_gets_it_before_their_time
    let zones = [("home", None), ("east", Some("Asia/Kolkata")), ("west", Some("America/Los_Angeles")), ("odd", Some("Pacific/Auckland"))];
    for (name, zone) in zones {
        let id = uid(&t.insert(mgr, "MarketingContact", json!({"email": format!("{name}@lt.test")})).await);
        if let Some(z) = zone {
            t.update(mgr, "MarketingContact", id, json!({"time_zone": z})).await;
        }
    }
    let lt = at_domain(t, mgr, "lt.test").await;
    let schedule = Utc::now() - Duration::minutes(1);
    let mut v = campaign("Local", &lt);
    v["send_time"] = json!("Local Time");
    let c = uid(&t.insert(mgr, "EmailCampaign", v).await);
    t.k.submit(mgr, "EmailCampaign", c, None).await.expect("submit");
    t.exec(&format!("update e_email_campaign set schedule_at = '{}' where tenant_id = $1 and id = '{c}' and $2::uuid is null", schedule.to_rfc3339()), None)
        .await;
    t.maint().await;
    let expected = |zone: Option<&str>| -> DateTime<Utc> {
        let ws: Tz = "UTC".parse().unwrap();
        let z: Tz = zone.map(|z| z.parse().unwrap()).unwrap_or(ws);
        let tod: NaiveTime = schedule.with_timezone(&ws).time();
        let l = schedule.with_timezone(&z).naive_local();
        let mut d = l.date().and_time(tod);
        if d < l {
            d += Duration::days(1);
        }
        z.from_local_datetime(&d).earliest().unwrap().with_timezone(&Utc)
    };
    let now = Utc::now();
    for (name, zone) in zones {
        let row = &t.list("CampaignRecipient", json!({"campaign": c.to_string(), "email": format!("{name}@lt.test")})).await[0];
        let at: DateTime<Utc> = row["send_after"].as_str().unwrap().parse().unwrap();
        let want = expected(zone);
        assert!((at - want).num_seconds().abs() <= 1, "{name}: {at} vs {want}");
        let due = want <= now;
        assert_eq!(row["status"] == json!("Pending"), !due, "{name}: {row:?}");
        assert!(at - schedule <= Duration::hours(25), "within a day of the start");
    }
    for _ in 0..3 {
        t.maint().await;
    }
    let waiting = t.list("CampaignRecipient", json!({"campaign": c.to_string(), "status": "Pending"})).await.len();
    assert_eq!(waiting, zones.iter().filter(|(_, z)| expected(*z) > Utc::now()).count(), "nobody is handed it before their time");
    assert_eq!(t.get("EmailCampaign", c).await["status"], json!("Sending"));
    t.exec("update e_campaign_recipient set send_after = now() - interval '1 minute' where tenant_id = $1 and status = 'Pending' and $2::uuid is null", None)
        .await;
    t.maint().await;
    t.maint().await;
    assert_eq!(t.get("EmailCampaign", c).await["status"], json!("Sent"));

    // best_hour_uses_the_most_opened_hour
    let shape = |top: &[(usize, i32)]| {
        let mut h = vec![0; 24];
        for (i, n) in top {
            h[*i] = *n;
        }
        hour_of(&h)
    };
    let shapes = [("a", shape(&[(20, 5), (7, 2), (9, 2)])), ("b", shape(&[(7, 3), (9, 3)])), ("c", shape(&[(4, 2)]))];
    for (name, hours) in &shapes {
        let id = uid(&t.insert(mgr, "MarketingContact", json!({"email": format!("{name}@bh.test")})).await);
        t.exec(&format!("update e_marketing_contact set open_hours = '{hours}'::jsonb where tenant_id = $1 and id = $2"), Some(id)).await;
    }
    let bh = at_domain(t, mgr, "bh.test").await;
    let mut v = campaign("Best", &bh);
    v["send_time"] = json!("Best Hour");
    let b = uid(&t.insert(mgr, "EmailCampaign", v).await);
    t.k.submit(mgr, "EmailCampaign", b, None).await.expect("submit");
    let schedule = Utc::now() - Duration::minutes(1);
    t.exec(&format!("update e_email_campaign set schedule_at = '{}' where tenant_id = $1 and id = '{b}' and $2::uuid is null", schedule.to_rfc3339()), None)
        .await;
    t.maint().await;
    let next = |hour: u32| {
        let d = schedule.date_naive().and_hms_opt(hour, 0, 0).unwrap().and_utc();
        if d < schedule { d + Duration::days(1) } else { d }
    };
    for (name, want) in [("a", Some(next(20))), ("b", Some(next(7))), ("c", None)] {
        let row = &t.list("CampaignRecipient", json!({"campaign": b.to_string(), "email": format!("{name}@bh.test")})).await[0];
        let got: Option<DateTime<Utc>> = row["send_after"].as_str().map(|s| s.parse().unwrap());
        assert_eq!(got.map(|g| g.timestamp()), want.map(|w| w.timestamp()), "{name}: {row:?}");
    }

    // a_day_with_a_missing_hour_still_sends: Berlin, the evening before the spring change, 02:30 by the workspace's clock
    let ber = uid(&t.insert(mgr, "MarketingContact", json!({"email": "ber@dst.test"})).await);
    t.update(mgr, "MarketingContact", ber, json!({"time_zone": "Europe/Berlin"})).await;
    let dst = at_domain(t, mgr, "dst.test").await;
    let mut v = campaign("Night", &dst);
    v["send_time"] = json!("Local Time");
    let n = uid(&t.insert(mgr, "EmailCampaign", v).await);
    t.k.submit(mgr, "EmailCampaign", n, None).await.expect("submit");
    t.exec(&format!("update e_email_campaign set schedule_at = '2026-03-28T02:30:00Z' where tenant_id = $1 and id = '{n}' and $2::uuid is null"), None).await;
    t.maint().await;
    let row = &t.list("CampaignRecipient", json!({"campaign": n.to_string()})).await[0];
    let at: DateTime<Utc> = row["send_after"].as_str().unwrap().parse().unwrap();
    let start = Utc.with_ymd_and_hms(2026, 3, 28, 2, 30, 0).unwrap();
    assert!(at > start && at - start <= Duration::hours(25), "{at}");

    // erasure_blanks_zone_and_hours
    let eve = uid(&t.insert(mgr, "MarketingContact", json!({"email": "eve@er.test", "time_zone": "Asia/Tokyo"})).await);
    t.exec(&format!("update e_marketing_contact set open_hours = '{}'::jsonb where tenant_id = $1 and id = $2", shape(&[(3, 3)])), Some(eve)).await;
    t.k.erase_personal_data(&t.admin, "eve@er.test", "her request").await.expect("erasure");
    let gone = t.get("MarketingContact", eve).await;
    assert!(gone["time_zone"].is_null() && gone["open_hours"].is_null(), "{gone:?}");

    // the_signup_form_takes_a_zone
    let form = uid(&t.insert(mgr, "SubscriptionForm", json!({"form_name": "Zones"})).await);
    t.site("POST", &format!("/subscribe/{form}"), "email=zed%40sf.test&time_zone=Asia%2FTokyo").await;
    t.site("POST", &format!("/subscribe/{form}"), "email=yan%40sf.test&time_zone=..%2F..%2Fetc").await;
    assert_eq!(t.contact("zed@sf.test").await["time_zone"], json!("Asia/Tokyo"));
    assert!(t.contact("yan@sf.test").await["time_zone"].is_null(), "an unknown zone is ignored and the signup still succeeds");
}

/// K0-6 and K0-7: a segment's preview stores nothing, and a rule reads only what its caller may read.
async fn segments_k0(t: &T, mgr: &Ctx) {
    // ───── K0-6 a_reader_previews_without_changing_the_segment ─────
    let reader = t
        .person("reader@acme.test", "Segment Reader", json!([{"entity": "MarketingSegment", "read": true}, {"entity": "MarketingContact", "read": true}]))
        .await;
    let everyone = uid(&t.insert(mgr, "MarketingSegment", json!({"segment_name": "Everyone", "match_type": "All", "rules": []})).await);
    let p = t.action(&reader, "MarketingSegment", everyone, "preview", json!({})).await;
    assert_eq!((p["contact_count"].as_i64(), p["mailable_count"].as_i64()), (Some(2), Some(2)), "{p}");
    assert_eq!(p["sample"].as_array().unwrap().len(), 2);
    let stored = t.get("MarketingSegment", everyone).await;
    assert!(stored["contact_count"].is_null() && stored["counted_at"].is_null(), "a preview stores no count: {stored:?}");
    // A saved query is previewed from its rows as they are now; its membership and schedule stay as they were.
    let query = uid(&t
        .insert(
            &t.admin,
            "MarketingSegment",
            json!({"segment_name": "All by query", "source": "Query", "query": {"entity": "MarketingContact", "filters": {}}, "refresh_minutes": 0}),
        )
        .await);
    let p = t.action(&reader, "MarketingSegment", query, "preview", json!({})).await;
    assert_eq!(p["contact_count"].as_i64(), Some(2), "{p}");
    let members = "select count(*) from campaigns_segment_member where tenant_id = $1 and segment_id = $2";
    assert_eq!(t.scalar::<i64>(members, Some(query)).await, 0, "a preview does not work out the membership");
    assert!(t.get("MarketingSegment", query).await["refreshed_at"].is_null());
    // Counting again is the same answer and still changes nothing (it runs in a read-only transaction).
    let again = t.action(&reader, "MarketingSegment", query, "preview", json!({})).await;
    assert_eq!(again["contact_count"], p["contact_count"]);
    // Whoever writes the segment stores its counts and membership; a reader cannot.
    refused(t.k.run_action(&reader, "MarketingSegment", query, "refresh", json!({})).await);
    let done = t.action(mgr, "MarketingSegment", query, "refresh", json!({})).await;
    assert_eq!((done["entered"].as_i64(), done["contact_count"].as_i64()), (Some(2), Some(2)), "{done}");
    assert_eq!(t.scalar::<i64>(members, Some(query)).await, 2);
    let stored = t.get("MarketingSegment", query).await;
    assert_eq!(stored["contact_count"], json!(2));
    assert!(!stored["counted_at"].is_null() && !stored["refreshed_at"].is_null(), "{stored:?}");
    // The schedule stores the counts too.
    t.exec("update e_marketing_segment set refresh_minutes = 5, contact_count = null, counted_at = null, refreshed_at = now() - interval '1 hour' where tenant_id = $1 and id = $2", Some(everyone)).await;
    t.maint().await;
    assert_eq!(t.get("MarketingSegment", everyone).await["contact_count"], json!(2));

    // ───── K0-7: caller-chosen fields pass the caller's own right to read ─────
    let fields = |extra: Value| {
        let mut f = vec![
            json!({"name": "title", "type": "data", "required": true}),
            json!({"name": "email", "type": "email"}),
            json!({"name": "tier", "type": "data"}),
        ];
        f.extend(extra.as_array().unwrap().iter().cloned());
        f
    };
    for (name, extra) in [
        ("XPerson", json!([{"name": "secret", "type": "data", "perm_group": "vip"}])),
        (
            "XOrder",
            json!([{"name": "amount", "type": "int", "perm_group": "vip"}, {"name": "status", "type": "data", "perm_group": "vip"},
                          {"name": "placed", "type": "date", "perm_group": "vip"}]),
        ),
    ] {
        let entity = json!({"name": name, "label": name, "kind": "master", "title_field": "title", "fields": fields(extra)});
        let custom: ferrum_kernel::customize::NewCustomization =
            serde_json::from_value(json!({"entity": name, "op": {"op": "define_entity", "entity": entity}})).unwrap();
        t.k.batch_customizations(&t.admin, vec![custom], false).await.map_err(|(_, e)| e).expect("the entity");
    }
    let admin = t.reload(&t.admin).await;
    let builder = t
        .person(
            "builder@acme.test",
            "Segment Builder",
            json!([{"entity": "MarketingSegment", "read": true, "write": true, "create": true}, {"entity": "MarketingContact", "read": true},
                   {"entity": "XPerson", "read": true}, {"entity": "XOrder", "read": true}]),
        )
        .await;
    let blind = t
        .person(
            "blind@acme.test",
            "Segment Writer",
            json!([{"entity": "MarketingSegment", "read": true, "write": true, "create": true}, {"entity": "MarketingContact", "read": true}]),
        )
        .await;
    let seg = |name: &str, rule: Value| json!({"segment_name": name, "match_type": "All", "rules": [rule]});
    let record =
        |entity: &str, field: &str| json!({"rule_type": "Record Field", "entity_name": entity, "field_name": field, "operator": "Equals", "value": "gold"});
    let related = |extra: Value| {
        let mut r = json!({"rule_type": "Related Records", "entity_name": "XOrder", "link_field": "email", "operator": "Greater Than", "value": "0"});
        for (k, v) in extra.as_object().unwrap() {
            r[k] = v.clone();
        }
        r
    };
    // a_segment_refuses_a_rule_on_a_field_the_caller_cannot_read: saving, and counting rules being edited
    refused(t.k.insert(&builder, "MarketingSegment", obj(seg("Secret", record("XPerson", "secret")))).await);
    let rules = |rule: Value| json!({"match_type": "All", "rules": [rule]});
    refused(t.k.run_action(&builder, "MarketingSegment", everyone, "count", rules(record("XPerson", "secret"))).await);
    for hidden in [
        related(json!({"aggregate": "Sum", "measure_field": "amount"})),
        related(json!({"match_field": "status", "match_value": "paid"})),
        related(json!({"date_field": "placed", "within_days": 30})),
    ] {
        refused(t.k.insert(&builder, "MarketingSegment", obj(seg("Hidden related", hidden.clone()))).await);
        refused(t.k.run_action(&builder, "MarketingSegment", everyone, "count", rules(hidden)).await);
    }
    // a field the caller may read still works, and so does the whole of it for someone who may read it all
    t.insert(&builder, "MarketingSegment", seg("Tier", record("XPerson", "tier"))).await;
    t.action(&builder, "MarketingSegment", everyone, "count", rules(related(json!({})))).await;
    let secret = uid(&t.insert(&admin, "MarketingSegment", seg("Secret by admin", record("XPerson", "secret"))).await);
    // an entity the caller may not read at all is as closed as a field
    refused(t.k.insert(&blind, "MarketingSegment", obj(seg("Tier", record("XPerson", "tier")))).await);
    refused(t.k.run_action(&blind, "MarketingSegment", everyone, "count", rules(related(json!({})))).await);
    // a_stored_segment_previews_only_what_the_caller_may_read: someone else's rule is no way to ask about a hidden field
    refused(t.k.run_action(&builder, "MarketingSegment", secret, "preview", json!({})).await);
    t.action(&admin, "MarketingSegment", secret, "preview", json!({})).await;
    // the sample names only columns the caller may read (none of a contact's is hidden today: the whole sample shows)
    let p = t.action(&builder, "MarketingSegment", everyone, "preview", json!({})).await;
    assert_eq!(p["sample"][0]["email"], json!("asha@x.test"));
    let _ = (Uuid::nil(), mgr);
}


fn headers(h: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    h.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// A connection made from the description the package offers, with a test secret.
async fn take_up(t: &T, name: &str, secret: &str) -> Uuid {
    let package = ferrum_app_campaigns::packages().unwrap().into_iter().find(|p| p.name() == "campaigns").unwrap();
    let offer = package.manifest.connections.iter().find(|c| c["name"] == json!(name)).unwrap().clone();
    let input: ferrum_kernel::connections::ConnectionInput =
        serde_json::from_value(json!({"name": name, "label": name, "description": offer["description"], "secrets": {"authorization": secret}, "enabled": true})).unwrap();
    t.k.save_connection_def(&t.admin, None, &input).await.expect("the connection")
}

async fn jobs(t: &T) {
    while t.k.run_jobs_once(4).await.unwrap() > 0 {}
}

async fn events(t: &T, email: &str) -> Vec<Doc> {
    t.list("DeliveryEvent", json!({"email": email})).await
}

/// campaigns-4: a provider's notices arrive through a connection described as data, once, and are applied once.
async fn provider_notices(t: &T, mgr: &Ctx) {
    const AUTH: &str = "Basic dXNlcjpwYXNzd29yZC0x";
    let conn = take_up(t, "postmark_bounces", AUTH).await;
    let ok = headers(&[("authorization", AUTH)]);
    let none = std::collections::BTreeMap::new();
    let notice = |kind: &str, id: i64, email: &str, what: &str| {
        json!({"RecordType": if kind == "SpamComplaint" { "SpamComplaint" } else { "Bounce" }, "ID": id, "Type": kind, "MessageID": format!("m-{id}"),
               "Email": email, "Description": what, "BouncedAt": "2026-11-20T10:00:00.1234567Z"}).to_string()
    };
    let post = |body: String| {
        let (k, ok, none) = (t.k.clone(), ok.clone(), none.clone());
        async move { k.connection_inbound(conn, &ok, &none, body.as_bytes()).await }
    };
    // a_postmark_hard_bounce_suppresses_the_address
    let hb = uid(&t.insert(mgr, "MarketingContact", json!({"email": "hb@x.test"})).await);
    let aud = at_domain(t, mgr, "x.test").await;
    let (campaign, _) = t.send(mgr, campaign("Provider", &aud)).await;
    let r = t.recipient_of(campaign, "hb@x.test").await;
    post(notice("HardBounce", 1, "hb@x.test", "The address does not exist")).await.expect("accepted");
    jobs(t).await;
    let e = events(t, "hb@x.test").await;
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!((e[0]["kind"].clone(), e[0]["outcome"].clone()), (json!("hard"), json!("Applied")), "{:?}", e[0]);
    assert_eq!(t.get("MarketingContact", hb).await["consent_status"], json!("Bounced"));
    let sup: i64 = t.scalar("select count(*) from e_email_suppression where tenant_id = $1 and email = 'hb@x.test' and reason = 'Hard Bounce' and $2::uuid is null", None).await;
    assert_eq!(sup, 1);
    assert_eq!(t.get("CampaignRecipient", r).await["bounce_type"], json!("Hard"));
    // a_postmark_complaint_marks_the_contact
    let cp = uid(&t.insert(mgr, "MarketingContact", json!({"email": "cp@x.test"})).await);
    post(notice("SpamComplaint", 2, "cp@x.test", "Spam")).await.expect("accepted");
    jobs(t).await;
    assert_eq!(t.get("MarketingContact", cp).await["consent_status"], json!("Complained"));
    // a_soft_bounce_counts_and_suppresses_at_the_limit (the workspace's limit is 3)
    let sb = uid(&t.insert(mgr, "MarketingContact", json!({"email": "sb@x.test"})).await);
    for (n, want) in [(10, 1), (11, 2)] {
        post(notice("SoftBounce", n, "sb@x.test", "Mailbox full")).await.expect("accepted");
        jobs(t).await;
        assert_eq!(t.get("MarketingContact", sb).await["soft_bounce_count"], json!(want));
    }
    post(notice("SoftBounce", 12, "sb@x.test", "Mailbox full")).await.expect("accepted");
    jobs(t).await;
    assert_eq!(t.get("MarketingContact", sb).await["consent_status"], json!("Bounced"));
    // a_notice_delivered_twice_is_applied_once, two_notices_with_one_event_id_make_one_event
    let body = notice("SoftBounce", 20, "twice@x.test", "Mailbox full");
    let twice = uid(&t.insert(mgr, "MarketingContact", json!({"email": "twice@x.test"})).await);
    post(body.clone()).await.expect("accepted");
    assert!(post(body).await.is_err(), "the same body again is a replay");
    post(notice("SoftBounce", 20, "twice@x.test", "Another description")).await.expect("another body, the same provider id");
    jobs(t).await;
    assert_eq!(events(t, "twice@x.test").await.len(), 1);
    assert_eq!(t.get("MarketingContact", twice).await["soft_bounce_count"], json!(1), "counted once");
    t.insert(mgr, "MarketingContact", json!({"email": "race@x.test"})).await;
    post(notice("SoftBounce", 21, "race@x.test", "first")).await.expect("accepted");
    post(notice("SoftBounce", 21, "race@x.test", "second")).await.expect("another body, the same provider id");
    let (a, b) = tokio::join!(t.k.run_jobs_once(1), t.k.run_jobs_once(1));
    a.unwrap();
    b.unwrap();
    jobs(t).await;
    assert_eq!(events(t, "race@x.test").await.len(), 1);
    // a_wrong_secret_is_refused_like_any_other
    let bad = t.k.connection_inbound(conn, &headers(&[("authorization", "Basic wrong")]), &none, notice("HardBounce", 31, "no@x.test", "x").as_bytes()).await.unwrap_err().to_string();
    let missing = t.k.connection_inbound(conn, &none, &none, notice("HardBounce", 32, "no@x.test", "x").as_bytes()).await.unwrap_err().to_string();
    assert_eq!(bad, missing, "every refusal reads the same");
    jobs(t).await;
    assert!(events(t, "no@x.test").await.is_empty(), "a refused call leaves nothing");
    // an_unknown_type_is_kept_and_ignored
    let un = uid(&t.insert(mgr, "MarketingContact", json!({"email": "un@x.test"})).await);
    post(notice("AutoResponder", 40, "un@x.test", "Out of office")).await.expect("accepted");
    jobs(t).await;
    assert_eq!(events(t, "un@x.test").await[0]["outcome"], json!("Ignored"));
    assert_eq!(t.get("MarketingContact", un).await["consent_status"], json!("Subscribed"));
    // a_bounce_for_an_address_never_mailed_still_suppresses
    post(notice("HardBounce", 50, "ghost@y.test", "x")).await.expect("accepted");
    jobs(t).await;
    let g = &events(t, "ghost@y.test").await[0];
    assert!(g["outcome"] == json!("Applied") && g["recipient"].is_null(), "{g:?}");
    let sup: i64 = t.scalar("select count(*) from e_email_suppression where tenant_id = $1 and email = 'ghost@y.test' and $2::uuid is null", None).await;
    assert_eq!(sup, 1);
    // a_sparkpost_batch_is_mapped_item_by_item
    let spark = take_up(t, "sparkpost_bounces", AUTH).await;
    for e in ["sp1@z.test", "sp2@z.test", "sp3@z.test"] {
        t.insert(mgr, "MarketingContact", json!({"email": e})).await;
    }
    let ev = |kind: &str, class: &str, id: &str, to: &str| json!({"msys": {"message_event": {"type": kind, "bounce_class": class, "rcpt_to": to, "message_id": format!("sp-{id}"), "event_id": id, "raw_reason": "x"}}});
    let batch = json!([ev("bounce", "10", "e1", "sp1@z.test"), ev("spam_complaint", "", "e2", "sp2@z.test"), ev("bounce", "51", "e3", "sp3@z.test")]);
    t.k.connection_inbound(spark, &ok, &none, batch.to_string().as_bytes()).await.expect("accepted");
    jobs(t).await;
    let kinds: Vec<(String, String)> = {
        let mut v = Vec::new();
        for e in ["sp1@z.test", "sp2@z.test", "sp3@z.test"] {
            let d = &events(t, e).await[0];
            v.push((d["kind"].as_str().unwrap().to_string(), d["outcome"].as_str().unwrap().to_string()));
        }
        v
    };
    assert_eq!(kinds, [("hard".into(), "Applied".into()), ("complaint".into(), "Applied".into()), ("class:51".into(), "Ignored".into())]);
    // nobody_writes_a_delivery_event_by_hand
    let id = uid(&events(t, "hb@x.test").await[0]);
    refused(t.k.insert(mgr, "DeliveryEvent", obj(json!({"event_key": "by-hand", "email": "a@b.test", "kind": "hard"}))).await);
    refused(t.k.update(mgr, "DeliveryEvent", id, obj(json!({"outcome": "Ignored"})), None).await);
    refused(t.k.delete(mgr, "DeliveryEvent", id, None).await);
    refused(t.k.delete(&t.admin, "DeliveryEvent", id, None).await);
    // erasure_reaches_delivery_events
    t.k.erase_personal_data(&t.admin, "un@x.test", "his request").await.expect("erasure");
    assert!(events(t, "un@x.test").await.is_empty(), "no event keeps the erased address");
}


fn design(heading: &str, href: &str) -> Value {
    json!({"version": 1, "rows": [{"columns": [{"blocks": [
        {"type": "heading", "text": heading, "level": 1},
        {"type": "button", "label": "Read more", "href": href},
        {"type": "text", "html": "<p><a href=\"https://example.com/a\">A link</a></p>"}]}]}]})
}

/// campaigns-5: a design is the email's source and `content` is what it renders to.
async fn designs(t: &T, mgr: &Ctx) {
    people(t, mgr, "d.test", 2).await;
    let aud = at_domain(t, mgr, "d.test").await;
    // a_campaign_saved_with_a_design_gets_its_content
    let c = uid(&t.insert(mgr, "EmailCampaign", json!({"campaign_name": "Designed", "subject": "Hi", "design": design("Hello", "https://shop.test/sale"), "audience": aud})).await);
    let html = t.get("EmailCampaign", c).await["content"].as_str().unwrap().to_string();
    assert!(html.contains("Hello") && html.contains("https://shop.test/sale"), "{html}");
    t.update(mgr, "EmailCampaign", c, json!({"design": design("Changed", "https://shop.test/sale")})).await;
    assert!(t.get("EmailCampaign", c).await["content"].as_str().unwrap().contains("Changed"));
    t.update(mgr, "EmailCampaign", c, json!({"content": "<p>mine</p>"})).await;
    assert!(t.get("EmailCampaign", c).await["content"].as_str().unwrap().contains("Changed"), "the design wins over what is written beside it");
    t.update(mgr, "EmailCampaign", c, json!({"design": null, "content": "<p>mine</p>"})).await;
    assert_eq!(t.get("EmailCampaign", c).await["content"], json!("<p>mine</p>"), "without a design the content is as written");
    refused(t.k.update(mgr, "EmailCampaign", c, obj(json!({"design": {"version": 2, "rows": []}})), None).await);
    // a_designed_campaign_is_sent_like_any_other
    let (sent, to) = t.send(mgr, json!({"campaign_name": "Designed send", "subject": "Hi", "design": design("Hello", "https://shop.test/sale"), "audience": aud})).await;
    assert_eq!(to.len(), 2);
    let preview = t.action(mgr, "EmailCampaign", sent, "preview", json!({})).await;
    assert!(preview["html"].as_str().unwrap().contains("Hello"), "{preview}");
    let comm: Uuid = t
        .scalar(&format!("select communication from e_campaign_recipient where tenant_id = $1 and campaign = '{sent}' and communication is not null and $2::uuid is null limit 1"), None)
        .await;
    let body: String = t.scalar(&format!("select content from e_communication where tenant_id = $1 and id = '{comm}' and $2::uuid is null"), None).await;
    assert!(body.contains("/c/") && body.contains("/o/") && body.to_lowercase().contains("unsubscribe"), "a tracked link, the pixel and the footer: {body}");
    assert!(!body.contains("https://shop.test/sale"), "the button's address is the tracked one");
    // a_version_with_its_own_design
    let v = uid(&t
        .insert(
            mgr,
            "EmailCampaign",
            json!({"campaign_name": "Versions", "subject": "Hi", "content": "<p>a</p>", "audience": aud, "ab_test": true,
                   "variants": [{"subject": "A"}, {"subject": "B", "design": design("Version B", "https://shop.test/b")}]}),
        )
        .await);
    let b = t.get("EmailCampaign", v).await["variants"][1]["content"].as_str().unwrap().to_string();
    assert!(b.contains("Version B"), "{b}");
    let bad = t
        .k
        .insert(mgr, "EmailCampaign", obj(json!({"campaign_name": "Bad version", "subject": "Hi", "content": "<p>a</p>", "audience": aud, "ab_test": true,
            "variants": [{"subject": "A"}, {"subject": "B", "design": {"version": 2, "rows": []}}]})))
        .await;
    refused(bad);
    // a_marketing_user_cannot_design_someone_elses_campaign
    let uma = t.person("drafter@acme.test", "Campaign Drafter", json!([{"entity": "EmailCampaign", "read": true, "write": true, "create": true, "if_owner": true}])).await;
    refused(t.k.update(&uma, "EmailCampaign", c, obj(json!({"design": design("Mine", "https://shop.test/sale")})), None).await);
}
