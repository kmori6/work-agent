use crate::infrastructure::persistence::postgres_chat_message_repository::PostgresChatMessageRepository;
use crate::infrastructure::persistence::postgres_chat_session_repository::PostgresChatSessionRepository;
use crate::presentation::handler::create_session_handler::create_session_handler;
use crate::presentation::handler::delete_session_handler::delete_session_handler;
use crate::presentation::handler::get_session_handler::get_session_handler;
use crate::presentation::handler::health_handler::health_handler;
use crate::presentation::handler::list_session_handler::list_session_handler;
use crate::presentation::state::app_state::AppState;

use axum::{Router, routing::get};
use sqlx::PgPool;
use std::{env, net::SocketAddr};

pub async fn run(addr: SocketAddr) -> Result<(), std::io::Error> {
    let database_url = env::var("DATABASE_URL")
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::NotFound, err))?;

    let pool = PgPool::connect(&database_url)
        .await
        .map_err(std::io::Error::other)?;

    let chat_session_repository = PostgresChatSessionRepository::new(pool.clone());
    let chat_message_repository = PostgresChatMessageRepository::new(pool);

    let app_state = AppState {
        chat_session_repository,
        chat_message_repository,
    };

    let api_routes = Router::new()
        .route("/health", get(health_handler))
        .route(
            "/sessions",
            get(list_session_handler).post(create_session_handler),
        )
        .route(
            "/sessions/{id}",
            get(get_session_handler).delete(delete_session_handler),
        )
        .with_state(app_state);

    let app = Router::new().nest("/v1", api_routes);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await
}
