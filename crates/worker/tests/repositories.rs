mod common;

use std::collections::BTreeMap;

use common::{INSTALLATION, SHOP_API_ID, SHOP_WEB_ID, World, labeled_payload};
use sanchoris_core::item::Entity;
use sanchoris_core::items::{
    AuditEvent, CONFIG_ACTIVE, CONFIG_DISABLED, Config, GitHubRepository, Guard, Integration,
    Project, Repository, SessionDef, SkillDef, Trigger, github_repositories,
};
use sanchoris_core::jobs::{Envelope, Job};
use sanchoris_core::keys;
use sanchoris_core::model::TaskStatus;
use sanchoris_core::testing::SEED_CONFIG_REPOSITORY_ID;
use sanchoris_worker::backfill;
use sanchoris_worker::config_sync::{disable_config, rename_repository};
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
    assert_eq!(tasks[0].name, "acme/storefront#70");
    assert_eq!(tasks[0].trigger_id, trigger_of(world.seeded.config_id));
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
            ("acme/shop-web#71".to_string(), trigger_of(second)),
            (
                "acme/shop-web-dev#72".to_string(),
                trigger_of(world.seeded.config_id)
            ),
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
async fn the_backfill_resolves_stored_names_and_disables_the_ones_outside_the_installation() {
    let world = World::new("pro").await;
    let tenant_id = world.tenant();
    let integration = world.dynamo.all::<Integration>().await.remove(0);
    world
        .dynamo
        .modify::<Integration>(&integration.key(), |integration| {
            integration.details =
                json!({ "account_login": "acme", "repositories": ["acme/sanchoris-config"] });
        })
        .await;

    let moved = world
        .github
        .rename(SEED_CONFIG_REPOSITORY_ID, "acme/platform-config");
    assert_eq!(moved.full_name, "acme/platform-config");
    let mut seeded = config(&world, world.seeded.config_id).await;
    seeded.repository = "acme/sanchoris-config".into();
    let mut legacy_config = seeded.to_item();
    for name in ["status", "repositories", "repository_id", "sync_error"] {
        legacy_config.remove(name);
    }
    world.dynamo.put_raw(legacy_config).await;
    for repository in world.dynamo.all::<Repository>().await {
        let mut legacy = repository.to_item();
        legacy.remove("github_repository_id");
        legacy.remove("full_name");
        world.dynamo.put_raw(legacy).await;
    }
    let stale_id = Uuid::new_v4();
    let mut stale = seeded.clone();
    stale.id = stale_id;
    stale.repository = "acme/elsewhere".into();
    stale.active_revision_id = None;
    let mut legacy_stale = stale.to_item();
    for name in ["status", "repositories", "repository_id", "sync_error"] {
        legacy_stale.remove(name);
    }
    world.dynamo.put_raw(legacy_stale).await;
    let stale_trigger = Trigger {
        id: trigger_of(stale_id),
        tenant_id,
        config_id: stale_id,
        config_revision_id: world.seeded.config_revision_id,
        name: "labeled-issue".into(),
        state: "ENABLED".into(),
        event_pattern: Some(json!({ "source": ["github"] })),
        schedule_expression: None,
        input_transformer: None,
        start: json!({}),
        removed_at: None,
        next_run_at: None,
        schedule_message_id: None,
    };
    world.dynamo.put(&stale_trigger).await;
    let legacy_guard = sanchoris_core::item::Key::main(
        keys::tenant_pk(tenant_id),
        "CONFIGREPO#acme/elsewhere#/".to_string(),
    );
    world
        .dynamo
        .put(&Guard::new(legacy_guard.clone(), stale_id))
        .await;

    let report = backfill::repository_ids(&world.worker.worker)
        .await
        .unwrap();
    world.worker.settle().await.unwrap();
    assert!(
        report.lines.iter().any(|line| line.ends_with(&format!(
            "acme/sanchoris-config -> acme/platform-config ({SEED_CONFIG_REPOSITORY_ID}), active"
        ))),
        "{:?}",
        report.lines
    );

    let resolved = config(&world, world.seeded.config_id).await;
    assert_eq!(resolved.repository_id, Some(SEED_CONFIG_REPOSITORY_ID));
    assert_eq!(resolved.repository, "acme/platform-config");
    assert_eq!(resolved.status, CONFIG_ACTIVE);
    assert_eq!(
        resolved.repository_named("acme/shop-web").map(|r| r.id),
        Some(SHOP_WEB_ID)
    );
    assert!(
        world
            .store
            .get::<Guard>(&keys::config_repository_key(
                tenant_id,
                SEED_CONFIG_REPOSITORY_ID,
                "/"
            ))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        world
            .dynamo
            .all::<Repository>()
            .await
            .iter()
            .all(|r| r.github_repository_id == SHOP_WEB_ID || r.github_repository_id == SHOP_API_ID)
    );

    let disabled = config(&world, stale_id).await;
    assert_eq!(disabled.status, CONFIG_DISABLED);
    assert_eq!(disabled.repository_id, None);
    assert_eq!(
        disabled.sync_error.as_deref(),
        Some("acme/elsewhere is not a repository of the tenant's GitHub App installation")
    );
    assert!(world.store.get_item(&legacy_guard).await.unwrap().is_none());
    let retired = world
        .store
        .get::<Trigger>(&keys::trigger_key(tenant_id, stale_id, "labeled-issue"))
        .await
        .unwrap()
        .unwrap();
    assert!(retired.removed_at.is_some());

    let integration = world.dynamo.all::<Integration>().await.remove(0);
    assert_eq!(
        github_repositories(&integration.details).unwrap().len(),
        common::known_repositories().len()
    );
    assert_eq!(INSTALLATION, integration.github_installation_id.unwrap());

    let again = backfill::repository_ids(&world.worker.worker)
        .await
        .unwrap();
    assert!(
        again.lines.iter().all(|line| line.contains("installation")),
        "{:?}",
        again.lines
    );
}
