mod common;

use common::World;
use serde_json::{Value, json};

fn ids(items: &Value) -> Vec<String> {
    items
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn every_screen_query_validates_and_executes() {
    let w = World::new().await;
    let tenant = json!({ "tenant": "acme" });

    let shell = w.ok(&w.mika, "Shell", json!({})).await;
    assert_eq!(shell["viewer"]["user"]["githubLogin"], "mika-dev");
    assert_eq!(shell["viewer"]["isOperator"], false);
    assert_eq!(shell["viewer"]["memberships"][0]["role"], "OWNER");
    assert_eq!(shell["viewer"]["memberships"][0]["tenant"]["slug"], "acme");

    let home = w.ok(&w.mika, "Home", tenant.clone()).await;
    let home = &home["tenant"];
    assert_eq!(home["plan"]["name"], "Pro");
    assert_eq!(home["plan"]["limitPeriod"], "MONTH");
    assert_eq!(home["usage"]["tasksThisPeriod"], 7);
    assert!(home["usage"]["periodEnd"].is_string());
    assert_eq!(home["usage"]["runningRuns"], 2);
    assert_eq!(
        home["activity"]["runsLast24h"].as_array().unwrap().len(),
        24
    );
    assert_eq!(home["activity"]["pullRequestsOpenedToday"], 1);
    assert_eq!(home["activity"]["costTodayUsd"], 1.25);
    assert_eq!(home["setup"]["githubLinked"], true);
    assert_eq!(home["setup"]["appInstalled"], true);
    assert_eq!(home["setup"]["slackConnected"], false);
    assert_eq!(home["setup"]["complete"], true);
    let tasks = home["tasks"].as_array().unwrap();
    assert_eq!(tasks[0]["status"], "NEEDS_YOU");
    assert_eq!(tasks[0]["needsYouReason"], "SKILL_FAILED");
    assert_eq!(tasks[0]["issue"]["number"], 35);
    assert_eq!(tasks[0]["turns"], 12);
    assert_eq!(tasks[0]["limits"]["turns"], 60);
    assert_eq!(tasks[0]["requiredSkills"][0]["state"], "FAILED");
    assert_eq!(tasks[0]["requiredSkills"][0]["latestRun"]["handoffs"], 2);
    assert_eq!(tasks[1]["channel"]["kind"], "WEB");
    assert_eq!(tasks[1]["channel"]["scope"], "all");
    assert_eq!(tasks.len(), 2);
    let credentials = home["credentials"].as_array().unwrap();
    assert_eq!(
        credentials
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["acme-claude-team", "mika-codex"]
    );
    assert_eq!(credentials[0]["usage"]["sevenDay"]["utilization"], 0.71);
    assert_eq!(credentials[1]["status"], "RECONNECT");
    assert_eq!(home["changeRequests"][0]["number"], 7);
    assert_eq!(
        home["changeRequests"][0]["openedFromTask"]["id"],
        w.chat_task.to_string()
    );
    assert_eq!(home["integrations"][0]["status"], "CONNECTED");
    assert_eq!(home["integrations"][1]["kind"], "SLACK");
    assert_eq!(home["integrations"][1]["status"], "NOT_CONNECTED");
    assert_eq!(home["triggers"].as_array().unwrap().len(), 3);

    let all = w
        .ok(
            &w.mika,
            "Tasks",
            json!({ "tenant": "acme", "filter": null }),
        )
        .await;
    assert_eq!(all["tenant"]["tasks"].as_array().unwrap().len(), 2);
    assert_eq!(all["tenant"]["skills"][0]["kind"], "RELIABLE");
    let filtered = w
        .ok(
            &w.mika,
            "Tasks",
            json!({ "tenant": "acme", "filter": { "skillName": "open-pr-green-ci" } }),
        )
        .await;
    assert_eq!(
        ids(&filtered["tenant"]["tasks"]),
        vec![w.issue_task.to_string()]
    );
    let by_trigger = w
        .ok(
            &w.mika,
            "Tasks",
            json!({ "tenant": "acme", "filter": { "triggerId": w.chat_trigger, "statuses": ["WAITING"] } }),
        )
        .await;
    assert_eq!(
        ids(&by_trigger["tenant"]["tasks"]),
        vec![w.chat_task.to_string()]
    );

    let task = w
        .ok(
            &w.mika,
            "Task",
            json!({ "tenant": "acme", "id": w.issue_task }),
        )
        .await;
    let task = &task["tenant"]["task"];
    assert_eq!(task["name"], "acme/shop-web#35");
    assert_eq!(task["maxNudges"], 2);
    assert_eq!(task["configRevision"], 1);
    assert_eq!(task["triggerEvent"]["senderLogin"], "mika-dev");
    assert_eq!(task["requester"]["kind"], "MEMBER");
    assert_eq!(task["requester"]["member"]["user"]["name"], "Mika");
    assert_eq!(task["session"]["name"], "shop");
    assert_eq!(task["skillRuns"][0]["stateEntryCount"], 4);
    assert_eq!(
        task["skillRuns"][0]["stateEntries"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(task["skillRuns"][0]["stateEntries"][3]["stateType"], "FAIL");
    assert_eq!(task["skillRuns"][0]["error"], "Skill.FixesExhausted");
    assert_eq!(task["runs"][0]["number"], 1);
    assert_eq!(task["outputs"][0]["kind"], "PULL_REQUEST");

    let run = w
        .ok(
            &w.mika,
            "Run",
            json!({ "tenant": "acme", "task": w.issue_task, "number": 1 }),
        )
        .await;
    let run = &run["tenant"]["task"]["run"];
    assert_eq!(run["credentialName"], "acme-claude-team");
    assert_eq!(run["workspace"]["platform"], "fargate");
    assert_eq!(run["workspace"]["vcpu"], 2);
    let kinds = run["log"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line["kind"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec!["SYSTEM", "INSTRUCTION", "EVENT", "INSTRUCTION", "SYSTEM"]
    );
    assert_eq!(run["skillRuns"][0]["skill"]["name"], "open-pr-green-ci");
    assert_eq!(run["outputs"][0]["number"], 52);

    let chat = w
        .ok(
            &w.mika,
            "Chat",
            json!({ "tenant": "acme", "task": w.chat_task, "withTask": true }),
        )
        .await;
    let chat = &chat["tenant"];
    assert_eq!(ids(&chat["chats"]), vec![w.chat_task.to_string()]);
    let messages = chat["task"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["authorKind"], "MEMBER");
    assert_eq!(messages[0]["member"]["user"]["name"], "Mika");
    assert_eq!(messages[1]["authorKind"], "AGENT");
    assert_eq!(messages[1]["text"], "The checkout spec is flaky.");
    assert_eq!(
        chat["task"]["session"]["repositories"],
        json!(["acme/idea"])
    );
    let without = w
        .ok(
            &w.mika,
            "Chat",
            json!({ "tenant": "acme", "task": w.chat_task, "withTask": false }),
        )
        .await;
    assert!(without["tenant"].get("task").is_none());

    let skills = w.ok(&w.mika, "Skills", tenant.clone()).await;
    let skill = &skills["tenant"]["skills"][0];
    assert_eq!(skill["requiredBy"][0]["name"], "labeled-issue");
    assert_eq!(skill["stats"]["runs"], 1);
    assert_eq!(skill["stats"]["succeeded"], 0);
    assert_eq!(
        skills["tenant"]["configs"][0]["newSkillUrl"],
        "https://github.com/acme/sanchoris-config/new/main/skills"
    );

    let detail = w
        .ok(
            &w.mika,
            "Skill",
            json!({ "tenant": "acme", "name": "open-pr-green-ci" }),
        )
        .await;
    let detail = &detail["tenant"]["skill"];
    assert_eq!(detail["startAt"], "Init");
    assert_eq!(detail["editUrl"], Value::Null);
    let states = detail["states"].as_array().unwrap();
    let give_up = states.iter().find(|s| s["name"] == "GiveUp").unwrap();
    assert_eq!(give_up["type"], "FAIL");
    assert_eq!(give_up["activity"]["needsYou"], 0);
    let push = states.iter().find(|s| s["name"] == "Push").unwrap();
    assert_eq!(push["x"], 240);
    assert_eq!(push["resource"], "sanchoris:::git:push");
    let hand_off = states.iter().find(|s| s["name"] == "HandOff").unwrap();
    assert_eq!(hand_off["activity"]["succeededToday"], 2);
    assert_eq!(
        detail["stats"]["failureErrors"],
        json!(["Skill.FixesExhausted"])
    );
    assert_eq!(detail["stats"]["medianHandoffs"], 2);
    assert_eq!(detail["recentRuns"][0]["number"], 1);

    let triggers = w.ok(&w.mika, "Triggers", tenant.clone()).await;
    let list = triggers["tenant"]["triggers"].as_array().unwrap();
    let labeled = list.iter().find(|t| t["name"] == "labeled-issue").unwrap();
    assert_eq!(labeled["match"]["source"], "GITHUB");
    assert_eq!(labeled["match"]["kind"], "ISSUE_LABELED");
    assert_eq!(labeled["match"]["senders"], json!(["mika-dev", "sota-dev"]));
    assert_eq!(labeled["requiredSkills"][0]["name"], "open-pr-green-ci");
    assert_eq!(labeled["openChangeRequest"]["number"], 7);
    assert_eq!(labeled["tasksLast7d"], 1);
    assert_eq!(
        labeled["editUrl"],
        "https://github.com/acme/sanchoris-config/edit/main/sanchoris.yaml"
    );
    let nightly = list.iter().find(|t| t["name"] == "nightly").unwrap();
    assert_eq!(nightly["match"]["source"], "SCHEDULE");
    assert!(nightly["match"]["nextRunAt"].is_string());
    let events = triggers["tenant"]["triggerEvents"].as_array().unwrap();
    assert_eq!(events.len(), 3);
    let skipped = events.iter().find(|e| e["result"] == "SKIPPED").unwrap();
    assert_eq!(skipped["reason"], "SENDER_NOT_ALLOWED");
    assert_eq!(skipped["senderLogin"], "rin-dev");
    assert_eq!(triggers["triggerTemplates"].as_array().unwrap().len(), 3);

    let integrations = w.ok(&w.mika, "Integrations", tenant.clone()).await;
    assert_eq!(
        integrations["tenant"]["integrations"][0]["manageUrl"],
        "https://github.com/organizations/acme/settings/installations/1001"
    );

    let configs = w.ok(&w.sota, "Configs", tenant.clone()).await;
    let config = &configs["tenant"]["configs"][0];
    assert_eq!(config["revision"]["number"], 1);
    assert_eq!(config["revision"]["validation"]["valid"], true);
    assert_eq!(config["triggerCount"], 3);
    assert_eq!(config["skillCount"], 1);
    let project = &configs["tenant"]["projects"][0];
    assert_eq!(project["repositories"].as_array().unwrap().len(), 2);
    assert_eq!(project["warmBuild"]["succeeded"], true);
    assert_eq!(project["sessions"][0]["name"], "shop");
    assert_eq!(
        configs["tenant"]["changeRequests"][0]["changes"],
        json!([{ "kind": "TRIGGER", "name": "labeled-issue" }])
    );

    let credentials = w.ok(&w.mika, "Credentials", tenant.clone()).await;
    assert_eq!(
        credentials["tenant"]["credentials"][1]["owner"]["user"]["name"],
        "Mika"
    );
    assert!(credentials["tenant"]["credentials"][1]["rejectedAt"].is_string());

    let members = w.ok(&w.ken, "Members", tenant.clone()).await;
    let members = &members["tenant"];
    assert_eq!(members["usage"]["seatsUsed"], 4);
    assert_eq!(members["viewerMember"]["role"], "MEMBER");
    let roles = members["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(roles, vec!["OWNER", "ADMIN", "MEMBER"]);
    assert_eq!(members["members"][0]["tasksStarted"], 2);
    assert_eq!(
        members["members"][2]["credentials"][0]["name"],
        "ken-claude"
    );
    assert_eq!(members["invitations"][0]["email"], "ren@acme.dev");
    assert_eq!(
        members["invitations"][0]["invitedBy"]["user"]["name"],
        "Mika"
    );

    let billing = w.ok(&w.mika, "Billing", tenant.clone()).await;
    assert_eq!(billing["tenant"]["plan"]["priceUsd"], 100);
    assert_eq!(billing["plans"][0]["name"], "Free");
    assert_eq!(billing["plans"][3]["workspaceHourLimit"], 500);

    let page = w.ok(&w.ren, "CreateTenantPage", json!({})).await;
    assert_eq!(page["viewer"]["invitations"][0]["tenantName"], "Acme");
    assert_eq!(
        page["viewer"]["invitations"][0]["invitedBy"]["user"]["name"],
        "Mika"
    );
    assert_eq!(page["plans"][0]["requiresPayment"], false);

    w.ok(
        &w.mika,
        "SetTriggerState",
        json!({ "triggerId": w.labeled_trigger, "state": "DISABLED" }),
    )
    .await;
    let audit = w
        .ok(
            &w.mika,
            "AuditLog",
            json!({ "tenant": "acme", "filter": null }),
        )
        .await;
    let event = &audit["tenant"]["auditEvents"][0];
    assert_eq!(event["actorKind"], "MEMBER");
    assert_eq!(event["actorName"], "Mika");
    assert_eq!(event["action"], "trigger.disable");
    assert_eq!(event["target"], "trigger labeled-issue");
    assert_eq!(event["origin"], "WEB");

    let operator = w.ok(&w.operator, "OperatorTenants", json!({})).await;
    let tenants = operator["operator"]["tenants"].as_array().unwrap();
    assert_eq!(tenants.len(), 2);
    assert_eq!(tenants[0]["name"], "Acme");
    assert_eq!(tenants[0]["githubOwner"]["kind"], "ORGANIZATION");
    assert_eq!(tenants[0]["usage"]["seatsUsed"], 4);
    assert_eq!(operator["operator"]["runningRuns"], 2);
    assert_eq!(operator["operator"]["revenueThisMonthUsd"], 100);
}

#[tokio::test]
async fn timestamps_use_utc_with_a_z_suffix() {
    let w = World::new().await;
    let data = w
        .ok(
            &w.mika,
            "Task",
            json!({ "tenant": "acme", "id": w.issue_task }),
        )
        .await;
    let updated = data["tenant"]["task"]["updatedAt"].as_str().unwrap();
    assert!(updated.ends_with('Z'), "{updated}");
    assert!(!updated.contains('.'), "{updated}");
}
