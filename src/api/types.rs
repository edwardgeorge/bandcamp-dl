#![allow(dead_code)]
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum::{Display, EnumString};

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PageContext {
    #[serde(rename = "fanId")]
    pub fan_id: Option<u64>,
    #[serde(rename = "userId")]
    pub user_id: Option<u64>,
    #[serde(rename = "isLoggedIn")]
    pub is_logged_in: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CollectionSummary {
    pub fan_id: u64,
    pub username: String,
    pub url: String,
    pub tralbum_lookup: Option<HashMap<String, LookupItem>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CollectionSummaryResult {
    pub fan_id: u64,
    pub collection_summary: CollectionSummary,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CollectionItemsRequest<'a> {
    pub fan_id: u64,
    pub count: usize,
    pub older_than_token: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LookupItem {
    item_type: String,
    item_id: u64,
    band_id: u64,
    purchased: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProfileData {
    pub collection_count: usize,
    pub item_cache: ItemCache,
    pub collection_data: CollectionData,
    #[serde(flatten)]
    other: Value,
}

impl ProfileData {
    pub fn iter_collection(&self) -> impl Iterator<Item = (Item, String)> + '_ {
        self.collection_data.sequence.iter().map(|id| {
            let it = self.item_cache.collection.get(id).unwrap();
            let u = self
                .collection_data
                .redownload_urls
                .get(&it.download_key().unwrap())
                .unwrap();
            (it.clone(), u.to_string())
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DownloadData {
    pub download_items: Vec<DownloadItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DownloadItem {
    pub downloads: HashMap<String, Download>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Download {
    pub description: String,
    pub encoding_name: String,
    pub size_mb: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CollectionData {
    pub batch_size: usize,
    pub hidden_items_count: usize,
    pub last_token: String,
    pub item_count: usize,
    pub redownload_urls: HashMap<String, String>,
    pub sequence: Vec<String>,
    #[serde(flatten)]
    other: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ItemCache {
    collection: HashMap<String, Item>,
    hidden: HashMap<String, Item>,
    wishlist: HashMap<String, Item>,
    #[serde(flatten)]
    other: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CollectionItemsResult {
    pub more_available: bool,
    pub items: Vec<Item>,
    pub redownload_urls: HashMap<String, String>,
    pub last_token: String,
}

impl CollectionItemsResult {
    pub fn iter_collection(&self) -> impl Iterator<Item = (Item, String)> + '_ {
        self.items.iter().map(|it| {
            let u = self
                .redownload_urls
                .get(&it.download_key().unwrap())
                .unwrap();
            (it.clone(), u.to_owned())
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Item {
    album_id: Option<u64>,
    also_collected_count: usize,
    band_id: u64,
    band_name: String,
    download_available: Option<bool>,
    hidden: Option<Value>,
    is_private: bool,
    is_preorder: bool,
    is_purchasable: bool,
    is_subscriber_only: bool,
    is_subscription_item: bool,
    pub item_id: u64,
    item_title: String,
    item_type: String,
    item_url: String,
    purchased: Option<String>,
    tralbum_id: u64,
    tralbum_type: String,
    sale_item_id: Option<u64>,
    sale_item_type: Option<String>,
    // #[serde(flatten)]
    // other: Value,
}

impl Item {
    pub fn display(&self) -> String {
        format!("{} - {}", self.item_title, self.band_name)
    }
    pub fn download_key(&self) -> Option<String> {
        Some(format!(
            "{}{}",
            self.sale_item_type.as_ref()?,
            self.sale_item_id.as_ref()?
        ))
    }
    pub fn to_path(&self) -> PathBuf {
        let level1 = sanitize_filename::sanitize(&self.band_name);
        let level2 = format!(
            "{level1} - {}",
            sanitize_filename::sanitize(&self.item_title),
        );
        Path::new(&level1).join(&level2)
    }
    pub fn download_available(&self) -> bool {
        self.download_available.unwrap_or(false)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MaybeError {
    #[serde(default)]
    pub error: bool,
    pub error_message: Option<String>,
    #[serde(flatten)]
    pub resp: Value,
}

#[derive(Debug, Clone, Copy, EnumString, Display, Default, ValueEnum)]
#[strum(serialize_all = "kebab-case")]
pub enum Format {
    AacHi,
    AiffLossless,
    Alac,
    Flac,
    #[default]
    Mp3_320,
    Mp3V0,
    Vorbis,
    Wav,
}
