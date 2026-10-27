use sanchoris_core::schema::{create_tables, priced_plans, seed_plans};
use sanchoris_core::store::Tables;

#[tokio::main]
async fn main() {
    let tables = match Tables::from_env() {
        Ok(tables) => tables,
        Err(error) => {
            eprintln!("sanchoris-dynamo-create: {error}");
            std::process::exit(2);
        }
    };
    let plans = match priced_plans(|name| std::env::var(name).ok()) {
        Ok(plans) => plans,
        Err(error) => {
            eprintln!("sanchoris-dynamo-create: {error}");
            std::process::exit(2);
        }
    };
    let config = aws_config::load_from_env().await;
    let client = aws_sdk_dynamodb::Client::new(&config);
    match create_tables(&client, &tables).await {
        Ok(created) if created.is_empty() => println!("tables already exist"),
        Ok(created) => println!("created {}", created.join(", ")),
        Err(error) => {
            eprintln!("sanchoris-dynamo-create: {error}");
            std::process::exit(1);
        }
    }
    if let Err(error) = seed_plans(&client, &tables, &plans).await {
        eprintln!("sanchoris-dynamo-create: {error}");
        std::process::exit(1);
    }
    println!("seeded {} plans", plans.len());
}
