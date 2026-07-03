pub(crate) mod account;
pub(crate) mod folder;
pub(crate) mod jobs;
pub(crate) mod message;
pub(crate) mod message_attachment;
pub(crate) mod message_location;
pub(crate) mod session;
pub(crate) mod stats;
pub(crate) mod user;
pub(crate) mod user_settings;

/// Case-insensitive equality filter on a `name` column.
///
/// Produces `LOWER(col) = LOWER(name)`, matching the pattern used by
/// `find_by_name` / `find_by_username` adapters.
pub(crate) fn lower_name_eq<C>(col: C, name: &str) -> sea_orm::sea_query::SimpleExpr
where
    C: sea_orm::sea_query::IntoColumnRef,
{
    use sea_orm::{
        ExprTrait,
        sea_query::{BinOper, Expr, Func},
    };
    Expr::expr(Func::lower(Expr::col(col))).binary(BinOper::Equal, Expr::value(name.to_lowercase()))
}
