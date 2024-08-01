use std::error::Error;

use reqwest::Url;
use reqwest_cookie_store::{CookieStore, CookieStoreMutex, RawCookie};
use rookie::firefox;
use time::OffsetDateTime;

pub fn get_cookies() -> Result<CookieStoreMutex, Box<dyn Error>> {
    let mut cs = CookieStore::new(None);
    for c in firefox(Some(vec![
        "bandcamp.com".to_string(),
        ".bandcamp.com".to_string(),
    ]))? {
        cs.insert_raw(
            &RawCookie::build((&c.name, &c.value))
                .domain(&c.domain)
                .secure(c.secure)
                .http_only(c.http_only)
                .expires(
                    c.expires
                        .map(|i| OffsetDateTime::from_unix_timestamp(i as i64).unwrap()),
                )
                .build(),
            &Url::parse(&format!(
                "https://{}{}",
                c.domain.trim_start_matches('.'),
                &c.path
            ))?,
        )
        .map_err(|e| format!("Got error on {c:?}: {e}"))?;
    }
    Ok(CookieStoreMutex::new(cs))
}
