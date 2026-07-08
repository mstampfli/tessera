//! Typed repositories. Each repo owns the SQL for one aggregate and returns
//! domain rows; handlers and workers never write ad-hoc SQL at call sites.

pub mod api_tokens;
pub mod audit;
pub mod sessions;
pub mod users;
