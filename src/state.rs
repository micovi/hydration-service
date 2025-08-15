use crate::models::{ProcessMetricsData, ProcessStatusData, ProcessState, StateFile};
use crate::queue::QueueManager;
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

const STATE_FILE_PATH: &str = "hydration-state.json";

pub async fn save_state(queue: &QueueManager) -> Result<()> {
    let all_processes = queue.all_processes.read().await;
    let active_ids = queue.active.read().await;
    let synced_ids = queue.synced.read().await;
    let queued = queue.queued.read().await;
    
    let mut processes = HashMap::new();
    for (id, status) in all_processes.iter() {
        processes.insert(
            id.clone(),
            ProcessStatusData {
                state: status.state.clone(),
                cron_initialized: status.cron_initialized,
                computed_slot: status.computed_slot,
                current_slot: status.current_slot,
                last_checked: status.last_checked,
                synced_at: status.synced_at,
                activated_at: status.activated_at,
                metrics: ProcessMetricsData {
                    initial_slot_deficit: status.metrics.initial_slot_deficit,
                    total_slots_advanced: status.metrics.total_slots_advanced,
                    sync_start_time: status.metrics.sync_start_time,
                    sync_end_time: status.metrics.sync_end_time,
                    avg_sync_rate: status.metrics.avg_sync_rate,
                    check_count: status.metrics.check_count,
                },
            },
        );
    }
    
    let state = StateFile {
        version: "2.0".to_string(),
        last_updated: Utc::now(),
        active_process_ids: active_ids.keys().cloned().collect(),
        synced_process_ids: synced_ids.keys().cloned().collect(),
        queued_process_ids: queued.iter().map(|c| c.process_id.clone()).collect(),
        processes,
    };
    
    let json = serde_json::to_string_pretty(&state)?;
    fs::write(STATE_FILE_PATH, json).await?;
    
    Ok(())
}

pub async fn load_state(queue: &QueueManager) -> Result<bool> {
    let path = Path::new(STATE_FILE_PATH);
    if !path.exists() {
        return Ok(false);
    }
    
    let content = fs::read_to_string(path).await?;
    let state: StateFile = serde_json::from_str(&content)?;
    
    // Restore processes
    let mut all_processes = queue.all_processes.write().await;
    let mut active = queue.active.write().await;
    let mut synced = queue.synced.write().await;
    let mut queued = queue.queued.write().await;
    
    // First, restore all processes to all_processes map
    for (id, data) in &state.processes {
        let mut status = crate::models::ProcessStatus {
            name: id.clone(), // Will be updated when config is loaded
            process_id: id.clone(),
            state: data.state.clone(),
            cron_initialized: data.cron_initialized,
            computed_slot: data.computed_slot,
            current_slot: data.current_slot,
            last_checked: data.last_checked,
            error: None,
            metrics: crate::models::ProcessMetrics {
                initial_slot_deficit: data.metrics.initial_slot_deficit,
                slots_advanced_last_check: 0,
                total_slots_advanced: data.metrics.total_slots_advanced,
                sync_start_time: data.metrics.sync_start_time,
                sync_end_time: data.metrics.sync_end_time,
                avg_sync_rate: data.metrics.avg_sync_rate,
                check_count: data.metrics.check_count,
                api_response_times: Vec::new(),
            },
            queue_position: None,
            activated_at: data.activated_at,
            synced_at: data.synced_at,
            hb_reserves: None,
            ao_reserves: None,
            reserves_last_checked: None,
            cron_created_at: None,
        };
        
        match data.state {
            ProcessState::Active => {
                active.insert(id.clone(), status.clone());
            }
            ProcessState::Synced => {
                synced.insert(id.clone(), status.clone());
            }
            ProcessState::Queued => {
                // Will be handled below with proper queue ordering
            }
            _ => {}
        }
        
        all_processes.insert(id.clone(), status);
    }
    
    // Now restore the queue in the correct order
    // First try to use queued_process_ids if available
    if !state.queued_process_ids.is_empty() {
        for (idx, process_id) in state.queued_process_ids.iter().enumerate() {
            // Create a ProcessConfig for the queue
            if let Some(status) = all_processes.get_mut(process_id) {
                status.queue_position = Some(idx);
                status.state = ProcessState::Queued;
                
                let config = crate::models::ProcessConfig {
                    name: status.name.clone(),
                    process_id: process_id.clone(),
                    base_url: None, // Will be updated from config if provided
                };
                queued.push_back(config);
            }
        }
    } else {
        // Fallback: If queued_process_ids is empty but we have processes with Queued state,
        // restore them to the queue (this handles legacy state files)
        let mut queued_processes: Vec<_> = state.processes.iter()
            .filter(|(_, data)| data.state == ProcessState::Queued)
            .map(|(id, _)| id.clone())
            .collect();
        
        // Sort them alphabetically to have a consistent order
        queued_processes.sort();
        
        for (idx, process_id) in queued_processes.iter().enumerate() {
            if let Some(status) = all_processes.get_mut(process_id) {
                status.queue_position = Some(idx);
                
                let config = crate::models::ProcessConfig {
                    name: status.name.clone(),
                    process_id: process_id.clone(),
                    base_url: None, // Will be updated from config if provided
                };
                queued.push_back(config);
            }
        }
        
        if !queued_processes.is_empty() {
            tracing::info!("Restored {} queued processes from legacy state format", queued_processes.len());
        }
    }
    
    Ok(true)
}