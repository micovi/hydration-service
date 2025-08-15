use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use crate::models::{AODryRunRequest, AODryRunResponse, AOTag};

const DEFAULT_BASE_URL: &str = "http://65.108.7.125:8734";
const AO_CU_URL: &str = "https://cu.ao-testnet.xyz";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct HyperBeamClient {
    client: Client,
}

impl HyperBeamClient {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("Failed to create HTTP client");
        
        Self { client }
    }

    pub async fn initialize_cron(&self, base_url: Option<&str>, process_id: &str) -> Result<()> {
        let base = base_url.unwrap_or(DEFAULT_BASE_URL);
        let url = format!("{}/~cron@1.0/once?cron-path=/{process_id}~process@1.0/now", base);
        
        let response = self.client
            .get(&url)
            .send()
            .await?;
        
        if !response.status().is_success() {
            return Err(anyhow!(
                "Failed to initialize cron: HTTP {} - {}",
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }
        
        Ok(())
    }

    pub async fn get_slot_value(
        &self,
        base_url: Option<&str>,
        process_id: &str,
        endpoint: &str,
    ) -> Result<(u64, f64)> {
        let base = base_url.unwrap_or(DEFAULT_BASE_URL);
        let url = format!("{}/{process_id}~process@1.0/{endpoint}", base);
        
        let start = Instant::now();
        let response = self.client
            .get(&url)
            .send()
            .await?;
        
        let response_time = start.elapsed().as_millis() as f64;
        
        if !response.status().is_success() {
            return Err(anyhow!(
                "Failed to get slot value: HTTP {} - {}",
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }
        
        let text = response.text().await?;
        let value = text.trim().parse::<u64>()
            .map_err(|e| anyhow!("Failed to parse slot value '{}': {}", text, e))?;
        
        Ok((value, response_time))
    }

    pub async fn get_computed_slot(
        &self,
        base_url: Option<&str>,
        process_id: &str,
    ) -> Result<(u64, f64)> {
        self.get_slot_value(base_url, process_id, "compute/at-slot").await
    }

    pub async fn get_current_slot(
        &self,
        base_url: Option<&str>,
        process_id: &str,
    ) -> Result<(u64, f64)> {
        self.get_slot_value(base_url, process_id, "slot/current").await
    }

    pub async fn check_slots(
        &self,
        base_url: Option<&str>,
        process_id: &str,
    ) -> Result<SlotCheckResult> {
        let (computed_future, current_future) = tokio::join!(
            self.get_computed_slot(base_url, process_id),
            self.get_current_slot(base_url, process_id)
        );
        
        let (computed_slot, computed_time) = computed_future?;
        let (current_slot, current_time) = current_future?;
        
        Ok(SlotCheckResult {
            computed_slot,
            current_slot,
            computed_response_time: computed_time,
            current_response_time: current_time,
        })
    }
    
    pub async fn check_current_slot(&self, base_url: Option<&str>, process_id: &str) -> Result<u64> {
        let (current_slot, _) = self.get_current_slot(base_url, process_id).await?;
        Ok(current_slot)
    }
}

#[derive(Debug, Clone)]
pub struct SlotCheckResult {
    pub computed_slot: u64,
    pub current_slot: u64,
    pub computed_response_time: f64,
    pub current_response_time: f64,
}

impl SlotCheckResult {
    pub fn is_synced(&self) -> bool {
        self.computed_slot == self.current_slot
    }
    
    pub fn deficit(&self) -> u64 {
        if self.current_slot > self.computed_slot {
            self.current_slot - self.computed_slot
        } else {
            0
        }
    }
}

impl HyperBeamClient {
    pub async fn fetch_hb_reserves(
        &self,
        base_url: Option<&str>,
        process_id: &str,
    ) -> Result<HashMap<String, String>> {
        let base = base_url.unwrap_or(DEFAULT_BASE_URL);
        let url = format!("{}/{process_id}~process@1.0/now/reserves", base);
        
        let response = self.client
            .get(&url)
            .send()
            .await?;
        
        if !response.status().is_success() {
            return Err(anyhow!(
                "Failed to fetch HB reserves: HTTP {} - {}",
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }
        
        let reserves: HashMap<String, String> = response.json().await?;
        Ok(reserves)
    }
    
    pub async fn fetch_ao_reserves(&self, process_id: &str) -> Result<HashMap<String, String>> {
        let payload = AODryRunRequest {
            id: "1234".to_string(),
            target: process_id.to_string(),
            owner: "1234".to_string(),
            anchor: "0".to_string(),
            data: "1234".to_string(),
            tags: vec![
                AOTag::new("Action", "Get-Reserves"),
                AOTag::new("Data-Protocol", "ao"),
                AOTag::new("Type", "Message"),
                AOTag::new("Variant", "ao.TN.1"),
            ],
        };
        
        let url = format!("{}/dry-run?process-id={}", AO_CU_URL, process_id);
        let response = self.client
            .post(&url)
            .json(&payload)
            .send()
            .await?;
        
        if !response.status().is_success() {
            return Err(anyhow!(
                "Failed to fetch AO reserves: HTTP {} - {}",
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }
        
        let data: AODryRunResponse = response.json().await?;
        
        // Extract reserves from tags
        let mut reserves = HashMap::new();
        if let Some(messages) = data.messages {
            if let Some(message) = messages.first() {
                for tag in &message.tags {
                    // Skip non-token tags
                    if !["Action", "Data-Protocol", "Type", "Variant", "Reference"].contains(&tag.name.as_str()) {
                        // Token addresses are 43 characters long
                        if tag.name.len() == 43 {
                            reserves.insert(tag.name.clone(), tag.value.clone());
                        }
                    }
                }
            }
        }
        
        Ok(reserves)
    }
    
    pub async fn fetch_reserves(
        &self,
        base_url: Option<&str>,
        process_id: &str,
    ) -> Result<ReservesResult> {
        let (hb_future, ao_future) = tokio::join!(
            self.fetch_hb_reserves(base_url, process_id),
            self.fetch_ao_reserves(process_id)
        );
        
        let hb_reserves = hb_future.ok();
        let ao_reserves = ao_future.ok();
        
        Ok(ReservesResult {
            hb_reserves,
            ao_reserves,
        })
    }
    
    pub async fn fetch_cron_list(&self, base_url: Option<&str>) -> Result<Vec<CronItem>> {
        let base = base_url.unwrap_or(DEFAULT_BASE_URL);
        let url = format!("{}/~cron@1.0/list/serialize~json@1.0", base);
        
        let response = self.client
            .get(&url)
            .send()
            .await?;
        
        if !response.status().is_success() {
            return Err(anyhow!("Failed to fetch cron list: {}", response.status()));
        }
        
        let cron_response: CronListResponse = response.json().await?;
        
        if cron_response.status != 200 {
            return Err(anyhow!("Cron list API returned status: {}", cron_response.status));
        }
        
        Ok(cron_response.body)
    }
}

#[derive(Debug, Clone)]
pub struct ReservesResult {
    pub hb_reserves: Option<HashMap<String, String>>,
    pub ao_reserves: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronItem {
    pub created_at: u64,
    pub path: String,
    pub pid: String,
    pub task_id: String,
    #[serde(rename = "type")]
    pub cron_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronListResponse {
    pub body: Vec<CronItem>,
    pub device: String,
    pub status: u16,
}