use poem_openapi::{ApiResponse, Object, OpenApi, payload::Json};

pub struct Router;

#[derive(Object)]
pub struct Group {
    // TODO: localize!
    pub description: String,
    pub name: String,
}

#[derive(ApiResponse)]
pub enum ListGroupsResponse {
    #[oai(status = 200)]
    Ok(Json<Vec<Group>>),
}

#[OpenApi]
impl Router {
    #[oai(path = "/groups", method = "get")]
    async fn list_groups(&self) -> ListGroupsResponse {
        ListGroupsResponse::Ok(Json(vec![]))
    }
}
