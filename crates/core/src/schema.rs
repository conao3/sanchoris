use aws_sdk_dynamodb::types::{
    AttributeDefinition, BillingMode, GlobalSecondaryIndex, KeySchemaElement, KeyType, Projection,
    ProjectionType, ScalarAttributeType, StreamSpecification, StreamViewType,
    TimeToLiveSpecification,
};
use serde::Deserialize;
use serde_json::Value;

use crate::aws::MissingEnv;
use crate::item::Entity;
use crate::items::Plan;
use crate::store::{DbError, Tables};

pub const TABLES_JSON: &str = include_str!("../../../infra/tables.json");
pub const PLANS_JSON: &str = include_str!("../../../infra/plans.json");

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSpec {
    pub logical_id: String,
    pub role: String,
    pub table_name_env: String,
    #[serde(default)]
    pub append_only: bool,
    pub properties: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schema {
    pub tables: Vec<TableSpec>,
    pub append_only_denied_actions: Vec<String>,
    pub read_only_leading_keys: Vec<String>,
}

pub fn schema() -> Schema {
    serde_json::from_str(TABLES_JSON).expect("infra/tables.json is valid")
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlanSpec {
    id: String,
    name: String,
    price_cents: i64,
    currency: String,
    billing_unit: String,
    #[serde(deserialize_with = "Option::deserialize")]
    task_limit: Option<i64>,
    task_limit_period: String,
    #[serde(deserialize_with = "Option::deserialize")]
    workspace_seconds_limit: Option<i64>,
    workspace_limit_period: String,
    #[serde(deserialize_with = "Option::deserialize")]
    member_limit: Option<i64>,
    max_concurrency: i64,
    retention_days: i64,
    audit_retention_days: i64,
    max_task_turns: i64,
    max_task_cost_usd: f64,
    max_task_timeout_seconds: i64,
    max_idle_seconds: i64,
    max_children_depth: i64,
    max_children_count: i64,
    computes: Vec<String>,
    workspace_cpu: i64,
    workspace_memory_mib: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct PlansFile {
    plans: Vec<PlanSpec>,
}

pub fn plans() -> Vec<Plan> {
    let file: PlansFile = serde_json::from_str(PLANS_JSON).expect("infra/plans.json is valid");
    file.plans
        .into_iter()
        .map(|p| Plan {
            id: p.id,
            name: p.name,
            price_cents: p.price_cents,
            currency: p.currency,
            billing_unit: p.billing_unit,
            task_limit: p.task_limit,
            task_limit_period: p.task_limit_period,
            workspace_seconds_limit: p.workspace_seconds_limit,
            workspace_limit_period: p.workspace_limit_period,
            member_limit: p.member_limit,
            max_concurrency: p.max_concurrency,
            retention_days: p.retention_days,
            audit_retention_days: p.audit_retention_days,
            max_task_turns: p.max_task_turns,
            max_task_cost_usd: p.max_task_cost_usd,
            max_task_timeout_seconds: p.max_task_timeout_seconds,
            max_idle_seconds: p.max_idle_seconds,
            max_children_depth: p.max_children_depth,
            max_children_count: p.max_children_count,
            computes: p.computes,
            workspace_cpu: p.workspace_cpu,
            workspace_memory_mib: p.workspace_memory_mib,
            stripe_price_id: None,
        })
        .collect()
}

pub fn stripe_price_env(plan_id: &str) -> String {
    format!("SANCHORIS_STRIPE_PRICE_{}", plan_id.to_uppercase())
}

pub fn priced_plans(price_of: impl Fn(&str) -> Option<String>) -> Result<Vec<Plan>, MissingEnv> {
    let mut missing = Vec::new();
    let plans = plans()
        .into_iter()
        .map(|mut plan| {
            if plan.price_cents > 0 {
                let name = stripe_price_env(&plan.id);
                match price_of(&name).filter(|price| !price.is_empty()) {
                    Some(price) => plan.stripe_price_id = Some(price),
                    None => missing.push(name),
                }
            }
            plan
        })
        .collect();
    if missing.is_empty() {
        Ok(plans)
    } else {
        Err(MissingEnv(missing))
    }
}

fn text<'a>(value: &'a Value, name: &str) -> &'a str {
    value[name]
        .as_str()
        .unwrap_or_else(|| panic!("infra/tables.json: {name} is a string"))
}

fn key_schema(value: &Value) -> Vec<KeySchemaElement> {
    value
        .as_array()
        .expect("infra/tables.json: KeySchema is an array")
        .iter()
        .map(|element| {
            KeySchemaElement::builder()
                .attribute_name(text(element, "AttributeName"))
                .key_type(KeyType::from(text(element, "KeyType")))
                .build()
                .expect("key schema elements build")
        })
        .collect()
}

fn dynamo_error(error: impl std::error::Error) -> DbError {
    DbError::Dynamo(aws_sdk_dynamodb::error::DisplayErrorContext(error).to_string())
}

pub async fn create_tables(
    client: &aws_sdk_dynamodb::Client,
    tables: &Tables,
) -> Result<Vec<String>, DbError> {
    let mut created = Vec::new();
    for spec in schema().tables {
        let name = match spec.role.as_str() {
            "main" => &tables.main,
            "audit" => &tables.audit,
            other => panic!("infra/tables.json: unknown role {other}"),
        };
        let properties = &spec.properties;
        let attributes = properties["AttributeDefinitions"]
            .as_array()
            .expect("infra/tables.json: AttributeDefinitions is an array")
            .iter()
            .map(|a| {
                AttributeDefinition::builder()
                    .attribute_name(text(a, "AttributeName"))
                    .attribute_type(ScalarAttributeType::from(text(a, "AttributeType")))
                    .build()
                    .expect("attribute definitions build")
            })
            .collect::<Vec<_>>();
        let indexes = properties["GlobalSecondaryIndexes"]
            .as_array()
            .map(|indexes| {
                indexes
                    .iter()
                    .map(|index| {
                        GlobalSecondaryIndex::builder()
                            .index_name(text(index, "IndexName"))
                            .set_key_schema(Some(key_schema(&index["KeySchema"])))
                            .projection(
                                Projection::builder()
                                    .projection_type(ProjectionType::from(text(
                                        &index["Projection"],
                                        "ProjectionType",
                                    )))
                                    .build(),
                            )
                            .build()
                            .expect("indexes build")
                    })
                    .collect::<Vec<_>>()
            });
        let mut request = client
            .create_table()
            .table_name(name)
            .billing_mode(BillingMode::from(text(properties, "BillingMode")))
            .set_attribute_definitions(Some(attributes))
            .set_key_schema(Some(key_schema(&properties["KeySchema"])))
            .set_global_secondary_indexes(indexes);
        if let Some(stream) = properties.get("StreamSpecification") {
            request = request.stream_specification(
                StreamSpecification::builder()
                    .stream_enabled(true)
                    .stream_view_type(StreamViewType::from(text(stream, "StreamViewType")))
                    .build()
                    .expect("stream specification builds"),
            );
        }
        match request.send().await {
            Ok(_) => created.push(name.clone()),
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(|e| e.is_resource_in_use_exception()) =>
            {
                continue;
            }
            Err(error) => return Err(dynamo_error(error)),
        }
        if let Some(ttl) = properties.get("TimeToLiveSpecification") {
            client
                .update_time_to_live()
                .table_name(name)
                .time_to_live_specification(
                    TimeToLiveSpecification::builder()
                        .attribute_name(text(ttl, "AttributeName"))
                        .enabled(ttl["Enabled"].as_bool().unwrap_or(false))
                        .build()
                        .expect("ttl specification builds"),
                )
                .send()
                .await
                .map_err(dynamo_error)?;
        }
    }
    Ok(created)
}

pub async fn seed_plans(
    client: &aws_sdk_dynamodb::Client,
    tables: &Tables,
    plans: &[Plan],
) -> Result<(), DbError> {
    for plan in plans {
        let mut item = plan.to_item();
        item.insert(
            crate::item::VERSION.into(),
            aws_sdk_dynamodb::types::AttributeValue::N("1".into()),
        );
        client
            .put_item()
            .table_name(&tables.main)
            .set_item(Some(item))
            .send()
            .await
            .map_err(dynamo_error)?;
    }
    Ok(())
}

pub async fn drop_tables(
    client: &aws_sdk_dynamodb::Client,
    tables: &Tables,
) -> Result<(), DbError> {
    for name in [&tables.main, &tables.audit] {
        client
            .delete_table()
            .table_name(name)
            .send()
            .await
            .map_err(dynamo_error)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_declares_both_tables_and_append_only_audit() {
        let schema = schema();
        let roles = schema
            .tables
            .iter()
            .map(|t| (t.role.as_str(), t.append_only))
            .collect::<Vec<_>>();
        assert_eq!(roles, [("main", false), ("audit", true)]);
        assert!(
            schema
                .append_only_denied_actions
                .iter()
                .any(|a| a == "dynamodb:UpdateItem")
        );
        assert!(
            schema
                .append_only_denied_actions
                .iter()
                .any(|a| a == "dynamodb:DeleteItem")
        );
        assert_eq!(schema.read_only_leading_keys, ["PLANS"]);
        assert_eq!(plans().len(), 5);
    }

    #[test]
    fn paid_plans_take_their_stripe_prices_and_fail_without_one() {
        let priced = priced_plans(|name| Some(format!("price_for_{name}"))).unwrap();
        let prices = priced
            .iter()
            .map(|plan| (plan.id.as_str(), plan.stripe_price_id.as_deref()))
            .collect::<Vec<_>>();
        assert_eq!(
            prices,
            [
                ("free", None),
                ("starter", Some("price_for_SANCHORIS_STRIPE_PRICE_STARTER")),
                ("pro", Some("price_for_SANCHORIS_STRIPE_PRICE_PRO")),
                ("max", Some("price_for_SANCHORIS_STRIPE_PRICE_MAX")),
                ("admin", None),
            ]
        );
        let missing = priced_plans(|name| {
            (name != "SANCHORIS_STRIPE_PRICE_PRO").then(|| "price_x".to_string())
        })
        .unwrap_err();
        assert_eq!(missing.0, ["SANCHORIS_STRIPE_PRICE_PRO"]);
    }

    #[test]
    fn a_plan_spec_states_each_limit_or_null() {
        let mut spec: Value = serde_json::from_str(PLANS_JSON).unwrap();
        let free = &mut spec["plans"][0];
        assert!(serde_json::from_value::<PlanSpec>(free.clone()).is_ok());
        free.as_object_mut().unwrap().remove("memberLimit");
        let error = serde_json::from_value::<PlanSpec>(free.clone()).unwrap_err();
        assert_eq!(error.to_string(), "missing field `memberLimit`");
    }
}
