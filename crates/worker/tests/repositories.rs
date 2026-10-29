use crate::common;

use std::collections::BTreeMap;

use aws_sdk_dynamodb::types::AttributeValue;
use common::{SHOP_API_ID, SHOP_WEB_ID, World, labeled_payload};
use sanchoris_core::item::{Entity, Key, Table};
use sanchoris_core::items::{
    AuditEvent, CONFIG_ACTIVE, CONFIG_DISABLED, Config, GitHubRepository, Guard, Integration,
    Project, Repository, SessionDef, SkillDef, Task, TaskName, Trigger, github_repositories,
};
use sanchoris_core::jobs::{Envelope, Job};
use sanchoris_core::keys;
use sanchoris_core::model::TaskStatus;
use sanchoris_core::testing::SEED_CONFIG_REPOSITORY_ID;
use sanchoris_worker::backfill;
use sanchoris_worker::config_sync::{disable_config, rename_repository};
use sanchoris_worker::ports::IssueRef;
use serde_json::{Value, json};
use uuid::Uuid;

fn payload_from(number: i64, repository: &GitHubRepository) -> Value {
    let mut payload = labeled_payload(number, "Fix flaky checkout test", "mika-dev");
    payload["repository"] = json!({ "id": repository.id, "full_name": repository.full_name });
    payload
}

async fn config(world: &World, config_id: Uuid) -> Config {
    world
        .store
        .get::<Config>(&keys::config_key(world.tenant(), config_id))
        .await
        .unwrap()
        .expect("the config exists")
}

async fn sync(world: &World, config_id: Uuid) {
    world
        .worker
        .worker
        .handle(&Envelope::new(Job::ConfigSync {
            tenant_id: world.tenant(),
            config_id,
        }))
        .await
        .unwrap();
}

async fn install(world: &World, repository: &GitHubRepository) {
    let integration = world.dynamo.all::<Integration>().await.remove(0);
    world
        .dynamo
        .modify::<Integration>(&integration.key(), |integration| {
            let mut repositories = github_repositories(&integration.details).unwrap();
            repositories.retain(|r| r.id != repository.id);
            repositories.push(repository.clone());
            integration.details["repositories"] = json!(repositories);
        })
        .await;
}

async fn add_config(world: &World, repository: &GitHubRepository) -> Uuid {
    let id = Uuid::new_v4();
    let now = world.clock_now();
    world
        .dynamo
        .put(&Config {
            id,
            tenant_id: world.tenant(),
            repository_id: Some(repository.id),
            repository: repository.full_name.clone(),
            root_path: "/".into(),
            branch: "main".into(),
            status: CONFIG_ACTIVE.into(),
            active_revision_id: None,
            revisions: 0,
            repositories: BTreeMap::new(),
            synced_at: None,
            sync_error: None,
            created_at: now,
            updated_at: now,
        })
        .await;
    world
        .dynamo
        .put(&Guard::new(
            keys::config_repository_key(world.tenant(), repository.id, "/"),
            id,
        ))
        .await;
    id
}

fn trigger_of(config_id: Uuid) -> Uuid {
    keys::trigger_id(config_id, "labeled-issue")
}

#[tokio::test]
async fn activation_resolves_the_named_repositories_to_ids() {
    let world = World::new("pro").await;
    let config = config(&world, world.seeded.config_id).await;
    assert_eq!(config.repository_id, Some(SEED_CONFIG_REPOSITORY_ID));
    assert_eq!(config.sync_error, None);
    let named = config
        .repositories
        .iter()
        .map(|(name, repository)| (name.as_str(), repository.id))
        .collect::<Vec<_>>();
    assert_eq!(
        named,
        vec![
            ("acme/idea", common::IDEA_ID),
            ("acme/shop-api", SHOP_API_ID),
            ("acme/shop-web", SHOP_WEB_ID),
        ]
    );
    let mut repositories = world
        .dynamo
        .all::<Repository>()
        .await
        .into_iter()
        .map(|r| (r.name, r.github_repository_id, r.full_name))
        .collect::<Vec<_>>();
    repositories.sort();
    assert_eq!(
        repositories,
        vec![
            ("backend".into(), SHOP_API_ID, "acme/shop-api".into()),
            ("frontend".into(), SHOP_WEB_ID, "acme/shop-web".into()),
        ]
    );
}

#[tokio::test]
async fn a_renamed_repository_keeps_starting_its_triggers_by_id() {
    let world = World::new("pro").await;
    let renamed = world.github.rename(SHOP_WEB_ID, "acme/storefront");
    world
        .deliver("delivery-renamed", "issues", &payload_from(70, &renamed))
        .await;
    world.worker.settle().await.unwrap();
    let tasks = world.tasks().await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, format!("github:{SHOP_WEB_ID}#70"));
    assert_eq!(
        tasks[0].display_name(std::slice::from_ref(&renamed)),
        "acme/storefront#70"
    );
    assert_eq!(tasks[0].trigger_id, trigger_of(world.seeded.config_id));
}

#[tokio::test]
async fn a_sub_issue_finds_its_parent_task_created_before_a_rename() {
    let world = World::new("pro").await;
    world
        .label_issue("delivery-35", 35, "Parent", "mika-dev")
        .await;
    world.worker.settle().await.unwrap();
    let parent = world.task("acme/shop-web#35").await;
    let renamed = world.github.rename(SHOP_WEB_ID, "acme/storefront");
    install(&world, &renamed).await;
    world.github.set_parent_ref(
        &renamed.full_name,
        36,
        IssueRef {
            repository: renamed.full_name.clone(),
            repository_id: SHOP_WEB_ID,
            number: 35,
        },
    );
    world
        .deliver("delivery-36", "issues", &payload_from(36, &renamed))
        .await;
    world.worker.settle().await.unwrap();
    let child = world
        .tasks()
        .await
        .into_iter()
        .find(|task| task.name == format!("github:{SHOP_WEB_ID}#36"))
        .expect("the sub-issue started a task");
    assert_eq!(child.parent_task_id, Some(parent.id));
    assert_eq!(child.display_name(&[renamed]), "acme/storefront#36");
}

#[tokio::test]
async fn configs_on_swapped_names_do_not_start_each_others_triggers() {
    let world = World::new("pro").await;
    let old = world.github.rename(SHOP_WEB_ID, "acme/shop-web-dev");
    let taken = world.github.add_repository(7201, "acme/shop-web");
    install(&world, &old).await;
    install(&world, &taken).await;
    let second = add_config(&world, &taken).await;
    sync(&world, second).await;
    assert_eq!(
        config(&world, second)
            .await
            .repository_named("acme/shop-web")
            .map(|r| r.id),
        Some(7201)
    );
    assert_eq!(
        config(&world, world.seeded.config_id)
            .await
            .repository_named("acme/shop-web")
            .map(|r| r.id),
        Some(SHOP_WEB_ID)
    );

    world
        .deliver("delivery-taken", "issues", &payload_from(71, &taken))
        .await;
    world
        .deliver("delivery-old", "issues", &payload_from(72, &old))
        .await;
    world.worker.settle().await.unwrap();

    let mut started = world
        .tasks()
        .await
        .into_iter()
        .map(|task| (task.name, task.trigger_id))
        .collect::<Vec<_>>();
    started.sort();
    assert_eq!(
        started,
        vec![
            (
                format!("github:{SHOP_WEB_ID}#72"),
                trigger_of(world.seeded.config_id)
            ),
            ("github:7201#71".to_string(), trigger_of(second)),
        ]
    );
}

#[tokio::test]
async fn a_rename_updates_the_cached_names() {
    let world = World::new("pro").await;
    let config_repository = world
        .github
        .rename(SEED_CONFIG_REPOSITORY_ID, "acme/platform-config");
    let shop = world.github.rename(SHOP_WEB_ID, "acme/storefront");
    let now = world.clock_now();
    rename_repository(&world.store, world.tenant(), &config_repository, now)
        .await
        .unwrap();
    rename_repository(&world.store, world.tenant(), &shop, now)
        .await
        .unwrap();
    let config = config(&world, world.seeded.config_id).await;
    assert_eq!(config.repository, "acme/platform-config");
    assert_eq!(
        config
            .repository_named("acme/shop-web")
            .map(|r| r.full_name.as_str()),
        Some("acme/storefront")
    );
    let frontend = world
        .dynamo
        .all::<Repository>()
        .await
        .into_iter()
        .find(|r| r.name == "frontend")
        .unwrap();
    assert_eq!(frontend.full_name, "acme/storefront");
    assert!(
        world
            .dynamo
            .all::<AuditEvent>()
            .await
            .iter()
            .any(|e| e.action == "config.rename"
                && e.summary
                    == "The config repository acme/sanchoris-config is now acme/platform-config")
    );
}

#[tokio::test]
async fn a_disabled_config_retires_its_definitions_and_stops_its_tasks() {
    let world = World::new("pro").await;
    let running = world.started_task(73).await;
    let reason = "acme/sanchoris-config was removed from the GitHub App installation";
    disable_config(
        &world.store,
        world.tenant(),
        world.seeded.config_id,
        reason,
        world.clock_now(),
    )
    .await
    .unwrap();
    world.worker.settle().await.unwrap();

    let disabled = config(&world, world.seeded.config_id).await;
    assert_eq!(disabled.status, CONFIG_DISABLED);
    assert_eq!(disabled.sync_error.as_deref(), Some(reason));
    assert!(
        world
            .store
            .get::<Guard>(&keys::config_repository_key(
                world.tenant(),
                SEED_CONFIG_REPOSITORY_ID,
                "/"
            ))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        world
            .dynamo
            .all::<Trigger>()
            .await
            .iter()
            .all(|t| t.removed_at.is_some())
    );
    assert!(world.dynamo.all::<SessionDef>().await.is_empty());
    assert!(world.dynamo.all::<SkillDef>().await.is_empty());
    assert!(world.dynamo.all::<Project>().await.is_empty());
    assert!(world.dynamo.all::<Repository>().await.is_empty());
    let task = world
        .tasks()
        .await
        .into_iter()
        .find(|t| t.id == running.id)
        .unwrap();
    assert_eq!(task.status, TaskStatus::Aborted);

    world
        .deliver(
            "delivery-after",
            "issues",
            &labeled_payload(74, "Another", "mika-dev"),
        )
        .await;
    world.worker.settle().await.unwrap();
    assert_eq!(world.tasks().await.len(), 1);
    world.resync_config().await;
    assert_eq!(
        world
            .worker
            .worker
            .handle(&Envelope::new(Job::SweepConfigs))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn sync_errors_are_recorded_and_cleared_by_the_next_sync() {
    let world = World::new("pro").await;
    {
        let mut state = world.github.state.lock().unwrap();
        state.head_sha = "config-ghost".into();
        let ghost = common::CONFIG_YAML.replace("Repos: [acme/idea]", "Repos: [acme/ghost]");
        state.files.insert("sanchoris.yaml".into(), ghost);
    }
    world.resync_config().await;
    sync(&world, world.seeded.config_id).await;
    let failed = config(&world, world.seeded.config_id).await;
    assert_eq!(
        failed.sync_error.as_deref(),
        Some("acme/ghost is not a repository of the tenant's GitHub App installation")
    );
    assert_eq!(failed.status, CONFIG_ACTIVE);

    {
        let mut state = world.github.state.lock().unwrap();
        state.head_sha = "config-1".into();
        state
            .files
            .insert("sanchoris.yaml".into(), common::CONFIG_YAML.into());
    }
    sync(&world, world.seeded.config_id).await;
    assert_eq!(
        config(&world, world.seeded.config_id).await.sync_error,
        None
    );
}

#[tokio::test]
async fn the_backfill_moves_definitions_under_the_active_revision_and_task_names_to_ids() {
    let world = World::new("pro").await;
    let tenant_id = world.tenant();
    let task = world.started_task(35).await;
    let active = config(&world, world.seeded.config_id)
        .await
        .active_revision_id
        .unwrap();
    let prefix = keys::definitions_prefix(active);
    let mut legacy_keys = Vec::new();
    for kind in ["project", "repository", "session", "skill"] {
        for mut item in world.dynamo.scan(Table::Main, kind).await {
            let key = Key::of(Table::Main, &item).unwrap();
            let mut tx = world.store.tx(tenant_id);
            tx.get_item(&key).await.unwrap();
            tx.delete(&key).unwrap();
            tx.commit().await.unwrap();
            let legacy_sk = key.sk.strip_prefix(&prefix).unwrap().to_string();
            item.insert("sk".into(), AttributeValue::S(legacy_sk.clone()));
            if kind == "repository" {
                item.remove("config_revision_id");
            }
            world.dynamo.put_raw(item).await;
            legacy_keys.push(Key::main(key.pk, legacy_sk));
        }
    }
    let stored = world
        .store
        .get::<Task>(&keys::task_key(tenant_id, task.id))
        .await
        .unwrap()
        .unwrap();
    let new_guard = keys::task_name_key(tenant_id, stored.trigger_id, &stored.name);
    let old_guard = keys::task_name_key(tenant_id, stored.trigger_id, "acme/shop-web#35");
    world
        .dynamo
        .modify::<Task>(&stored.key(), |task| {
            task.name = "acme/shop-web#35".into();
            task.channel = Some(json!({ "source": "github", "repository": "acme/shop-web" }));
        })
        .await;
    let mut tx = world.store.tx(tenant_id);
    let guard = tx.require::<TaskName>(&new_guard).await.unwrap();
    tx.delete(&new_guard).unwrap();
    tx.insert(&TaskName {
        name: "acme/shop-web#35".into(),
        ..guard
    })
    .unwrap();
    tx.commit().await.unwrap();

    let report = backfill::stable_keys(&world.worker.worker).await.unwrap();
    world.worker.settle().await.unwrap();
    assert!(
        report
            .lines
            .iter()
            .any(|line| line.ends_with("acme/shop-web#35 -> github:7101#35")),
        "{:?}",
        report.lines
    );
    for key in &legacy_keys {
        assert!(
            world.store.get_item(key).await.unwrap().is_none(),
            "{key:?}"
        );
    }
    let sessions = world
        .store
        .query::<SessionDef>(
            &keys::config_pk(tenant_id, world.seeded.config_id),
            &keys::sessions_prefix(active),
        )
        .await
        .unwrap();
    assert!(!sessions.is_empty());
    assert!(
        world
            .dynamo
            .all::<Repository>()
            .await
            .iter()
            .all(|r| r.config_revision_id == active)
    );
    let moved = world
        .store
        .get::<Task>(&keys::task_key(tenant_id, task.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(moved.name, "github:7101#35");
    assert_eq!(moved.channel.unwrap()["repository_id"], SHOP_WEB_ID);
    assert!(world.store.get_item(&old_guard).await.unwrap().is_none());
    assert!(world.store.get_item(&new_guard).await.unwrap().is_some());
    assert_eq!(world.task("acme/shop-web#35").await.id, task.id);

    let again = backfill::stable_keys(&world.worker.worker).await.unwrap();
    assert!(again.lines.is_empty(), "{:?}", again.lines);
}
