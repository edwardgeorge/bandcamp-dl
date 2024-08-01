use std::{error::Error, fmt::Display, str::FromStr};

use reqwest::Url;
use reqwest_cookie_store::{CookieStore, CookieStoreMutex, RawCookie};
use rookie::{enums::Cookie, firefox};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    Firefox,
}

impl Browser {
    pub fn to_str(&self) -> &'static str {
        match self {
            Self::Firefox => "firefox",
        }
    }
    fn get_cookies(&self, domains: Option<Vec<String>>) -> Result<Vec<Cookie>, Box<dyn Error>> {
        Ok(match self {
            Self::Firefox => firefox(domains)?,
        })
    }
}

impl FromStr for Browser {
    type Err = Box<dyn Error>;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "firefox" => Ok(Browser::Firefox),
            _ => Err(format!("Unknown/Unsupported browser '{s}").into()),
        }
    }
}

impl Display for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_str())
    }
}

pub fn get_cookies(browser: Browser) -> Result<CookieStoreMutex, Box<dyn Error>> {
    let mut cs = CookieStore::new(None);
    for c in browser.get_cookies(Some(vec![
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
