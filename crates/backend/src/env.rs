#[derive(Clone)]
pub struct BackendEnv {
    pub database_url: String,
    pub cognito_user_pool_id: String,
    pub cognito_client_id: String,
}

pub fn load() -> BackendEnv {
    BackendEnv {
        database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
        cognito_user_pool_id: std::env::var("COGNITO_USER_POOL_ID")
            .expect("COGNITO_USER_POOL_ID must be set"),
        cognito_client_id: std::env::var("COGNITO_CLIENT_ID")
            .expect("COGNITO_CLIENT_ID must be set"),
    }
}

pub fn cognito_region(user_pool_id: &str) -> String {
    user_pool_id
        .split('_')
        .next()
        .filter(|region| !region.is_empty())
        .unwrap_or("ap-northeast-1")
        .to_string()
}
