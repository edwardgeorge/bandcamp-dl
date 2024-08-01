use std::{error::Error, sync::Arc};

use reqwest::Client;
use reqwest_cookie_store::CookieStoreMutex;

pub fn get_client(cs: Option<Arc<CookieStoreMutex>>) -> Result<Client, Box<dyn Error>> {
    let mut cb = Client::builder()
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:122.0) Gecko/20100101 Firefox/122.0",
        )
        .cookie_store(true)
        .gzip(true);
    cb = match cs {
        Some(v) => cb.cookie_provider(v),
        None => cb.cookie_store(true),
    };
    Ok(cb.build()?)
}
