pub mod types;

use std::error::Error;

use lazy_static::lazy_static;
use reqwest::{Client, Response};
use scraper::{Html, Selector};
use serde::de::DeserializeOwned;
pub use types::*;

lazy_static! {
    //static ref PAGE_FOOTER: Selector = Selector::parse("div#centerWrapper page-footer").unwrap();
    static ref PAGE_DATA: Selector = Selector::parse("div#pagedata").unwrap();
}

pub async fn resp_deser<T>(resp: Response) -> Result<T, Box<dyn Error>>
where
    T: DeserializeOwned,
{
    let t = resp.text().await?;
    let v: MaybeError = serde_json::from_str(&t)?;
    if v.error {
        return Err(v
            .error_message
            .unwrap_or_else(|| "unknown error from api".to_owned())
            .into());
    }
    Ok(serde_json::from_value(v.resp).map_err(|e| format!("Error deserialising response: {e}"))?)
}

pub async fn collection_summary(
    client: &Client,
) -> Result<CollectionSummaryResult, Box<dyn Error>> {
    let r = client
        .get("https://bandcamp.com/api/fan/2/collection_summary")
        .send()
        .await?
        .error_for_status()?;
    resp_deser(r).await
}

pub async fn user_profile(client: &Client, url: &str) -> Result<ProfileData, Box<dyn Error>> {
    let r = client.get(url).send().await?.error_for_status()?;
    let doc = Html::parse_document(&r.text().await?);
    for el in doc.select(&PAGE_DATA) {
        let a = el.attr("data-blob").unwrap();
        let v: ProfileData = serde_json::from_str(&a)?;
        return Ok(v);
    }
    Err("No data-blob found in user profile".into())
}

pub async fn collection_items(
    client: &Client,
    req: &CollectionItemsRequest<'_>,
) -> Result<CollectionItemsResult, Box<dyn Error>> {
    let r = client
        .post("https://bandcamp.com/api/fancollection/1/collection_items")
        .json(req)
        .header("Content-type", "application/json")
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await?
        .error_for_status()?;
    resp_deser(r).await
}

pub async fn get_download_link(
    client: &Client,
    url: &str,
    format: &str,
) -> Result<String, Box<dyn Error>> {
    let r = client.get(url).send().await?.error_for_status()?;
    let doc = Html::parse_document(&r.text().await?);
    for el in doc.select(&PAGE_DATA) {
        let a = el
            .attr("data-blob")
            .ok_or("data-blob attribute not found")?;
        let v: DownloadData = serde_json::from_str(&a)?;
        let d = v
            .download_items
            .first()
            .ok_or("No download items present!")?;
        let u = d.downloads.get(format).ok_or_else(|| {
            format!(
                "format {format} not available in: {}",
                d.downloads.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        return Ok(u.url.to_owned());
    }
    Err(format!("No page-data for download at: {}", url).into())
}

pub async fn list_remaining_collection<F>(
    client: &Client,
    fan_id: u64,
    profile: &ProfileData,
    progress_cb: Option<F>,
) -> Result<Vec<(Item, String)>, Box<dyn Error>>
where
    F: Fn(usize),
{
    let mut result: Vec<_> = profile.iter_collection().collect();
    let mut remaining = profile.collection_count - profile.collection_data.batch_size;
    let mut last_token = profile.collection_data.last_token.clone();
    while remaining > 0 {
        let items = collection_items(
            &client,
            &CollectionItemsRequest {
                fan_id,
                count: std::cmp::min(remaining, 500),
                older_than_token: &last_token,
            },
        )
        .await
        .unwrap();
        result.extend(items.iter_collection());
        if let Some(f) = &progress_cb {
            f(result.len());
        }
        remaining -= items.items.len();
        if !items.more_available {
            break;
        }
        last_token = items.last_token;
    }
    Ok(result)
}
