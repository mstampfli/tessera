//! Typed repositories. Each repo owns the SQL for one aggregate and returns
//! domain rows; handlers and workers never write ad-hoc SQL at call sites.

pub mod api_tokens;
pub mod audit;
pub mod chunks;
pub mod documents;
pub mod embeddings;
pub mod entities;
pub mod sessions;
pub mod sources;
pub mod users;
