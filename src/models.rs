use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    Queued,
    Active,
    Synced,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessConfig {
    pub name: String,
    #[serde(rename = "processId")]
    pub process_id: String,
    #[serde(rename = "baseUrl")]
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessMetrics {
    pub initial_slot_deficit: Option<u64>,
    pub slots_advanced_last_check: u64,
    pub total_slots_advanced: u64,
    pub sync_start_time: Option<DateTime<Utc>>,
    pub sync_end_time: Option<DateTime<Utc>>,
    pub avg_sync_rate: f64,
    pub check_count: u64,
    pub api_response_times: Vec<f64>,
}

impl Default for ProcessMetrics {
    fn default() -> Self {
        Self {
            initial_slot_deficit: None,
            slots_advanced_last_check: 0,
            total_slots_advanced: 0,
            sync_start_time: None,
            sync_end_time: None,
            avg_sync_rate: 0.0,
            check_count: 0,
            api_response_times: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessStatus {
    pub name: String,
    pub process_id: String,
    pub state: ProcessState,
    pub cron_initialized: bool,
    pub computed_slot: Option<u64>,
    pub current_slot: Option<u64>,
    pub last_checked: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub metrics: ProcessMetrics,
    pub queue_position: Option<usize>,
    pub activated_at: Option<DateTime<Utc>>,
    pub synced_at: Option<DateTime<Utc>>,
    pub hb_reserves: Option<HashMap<String, String>>,
    pub ao_reserves: Option<HashMap<String, String>>,
    pub reserves_last_checked: Option<DateTime<Utc>>,
    pub cron_created_at: Option<DateTime<Utc>>,
}

impl ProcessStatus {
    pub fn new(name: String, process_id: String) -> Self {
        Self {
            name,
            process_id,
            state: ProcessState::Queued,
            cron_initialized: false,
            computed_slot: None,
            current_slot: None,
            last_checked: None,
            error: None,
            metrics: ProcessMetrics::default(),
            queue_position: None,
            activated_at: None,
            synced_at: None,
            hb_reserves: None,
            ao_reserves: None,
            reserves_last_checked: None,
            cron_created_at: None,
        }
    }

    pub fn deficit(&self) -> Option<u64> {
        match (self.current_slot, self.computed_slot) {
            (Some(current), Some(computed)) if current > computed => Some(current - computed),
            _ => None,
        }
    }

    pub fn is_synced(&self) -> bool {
        match (self.current_slot, self.computed_slot) {
            (Some(current), Some(computed)) => current == computed,
            _ => false,
        }
    }
    
    pub fn reserves_match(&self) -> Option<bool> {
        match (&self.hb_reserves, &self.ao_reserves) {
            (Some(hb), Some(ao)) => {
                // Only compare actual token process IDs (43 chars), ignore TokenA/TokenB/K
                let hb_tokens: HashMap<&String, &String> = hb.iter()
                    .filter(|(key, _)| key.len() == 43 && !["TokenA", "TokenB", "K"].contains(&key.as_str()))
                    .collect();
                
                let ao_tokens: HashMap<&String, &String> = ao.iter()
                    .filter(|(key, _)| key.len() == 43)
                    .collect();
                
                // Check if we have the same tokens
                if hb_tokens.len() != ao_tokens.len() {
                    return Some(false);
                }
                
                // Compare reserves for each token
                for (token_id, hb_amount) in hb_tokens.iter() {
                    match ao_tokens.get(token_id) {
                        Some(ao_amount) => {
                            // Compare the raw reserve amounts
                            if hb_amount != ao_amount {
                                return Some(false);
                            }
                        },
                        None => {
                            // Token exists in HB but not in AO
                            return Some(false);
                        }
                    }
                }
                
                Some(true)
            },
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateFile {
    pub version: String,
    pub last_updated: DateTime<Utc>,
    pub queued_process_ids: Vec<String>,
    pub active_process_ids: Vec<String>,
    pub synced_process_ids: Vec<String>,
    pub processes: HashMap<String, ProcessStatusData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessStatusData {
    pub state: ProcessState,
    pub cron_initialized: bool,
    pub computed_slot: Option<u64>,
    pub current_slot: Option<u64>,
    pub last_checked: Option<DateTime<Utc>>,
    pub synced_at: Option<DateTime<Utc>>,
    pub activated_at: Option<DateTime<Utc>>,
    pub metrics: ProcessMetricsData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessMetricsData {
    pub initial_slot_deficit: Option<u64>,
    pub total_slots_advanced: u64,
    pub sync_start_time: Option<DateTime<Utc>>,
    pub sync_end_time: Option<DateTime<Utc>>,
    pub avg_sync_rate: f64,
    pub check_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(rename = "baseUrl")]
    pub base_url: Option<String>,
    pub processes: Vec<ProcessConfig>,
}

#[derive(Debug, Serialize)]
pub struct ApiStatus {
    pub active_count: usize,
    pub queued_count: usize,
    pub synced_count: usize,
    pub total_count: usize,
    pub runtime_seconds: u64,
    pub active_processes: Vec<ProcessStatus>,
    pub queue_preview: Vec<ProcessStatus>,
    pub recent_synced: Vec<ProcessStatus>,
}

#[derive(Debug, Deserialize)]
pub struct AddProcessRequest {
    pub name: String,
    pub process_id: String,
    pub base_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AODryRunRequest {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Target")]
    pub target: String,
    #[serde(rename = "Owner")]
    pub owner: String,
    #[serde(rename = "Anchor")]
    pub anchor: String,
    #[serde(rename = "Data")]
    pub data: String,
    #[serde(rename = "Tags")]
    pub tags: Vec<AOTag>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AOTag {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct AODryRunResponse {
    #[serde(rename = "Messages")]
    pub messages: Option<Vec<AOMessage>>,
    #[serde(rename = "GasUsed")]
    pub gas_used: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct AOMessage {
    #[serde(rename = "Tags")]
    pub tags: Vec<AOTag>,
}

impl AOTag {
    pub fn new(name: &str, value: &str) -> Self {
        Self {
            name: name.to_string(),
            value: value.to_string(),
        }
    }
}