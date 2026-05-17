use poem::web::cookie::{Cookie, SameSite};
use uuid::Uuid;

pub const REFRESH_TOKEN_COOKIE: &str = "teknologappen-auth-refresh-token";

pub fn set_attrs(mut cookie: Cookie, days: u64) -> Cookie {
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::None);
    cookie.set_secure(true);
    cookie.set_max_age(std::time::Duration::from_hours(24 * days));
    cookie.set_path("/");
    cookie
}
pub fn get(refresh_token: Uuid) -> Cookie {
    set_attrs(
        Cookie::new_with_str(REFRESH_TOKEN_COOKIE, refresh_token),
        365,
    )
}
pub fn remove() -> Cookie {
    set_attrs(Cookie::new_with_str(REFRESH_TOKEN_COOKIE, ""), 0)
}
