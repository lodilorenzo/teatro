use axum::{Extension, Json};

use crate::{api::auth::AuthenticatedUser, domain::user::PublicUser};

pub async fn me(Extension(user): Extension<AuthenticatedUser>) -> Json<PublicUser> {
    Json(user.public_user().clone())
}
