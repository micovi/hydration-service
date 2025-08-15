use crate::models::{ProcessConfig, ProcessState, ProcessStatus};
use chrono::Utc;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

const MAX_ACTIVE_PROCESSES: usize = 5;

pub struct QueueManager {
    pub active: Arc<RwLock<HashMap<String, ProcessStatus>>>,
    pub queued: Arc<RwLock<VecDeque<ProcessConfig>>>,
    pub synced: Arc<RwLock<HashMap<String, ProcessStatus>>>,
    pub all_processes: Arc<RwLock<HashMap<String, ProcessStatus>>>,
}

impl QueueManager {
    pub fn new() -> Self {
        Self {
            active: Arc::new(RwLock::new(HashMap::new())),
            queued: Arc::new(RwLock::new(VecDeque::new())),
            synced: Arc::new(RwLock::new(HashMap::new())),
            all_processes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_to_queue(&self, config: ProcessConfig) -> Result<(), String> {
        let process_id = config.process_id.clone();
        
        // Check if already exists
        let all = self.all_processes.read().await;
        if all.contains_key(&process_id) {
            return Err(format!("Process {} already exists", process_id));
        }
        drop(all);
        
        // Create new status
        let mut status = ProcessStatus::new(config.name.clone(), process_id.clone());
        status.state = ProcessState::Queued;
        
        // Add to queue
        let mut queue = self.queued.write().await;
        queue.push_back(config);
        status.queue_position = Some(queue.len() - 1);
        drop(queue);
        
        // Add to all processes
        let mut all = self.all_processes.write().await;
        all.insert(process_id, status);
        
        Ok(())
    }

    pub async fn activate_next(&self) -> Option<ProcessConfig> {
        let active_count = self.active.read().await.len();
        if active_count >= MAX_ACTIVE_PROCESSES {
            return None;
        }
        
        let mut queue = self.queued.write().await;
        if let Some(config) = queue.pop_front() {
            let process_id = config.process_id.clone();
            drop(queue);
            
            // Update status
            let mut all = self.all_processes.write().await;
            if let Some(status) = all.get_mut(&process_id) {
                status.state = ProcessState::Active;
                status.activated_at = Some(Utc::now());
                status.queue_position = None;
                
                // Add to active
                let mut active = self.active.write().await;
                active.insert(process_id, status.clone());
            }
            
            // Update queue positions
            let queue = self.queued.read().await;
            for (idx, queued_config) in queue.iter().enumerate() {
                if let Some(status) = all.get_mut(&queued_config.process_id) {
                    status.queue_position = Some(idx);
                }
            }
            
            Some(config)
        } else {
            None
        }
    }

    pub async fn mark_synced(&self, process_id: &str) -> Result<(), String> {
        // Remove from active
        let mut active = self.active.write().await;
        if let Some(mut status) = active.remove(process_id) {
            status.state = ProcessState::Synced;
            status.synced_at = Some(Utc::now());
            
            // Add to synced
            let mut synced = self.synced.write().await;
            synced.insert(process_id.to_string(), status.clone());
            
            // Update in all processes
            let mut all = self.all_processes.write().await;
            all.insert(process_id.to_string(), status);
            
            Ok(())
        } else {
            Err(format!("Process {} not in active list", process_id))
        }
    }

    pub async fn mark_error(&self, process_id: &str, error: String) -> Result<(), String> {
        // Remove from active
        let mut active = self.active.write().await;
        if let Some(mut status) = active.remove(process_id) {
            status.state = ProcessState::Error;
            status.error = Some(error);
            
            // Update in all processes
            let mut all = self.all_processes.write().await;
            all.insert(process_id.to_string(), status);
            
            Ok(())
        } else {
            Err(format!("Process {} not in active list", process_id))
        }
    }

    pub async fn restart_process(&self, process_id: &str) -> Result<(), String> {
        let mut all = self.all_processes.write().await;
        
        if let Some(status) = all.get_mut(process_id) {
            // Reset status
            status.state = ProcessState::Queued;
            status.error = None;
            status.cron_initialized = false;
            status.activated_at = None;
            status.synced_at = None;
            status.metrics = Default::default();
            
            // Create config from status
            let config = ProcessConfig {
                name: status.name.clone(),
                process_id: process_id.to_string(),
                base_url: None,
            };
            
            // Add back to queue
            let mut queue = self.queued.write().await;
            queue.push_back(config);
            status.queue_position = Some(queue.len() - 1);
            
            Ok(())
        } else {
            Err(format!("Process {} not found", process_id))
        }
    }

    pub async fn get_status(&self) -> (usize, usize, usize) {
        let active = self.active.read().await.len();
        let queued = self.queued.read().await.len();
        let synced = self.synced.read().await.len();
        (active, queued, synced)
    }

    pub async fn get_active_processes(&self) -> Vec<ProcessStatus> {
        self.active.read().await.values().cloned().collect()
    }

    pub async fn get_queue_preview(&self, limit: usize) -> Vec<ProcessStatus> {
        let queue = self.queued.read().await;
        let all = self.all_processes.read().await;
        
        queue.iter()
            .take(limit)
            .filter_map(|config| all.get(&config.process_id))
            .cloned()
            .collect()
    }

    pub async fn get_recent_synced(&self, limit: usize) -> Vec<ProcessStatus> {
        let mut synced: Vec<_> = self.synced.read().await.values().cloned().collect();
        synced.sort_by_key(|s| s.synced_at);
        synced.reverse();
        synced.into_iter().take(limit).collect()
    }

    pub async fn update_process_status(&self, process_id: &str, update_fn: impl FnOnce(&mut ProcessStatus)) -> Result<(), String> {
        let mut all = self.all_processes.write().await;
        if let Some(status) = all.get_mut(process_id) {
            update_fn(status);
            
            // Also update in active if present
            let mut active = self.active.write().await;
            if let Some(active_status) = active.get_mut(process_id) {
                *active_status = status.clone();
            }
            drop(active);
            
            // Also update in synced if present
            let mut synced = self.synced.write().await;
            if let Some(synced_status) = synced.get_mut(process_id) {
                *synced_status = status.clone();
            }
            
            Ok(())
        } else {
            Err(format!("Process {} not found", process_id))
        }
    }

    pub async fn update_process_base_url(&self, process_id: &str, base_url: Option<String>) {
        let mut all = self.all_processes.write().await;
        if let Some(_status) = all.get_mut(process_id) {
            // Store base_url in process status if we add that field
            // For now, just log it
            info!("Updated base_url for process {}: {:?}", process_id, base_url);
        }
    }
    
    pub async fn update_process_config(&self, process_id: &str, name: String, base_url: Option<String>) {
        // Update in all_processes
        let mut all = self.all_processes.write().await;
        if let Some(status) = all.get_mut(process_id) {
            status.name = name.clone();
            // Store base_url in process status if we add that field
            info!("Updated name and base_url for process {}: name={}, base_url={:?}", process_id, name, base_url);
        }
        drop(all);
        
        // Also update in active if present
        let mut active = self.active.write().await;
        if let Some(status) = active.get_mut(process_id) {
            status.name = name.clone();
        }
        drop(active);
        
        // Also update in synced if present
        let mut synced = self.synced.write().await;
        if let Some(status) = synced.get_mut(process_id) {
            status.name = name.clone();
        }
        drop(synced);
        
        // Also update in queued if present
        let mut queued = self.queued.write().await;
        for config in queued.iter_mut() {
            if config.process_id == process_id {
                config.name = name.clone();
                config.base_url = base_url;
                break;
            }
        }
    }
}