//! This contains all functionality specific to admins.
//!
//! Viewing data which normal users also view is handled by their respective functions instead.

use std::collections::HashMap;
use std::ops::Deref;

use fed_auth_verifier::{User, callbacks::AuthCallbackDataV1};
use minilith_errors::{MinilithEndpointError, MinilithErrorResultExt as _};
use poem_openapi::{Object, OpenApi, payload::Json};
use s3::post_policy::PostPolicyExpiration;
use sqlx::postgres::types::PgLTree;
use sqlx::types::Uuid;
use sqlx::types::time::OffsetDateTime;

use crate::context::ContextWrapper;
use crate::group::{self, Path};
use crate::{
    DbInternationalizedString as DIS, InternationalizedString, MinilithErrorOptionExt as _,
    MinilithErrorResultExt as _, MinilithResult,
};

#[derive(Clone, Debug)]
pub struct Router {
    pub context: ContextWrapper,
}
impl Deref for Router {
    type Target = ContextWrapper;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}
const DEMO: &str = include_str!("./demo.csv");
fn get_guild(stil_id: &str) -> Option<char> {
    for line in DEMO.lines().skip(1) {
        let Some(id) = line.split(',').nth(3) else {
            continue;
        };
        let id = id
            .strip_prefix('"')
            .unwrap_or(id)
            .strip_suffix('"')
            .unwrap_or(id);
        if id == stil_id {
            let guild = line
                .split(',')
                .nth(4)?
                .strip_prefix('"')
                .unwrap_or(id)
                .strip_suffix('"')
                .unwrap_or(id);
            return guild.chars().next().as_ref().map(char::to_ascii_lowercase);
        }
    }
    None
}

#[derive(Object)]
struct MyGroup {
    id: Uuid,
    path: Path,
    name: InternationalizedString,
    description: InternationalizedString,
    logo_url: String,
}

#[derive(Object)]
struct ObjectUploadAllowanceRequest {
    /// Must not contain the `.`. I.e. `jpg`, `JPEG`, `png` is ok.
    extension: String,
}
/// See `Post File using FormData in Node.js` at
/// <https://www.npmjs.com/package/@aws-sdk/s3-presigned-post>
#[derive(Object)]
struct ObjectUploadAllowanceResponse {
    url: String,
    fields: HashMap<String, String>,
    dynamic_fields: HashMap<String, String>,

    /// the key you must upload to
    key: String,
    /// max size in bytes the object can be.
    max_size_bytes: u32,
}

#[OpenApi(prefix_path = "/admin")]
impl Router {
    /// # Extension
    ///
    /// Only the following extensions are allowed (case doesn't matter):
    ///
    /// - jpg
    /// - jpeg
    /// - webp
    /// - png
    /// - avif
    ///
    /// Notably, no GIF support.
    ///
    /// # Errors
    ///
    /// - You must be admin for some group.
    /// - Extension has to be valid
    /// - internal errors
    #[oai(path = "/upload-image", method = "post")]
    async fn upload_image(
        &self,
        user: User,
        Json(mut body): Json<ObjectUploadAllowanceRequest>,
    ) -> MinilithResult<Json<ObjectUploadAllowanceResponse>> {
        // only admins can upload
        group::admin::check_has_any_adminship(&self.db, user.get_id()).await?;

        body.extension.make_ascii_lowercase();
        if !matches!(
            body.extension.as_str(),
            "jpg" | "jpeg" | "webp" | "png" | "avif"
        ) {
            return Err(MinilithEndpointError::bad_frontend_code(
                "invalid extension",
                "",
            ));
        }

        let uuid = Uuid::new_v4();
        let max_size_bytes = 1024u32 * 1024 * 4;
        let key = format!("{uuid}.{}", body.extension);
        let policy = s3::post_policy::PostPolicy::new(PostPolicyExpiration::ExpiresIn(60 * 5))
            .condition(
                s3::PostPolicyField::Key,
                s3::PostPolicyValue::Exact(key.as_str().into()),
            )
            .wrap_err_internal("s3: bad key condition")?
            .condition(
                s3::PostPolicyField::ContentLengthRange,
                s3::PostPolicyValue::Range(0, max_size_bytes),
            )
            .wrap_err_internal("s3: bad content length condition")?
            .sign(self.image_bucket().clone().into())
            .await
            .wrap_err_internal("s3: failed to sign")?;

        Ok(Json(ObjectUploadAllowanceResponse {
            url: policy.url,
            fields: policy.fields,
            dynamic_fields: policy.dynamic_fields,
            key,
            max_size_bytes,
        }))
    }
}
