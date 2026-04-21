use poem_openapi::{ApiResponse, Object, OpenApi, payload::Json};

#[derive(Copy, Clone, Debug)]
pub struct Router;

#[derive(Debug, Object)]
pub struct Group {
    // TODO: localize!
    pub description: String,
    pub name: String,
}

#[derive(Debug, ApiResponse)]
pub enum ListGroupsResponse {
    #[oai(status = 200)]
    Ok(Json<Vec<Group>>),
}

#[OpenApi]
impl Router {
    #[expect(clippy::unused_async, reason = "this is just a mock rn")]
    #[oai(path = "/groups", method = "get")]
    async fn list_groups(&self) -> ListGroupsResponse {
        ListGroupsResponse::Ok(Json(vec![]))
    }
}
