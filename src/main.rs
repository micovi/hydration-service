mod models;
mod hyperbeam;
mod queue;
mod state;
mod config;

use anyhow::{anyhow, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use models::{AddProcessRequest, ApiResponse, ApiStatus, Config, ProcessConfig, ProcessState};
use queue::QueueManager;
use hyperbeam::{HyperBeamClient, CronItem};
use config::ServiceConfig;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tower_http::cors::CorsLayer;
use tracing::{debug, error, info, warn};

struct AppState {
    queue: Arc<QueueManager>,
    client: Arc<HyperBeamClient>,
    start_time: chrono::DateTime<Utc>,
    cron_list: Arc<RwLock<Vec<CronItem>>>,
    config: Arc<ServiceConfig>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration
    let service_config = ServiceConfig::load()?;
    let service_config = Arc::new(service_config);
    
    // Initialize tracing based on config
    let filter = format!("hydration_service={},tower_http=warn", service_config.logging.level);
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

    info!("Starting Hydration Service");
    info!("Using HyperBEAM URL: {}", service_config.hyperbeam.base_url);
    info!("Using AO CU URL: {}", service_config.ao.cu_url);

    // Initialize components
    let queue = Arc::new(QueueManager::new(service_config.limits.max_active_processes));
    let client = Arc::new(HyperBeamClient::new(
        service_config.hyperbeam.base_url.clone(),
        service_config.ao.cu_url.clone(),
    ));
    
    // Load previous state
    let state_loaded = state::load_state(&queue).await?;
    if state_loaded {
        info!("Loaded previous state from disk");
    }

    // Load config and reconcile with existing state
    if let Some(config_path) = std::env::args().nth(1) {
        info!("Loading config from: {}", config_path);
        let config_str = tokio::fs::read_to_string(config_path).await?;
        let config: Config = serde_json::from_str(&config_str)?;
        
        // Reconcile config with existing state
        let existing_processes = queue.all_processes.read().await;
        let existing_ids: std::collections::HashSet<String> = existing_processes.keys().cloned().collect();
        drop(existing_processes);
        
        let mut new_processes = 0;
        let mut existing_in_config = 0;
        
        for process_config in config.processes {
            if existing_ids.contains(&process_config.process_id) {
                // Process already exists in state, update name and base_url from config
                existing_in_config += 1;
                info!("Process {} already in state, updating name and keeping existing state", process_config.name);
                
                // Update name and base_url from config
                queue.update_process_config(&process_config.process_id, process_config.name.clone(), process_config.base_url).await;
            } else {
                // New process not in state, add to queue
                if let Err(e) = queue.add_to_queue(process_config.clone()).await {
                    warn!("Failed to add process {} to queue: {}", process_config.name, e);
                } else {
                    new_processes += 1;
                    info!("Added new process {} to queue", process_config.name);
                }
            }
        }
        
        info!("Config reconciliation: {} new processes added, {} already existed", 
              new_processes, existing_in_config);
    }

    let app_state = Arc::new(AppState {
        queue: queue.clone(),
        client: client.clone(),
        start_time: Utc::now(),
        cron_list: Arc::new(RwLock::new(Vec::new())),
        config: service_config.clone(),
    });

    // Recovery: Check active processes that are initialized but have no slot values
    let active_processes = queue.get_active_processes().await;
    for process in active_processes {
        if process.cron_initialized && process.computed_slot.is_none() {
            info!("Recovering active process {} - fetching initial slot values", process.process_id);
            let client_clone = client.clone();
            let queue_clone = queue.clone();
            let process_id = process.process_id.clone();
            
            tokio::spawn(async move {
                match client_clone.check_slots(None, &process_id).await {
                    Ok(result) => {
                        let _ = queue_clone.update_process_status(&process_id, |status| {
                            status.computed_slot = Some(result.computed_slot);
                            status.current_slot = Some(result.current_slot);
                            status.last_checked = Some(Utc::now());
                            status.metrics.check_count = 1;
                            
                            if result.computed_slot < result.current_slot {
                                status.metrics.initial_slot_deficit = Some(result.current_slot - result.computed_slot);
                                status.metrics.sync_start_time = Some(Utc::now());
                            }
                        }).await;
                        info!("Recovered process {} - Computed: {}, Current: {}", 
                             process_id, result.computed_slot, result.current_slot);
                    },
                    Err(e) => {
                        error!("Failed to recover process {}: {}", process_id, e);
                    }
                }
            });
        }
    }

    // Start monitoring task
    let monitor_state = app_state.clone();
    tokio::spawn(async move {
        monitor_loop(monitor_state).await;
    });
    
    // Start synced pools monitoring task
    let synced_monitor_state = app_state.clone();
    tokio::spawn(async move {
        monitor_synced_pools(synced_monitor_state).await;
    });
    
    // Start cron list monitoring task
    let cron_monitor_state = app_state.clone();
    tokio::spawn(async move {
        // Fetch cron list immediately on startup
        info!("Fetching initial cron list from HyperBEAM");
        if let Ok(cron_items) = cron_monitor_state.client.fetch_cron_list(None).await {
            let mut cron_list = cron_monitor_state.cron_list.write().await;
            *cron_list = cron_items;
            info!("Initial cron list loaded with {} items", cron_list.len());
        }
        
        // Then continue monitoring
        monitor_cron_list(cron_monitor_state).await;
    });
    
    // Start queue monitoring task for current slots
    let queue_monitor_state = app_state.clone();
    tokio::spawn(async move {
        monitor_queue_slots(queue_monitor_state).await;
    });

    // Build router
    let app = Router::new()
        .route("/", get(render_tui))
        .route("/api/status", get(get_status))
        .route("/api/state", get(get_state))
        .route("/api/queue/add", post(add_to_queue))
        .route("/api/process/:id/restart", post(restart_process))
        .layer(CorsLayer::permissive())
        .with_state(app_state);

    let bind_addr = format!("{}:{}", service_config.server.host, service_config.server.port);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await?;
    
    info!("Server running on http://{}", bind_addr);
    
    axum::serve(listener, app).await?;
    
    Ok(())
}

async fn monitor_queue_slots(state: Arc<AppState>) {
    // Initial delay to let things settle
    sleep(Duration::from_secs(10)).await;
    
    loop {
        // Get queued processes
        let queue_preview = state.queue.get_queue_preview(20).await;
        let queue_count = queue_preview.len();
        
        if queue_count > 0 {
            debug!("Checking current slots for {} queued processes", queue_count);
            
            for process in queue_preview {
                let client = state.client.clone();
                let queue = state.queue.clone();
                let pid = process.process_id.clone();
                
                // Don't spawn, do it sequentially to avoid overwhelming the API
                match client.check_current_slot(None, &pid).await {
                    Ok(current_slot) => {
                        debug!("Got current slot {} for queued process {}", current_slot, &pid[..8]);
                        let _ = queue.update_process_status(&pid, |status| {
                            status.current_slot = Some(current_slot);
                            status.last_checked = Some(Utc::now());
                        }).await;
                    },
                    Err(_) => {
                        // Process might not exist yet, which is normal for queued items
                    }
                }
                
                // Small delay between checks
                sleep(Duration::from_millis(100)).await;
            }
        }
        
        // Update every 30 seconds
        sleep(Duration::from_secs(30)).await;
    }
}

async fn monitor_cron_list(state: Arc<AppState>) {
    loop {
        // Fetch cron list from HyperBEAM
        match state.client.fetch_cron_list(None).await {
            Ok(cron_items) => {
                let count = cron_items.len();
                info!("Fetched {} cron items from HyperBEAM", count);
                
                // Update the shared cron list
                let mut cron_list = state.cron_list.write().await;
                *cron_list = cron_items.clone();
                drop(cron_list);
                
                // Check slots for each active cron process
                for cron_item in &cron_items {
                    // Extract process ID from path (format: /processId~process@1.0/now)
                    if let Some(process_id) = cron_item.path
                        .strip_prefix("/")
                        .and_then(|p| p.split("~").next()) {
                        
                        let created_at = chrono::DateTime::from_timestamp_millis(cron_item.created_at as i64);
                        
                        // Check if we're tracking this process and fetch its slots
                        let all_processes = state.queue.all_processes.read().await;
                        if all_processes.contains_key(process_id) {
                            drop(all_processes);
                            
                            // Fetch current slot values for this active process
                            let client = state.client.clone();
                            let queue = state.queue.clone();
                            let pid = process_id.to_string();
                            
                            tokio::spawn(async move {
                                match client.check_slots(None, &pid).await {
                                    Ok(result) => {
                                        // First update the status
                                        let _ = queue.update_process_status(&pid, |status| {
                                            let old_computed = status.computed_slot;
                                            status.computed_slot = Some(result.computed_slot);
                                            status.current_slot = Some(result.current_slot);
                                            status.last_checked = Some(Utc::now());
                                            status.cron_created_at = created_at;
                                            
                                            // Track advancement
                                            if let Some(prev) = old_computed {
                                                if result.computed_slot > prev {
                                                    status.metrics.total_slots_advanced += result.computed_slot - prev;
                                                }
                                            }
                                            
                                            // Calculate sync rate based on cron creation time
                                            if let Some(created) = created_at {
                                                let minutes_elapsed = (Utc::now() - created).num_seconds() as f64 / 60.0;
                                                if minutes_elapsed > 0.0 && status.metrics.total_slots_advanced > 0 {
                                                    status.metrics.avg_sync_rate = status.metrics.total_slots_advanced as f64 / minutes_elapsed;
                                                }
                                            }
                                        }).await;
                                        
                                        // Check if synced and use the proper queue method
                                        if result.is_synced() {
                                            if let Err(e) = queue.mark_synced(&pid).await {
                                                // Process might already be marked as synced
                                                debug!("Failed to mark {} as synced: {}", pid, e);
                                            } else {
                                                info!("Process {} is now synced via cron check!", pid);
                                            }
                                        }
                                    },
                                    Err(e) => {
                                        error!("Failed to check slots for active process {}: {}", pid, e);
                                    }
                                }
                            });
                        }
                    }
                }
            },
            Err(e) => {
                error!("Failed to fetch cron list: {}", e);
            }
        }
        
        // Update every 15 seconds for faster refresh
        sleep(Duration::from_secs(15)).await;
    }
}

async fn monitor_synced_pools(state: Arc<AppState>) {
    // Initial delay to let pools sync first
    sleep(Duration::from_secs(5)).await;
    
    loop {
        info!("Updating synced pools data...");
        
        // Get list of synced pools
        let synced = state.queue.synced.read().await.clone();
        let synced_count = synced.len();
        
        if synced_count > 0 {
            info!("Checking {} synced pools for updates", synced_count);
        }
        
        for (process_id, _) in synced {
            let client = state.client.clone();
            let queue = state.queue.clone();
            let pid = process_id.clone();
            
            tokio::spawn(async move {
                // Check both computed and current slots
                match client.check_slots(None, &process_id).await {
                    Ok(result) => {
                        let was_synced = queue.synced.read().await.contains_key(&process_id);
                        let still_synced = result.is_synced();
                        
                        // Update slot values
                        let update_result = queue.update_process_status(&process_id, |status| {
                            let old_computed = status.computed_slot;
                            let old_current = status.current_slot;
                            
                            status.computed_slot = Some(result.computed_slot);
                            status.current_slot = Some(result.current_slot);
                            status.last_checked = Some(Utc::now());
                            
                            // Log if values changed
                            if old_computed != Some(result.computed_slot) || old_current != Some(result.current_slot) {
                                info!("Pool {} slots updated: computed {} -> {}, current {} -> {}", 
                                    &pid[..8],
                                    old_computed.unwrap_or(0), result.computed_slot,
                                    old_current.unwrap_or(0), result.current_slot
                                );
                            }
                            
                            // If no longer synced, mark it but keep in synced list for monitoring
                            if was_synced && !still_synced {
                                warn!("Pool {} is no longer synced! Computed: {}, Current: {}", 
                                    &pid[..8], result.computed_slot, result.current_slot);
                            }
                        }).await;
                        
                        if let Err(e) = update_result {
                            error!("Failed to update slots for {}: {}", &pid[..8], e);
                        }
                    },
                    Err(e) => {
                        error!("Failed to check slots for {}: {}", &pid[..8], e);
                    }
                }
                
                // Fetch reserves
                match client.fetch_reserves(None, &process_id).await {
                    Ok(reserves) => {
                        let hb_count = reserves.hb_reserves.as_ref().map(|r| r.len()).unwrap_or(0);
                        let ao_count = reserves.ao_reserves.as_ref().map(|r| r.len()).unwrap_or(0);
                        
                        let update_result = queue.update_process_status(&process_id, |status| {
                            let old_hb_count = status.hb_reserves.as_ref().map(|r| r.len()).unwrap_or(0);
                            let old_ao_count = status.ao_reserves.as_ref().map(|r| r.len()).unwrap_or(0);
                            
                            status.hb_reserves = reserves.hb_reserves;
                            status.ao_reserves = reserves.ao_reserves;
                            status.reserves_last_checked = Some(Utc::now());
                            
                            // Log if reserve counts changed
                            if old_hb_count != hb_count || old_ao_count != ao_count {
                                info!("Pool {} reserves updated: HB {} -> {}, AO {} -> {}", 
                                    &pid[..8], old_hb_count, hb_count, old_ao_count, ao_count);
                            }
                        }).await;
                        
                        if let Err(e) = update_result {
                            error!("Failed to update reserves for {}: {}", &pid[..8], e);
                        }
                    },
                    Err(e) => {
                        error!("Failed to fetch reserves for {}: {}", &pid[..8], e);
                    }
                }
            });
        }
        
        // Check every 60 seconds for synced pools
        sleep(Duration::from_secs(60)).await;
    }
}

async fn monitor_loop(state: Arc<AppState>) {
    loop {
        // Check active processes
        let active = state.queue.get_active_processes().await;
        
        for process in active {
            // Skip if process hasn't been initialized yet
            if !process.cron_initialized {
                continue;
            }
            
            let client = state.client.clone();
            let queue = state.queue.clone();
            let process_id = process.process_id.clone();
            
            tokio::spawn(async move {
                if let Err(e) = check_process(&client, &queue, &process).await {
                    error!("Error checking process {}: {}", process_id, e);
                }
            });
        }
        
        // Try to activate next process
        while let Some(config) = state.queue.activate_next().await {
            info!("Activating process: {}", config.name);
            
            let client = state.client.clone();
            let queue = state.queue.clone();
            
            tokio::spawn(async move {
                if let Err(e) = initialize_process(&client, &queue, &config).await {
                    error!("Failed to initialize {}: {}", config.process_id, e);
                    let _ = queue.mark_error(&config.process_id, e.to_string()).await;
                }
            });
        }
        
        // Save state
        if let Err(e) = state::save_state(&state.queue).await {
            error!("Failed to save state: {}", e);
        }
        
        sleep(Duration::from_secs(15)).await;
    }
}

async fn check_process(
    client: &HyperBeamClient,
    queue: &QueueManager,
    process: &models::ProcessStatus,
) -> Result<()> {
    let result = client.check_slots(None, &process.process_id).await?;
    
    let previous_computed = process.computed_slot;
    
    queue.update_process_status(&process.process_id, |status| {
        // Update slots
        status.computed_slot = Some(result.computed_slot);
        status.current_slot = Some(result.current_slot);
        status.last_checked = Some(Utc::now());
        
        // Update metrics
        status.metrics.check_count += 1;
        status.metrics.api_response_times.push(result.computed_response_time);
        status.metrics.api_response_times.push(result.current_response_time);
        
        if status.metrics.api_response_times.len() > 20 {
            status.metrics.api_response_times = status.metrics.api_response_times[status.metrics.api_response_times.len() - 20..].to_vec();
        }
        
        // Track advancement
        if let Some(prev) = previous_computed {
            if result.computed_slot > prev {
                status.metrics.slots_advanced_last_check = result.computed_slot - prev;
                status.metrics.total_slots_advanced += status.metrics.slots_advanced_last_check;
            } else {
                status.metrics.slots_advanced_last_check = 0;
            }
        }
        
        // Track initial deficit
        if status.metrics.initial_slot_deficit.is_none() {
            status.metrics.initial_slot_deficit = Some(result.deficit());
            status.metrics.sync_start_time = Some(Utc::now());
        }
        
        // Calculate sync rate
        if let Some(start) = status.metrics.sync_start_time {
            let minutes = (Utc::now() - start).num_seconds() as f64 / 60.0;
            if minutes > 0.0 && status.metrics.total_slots_advanced > 0 {
                status.metrics.avg_sync_rate = status.metrics.total_slots_advanced as f64 / minutes;
            }
        }
    }).await.map_err(|e| anyhow!(e))?;
    
    // Check if synced
    if result.is_synced() {
        info!("Process {} is synced!", process.process_id);
        queue.mark_synced(&process.process_id).await.map_err(|e| anyhow!(e))?;
        
        // Immediately fetch reserves for newly synced pool
        info!("Fetching reserves for newly synced pool: {}", process.process_id);
        if let Ok(reserves) = client.fetch_reserves(None, &process.process_id).await {
            let _ = queue.update_process_status(&process.process_id, |status| {
                status.hb_reserves = reserves.hb_reserves;
                status.ao_reserves = reserves.ao_reserves;
                status.reserves_last_checked = Some(Utc::now());
            }).await;
            info!("Reserves fetched for {}", process.process_id);
        }
    }
    
    Ok(())
}

async fn initialize_process(
    client: &HyperBeamClient,
    queue: &QueueManager,
    config: &ProcessConfig,
) -> Result<()> {
    info!("Initializing cron for {}", config.name);
    
    client.initialize_cron(config.base_url.as_deref(), &config.process_id).await?;
    
    queue.update_process_status(&config.process_id, |status| {
        status.cron_initialized = true;
    }).await.map_err(|e| anyhow!(e))?;
    
    // Immediately check slots after initializing
    info!("Getting initial slot values for {}", config.name);
    let result = client.check_slots(config.base_url.as_deref(), &config.process_id).await?;
    
    queue.update_process_status(&config.process_id, |status| {
        status.computed_slot = Some(result.computed_slot);
        status.current_slot = Some(result.current_slot);
        status.last_checked = Some(Utc::now());
        status.metrics.check_count = 1;
        
        // Set initial deficit
        if result.computed_slot < result.current_slot {
            status.metrics.initial_slot_deficit = Some(result.current_slot - result.computed_slot);
            status.metrics.sync_start_time = Some(Utc::now());
        }
    }).await.map_err(|e| anyhow!(e))?;
    
    info!("Process {} initialized - Computed: {}, Current: {}", 
         config.name, result.computed_slot, result.current_slot);
    
    Ok(())
}

async fn render_tui(State(state): State<Arc<AppState>>) -> Html<String> {
    let (_, queued_count, synced_count) = state.queue.get_status().await;
    let runtime = (Utc::now() - state.start_time).num_seconds();
    let queue_preview = state.queue.get_queue_preview(10).await;
    let all_synced: Vec<_> = state.queue.synced.read().await.values().cloned().collect();
    let cron_list = state.cron_list.read().await.clone();
    
    // Get active processes based on cron list
    let mut active_from_crons: Vec<models::ProcessStatus> = Vec::new();
    let all_processes = state.queue.all_processes.read().await;
    
    for cron_item in &cron_list {
        // Extract process ID from path
        if let Some(process_id) = cron_item.path
            .strip_prefix("/")
            .and_then(|p| p.split("~").next()) {
            
            // Check if we're tracking this process
            if let Some(process) = all_processes.get(process_id) {
                let mut process_with_cron = process.clone();
                // Update with cron created time
                let created_at = chrono::DateTime::from_timestamp_millis(cron_item.created_at as i64);
                process_with_cron.cron_created_at = created_at;
                
                // Calculate real-time sync rate based on cron creation
                if let Some(created) = created_at {
                    let minutes_elapsed = (Utc::now() - created).num_seconds() as f64 / 60.0;
                    if minutes_elapsed > 0.0 && process_with_cron.metrics.total_slots_advanced > 0 {
                        process_with_cron.metrics.avg_sync_rate = process_with_cron.metrics.total_slots_advanced as f64 / minutes_elapsed;
                    }
                }
                
                active_from_crons.push(process_with_cron);
            }
        }
    }
    
    let active_count = active_from_crons.len();
    
    let html = format!(r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Hydration Service</title>
    <style>
        body {{
            background: #ffffff;
            color: #000000;
            font-family: 'Courier New', monospace;
            padding: 20px;
            margin: 0;
        }}
        .container {{
            max-width: 1200px;
            margin: 0 auto;
        }}
        .header {{
            border: 3px solid #000000;
            padding: 15px;
            text-align: center;
            margin-bottom: 20px;
            background: #f8f8f8;
        }}
        h1 {{
            margin: 0;
            font-size: 24px;
            font-weight: bold;
        }}
        .stats {{
            color: #333333;
            margin: 10px 0;
            font-weight: bold;
        }}
        .section {{
            border: 2px solid #000000;
            padding: 15px;
            margin-bottom: 20px;
            background: #ffffff;
        }}
        .section-title {{
            background: #000000;
            color: #ffffff;
            padding: 5px 10px;
            display: inline-block;
            margin: -25px 0 10px 0;
            font-weight: bold;
        }}
        table {{
            width: 100%;
            border-collapse: collapse;
        }}
        th {{
            text-align: left;
            padding: 8px;
            border-bottom: 2px solid #000000;
            background: #f0f0f0;
            font-weight: bold;
        }}
        td {{
            padding: 8px;
            border-bottom: 1px solid #cccccc;
            vertical-align: top;
        }}
        td div {{
            line-height: 1.4;
            white-space: nowrap;
            overflow: hidden;
            text-overflow: ellipsis;
        }}
        tr:hover {{
            background: #f8f8f8;
        }}
        .synced {{
            color: #000000;
            font-weight: bold;
        }}
        .error {{
            color: #666666;
            text-decoration: underline;
        }}
        .deficit {{
            color: #333333;
            font-style: italic;
        }}
        .queue-item {{
            margin: 8px 0;
            padding-left: 20px;
            font-family: monospace;
        }}
        .refresh {{
            color: #666666;
            font-size: 12px;
            text-align: center;
            margin-top: 20px;
            font-style: italic;
        }}
    </style>
    <meta http-equiv="refresh" content="5">
</head>
<body>
    <div class="container">
        <div class="header">
            <h1>HYDRATION SERVICE</h1>
            <div class="stats">
                Runtime: {}m {}s | Active: {}/{} | Queued: {} | Synced: {}
            </div>
        </div>
        
        <div class="section">
            <div class="section-title">[ ACTIVE PROCESSES ]</div>
            <table>
                <thead>
                    <tr>
                        <th>Process Id</th>
                        <th>Computed</th>
                        <th>Current</th>
                        <th>Deficit</th>
                        <th>Rate/min</th>
                    </tr>
                </thead>
                <tbody>
                    {}
                </tbody>
            </table>
        </div>
        
        <div class="section">
            <div class="section-title">[ QUEUE (Next 10) ]</div>
            <table>
                <thead>
                    <tr>
                        <th width="10%">#</th>
                        <th width="60%">Process ID</th>
                        <th width="30%">Current Slot</th>
                    </tr>
                </thead>
                <tbody>
                    {}
                </tbody>
            </table>
        </div>
        
        <div class="section">
            <div class="section-title">[ SYNCED POOLS ({}) ]</div>
            <table>
                <thead>
                    <tr>
                        <th width="20%">Process ID</th>
                        <th width="8%">Computed</th>
                        <th width="8%">Current</th>
                        <th width="27%">HB Reserves</th>
                        <th width="27%">AO Reserves</th>
                        <th width="10%">Match</th>
                    </tr>
                </thead>
                <tbody>
                    {}
                </tbody>
            </table>
        </div>
        
        <div class="section">
            <div class="section-title">[ ACTIVE CRONS ({}) ]</div>
            <table>
                <thead>
                    <tr>
                        <th width="20%">Task ID</th>
                        <th width="15%">Type</th>
                        <th width="45%">Path</th>
                        <th width="20%">Created</th>
                    </tr>
                </thead>
                <tbody>
                    {}
                </tbody>
            </table>
        </div>
        
        <div class="refresh">Page refreshes every 5 seconds</div>
    </div>
</body>
</html>
    "#,
        runtime / 60, runtime % 60,
        active_count, 5, queued_count, synced_count,
        render_active_table(&active_from_crons),
        render_queue(&queue_preview),
        synced_count,
        render_synced_table(&all_synced),
        cron_list.len(),
        render_cron_table(&cron_list)
    );
    
    Html(html)
}

fn render_active_table(processes: &[models::ProcessStatus]) -> String {
    if processes.is_empty() {
        return "<tr><td colspan='5'>No active processes (check cron list)</td></tr>".to_string();
    }
    
    processes.iter().map(|p| {
        let computed = p.computed_slot.map_or("-".to_string(), |s| s.to_string());
        let current = p.current_slot.map_or("-".to_string(), |s| s.to_string());
        let deficit = p.deficit().map_or("-".to_string(), |d| {
            if d == 0 {
                "<span class='synced'>SYNCED</span>".to_string()
            } else {
                format!("<span class='deficit'>{}</span>", d)
            }
        });
        
        // Calculate rate based on cron creation time if available
        let rate = if let Some(cron_created) = p.cron_created_at {
            let minutes_elapsed = (Utc::now() - cron_created).num_seconds() as f64 / 60.0;
            if minutes_elapsed > 0.0 && p.metrics.total_slots_advanced > 0 {
                let calc_rate = p.metrics.total_slots_advanced as f64 / minutes_elapsed;
                format!("{:.1}", calc_rate)
            } else if p.metrics.avg_sync_rate > 0.0 {
                format!("{:.1}", p.metrics.avg_sync_rate)
            } else {
                "-".to_string()
            }
        } else if p.metrics.avg_sync_rate > 0.0 {
            format!("{:.1}", p.metrics.avg_sync_rate)
        } else {
            "-".to_string()
        };
        
        // Show process ID shortened if too long
        let process_id_display = if p.process_id.len() > 43 {
            format!("{}...{}", &p.process_id[..20], &p.process_id[p.process_id.len()-20..])
        } else {
            p.process_id.clone()
        };
        
        format!(
            "<tr><td title='{}'>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            p.process_id, process_id_display, computed, current, deficit, rate
        )
    }).collect::<Vec<_>>().join("\n")
}

fn render_queue(processes: &[models::ProcessStatus]) -> String {
    if processes.is_empty() {
        return "<tr><td colspan='3'>Queue is empty</td></tr>".to_string();
    }
    
    processes.iter().enumerate().map(|(i, p)| {
        let current_slot = p.current_slot.map_or("-".to_string(), |s| s.to_string());
        format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
            i + 1, p.process_id, current_slot
        )
    }).collect::<Vec<_>>().join("\n")
}

fn render_synced_table(processes: &[models::ProcessStatus]) -> String {
    if processes.is_empty() {
        return "<tr><td colspan='7'>No synced processes yet</td></tr>".to_string();
    }
    
    processes.iter().map(|p| {
        let computed = p.computed_slot.map_or("-".to_string(), |s| s.to_string());
        let current = p.current_slot.map_or("-".to_string(), |s| s.to_string());
        
        // Check if still synced
        let is_still_synced = p.computed_slot == p.current_slot;
        let current_display = if !is_still_synced && p.current_slot.is_some() {
            format!("<span class='error'>{}</span>", current)
        } else {
            current
        };
        
        // Format reserves in a structured way with sorted tokens
        let (hb_reserves_str, ao_reserves_str) = match (&p.hb_reserves, &p.ao_reserves) {
            (Some(hb), Some(ao)) => {
                // Get all valid token IDs from both sources
                let mut all_tokens: Vec<String> = hb.keys()
                    .chain(ao.keys())
                    .filter(|k| k.len() == 43 && !["TokenA", "TokenB", "K"].contains(&k.as_str()))
                    .cloned()
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                
                // Sort tokens for consistent display
                all_tokens.sort();
                
                if all_tokens.is_empty() {
                    ("No token reserves".to_string(), "No token reserves".to_string())
                } else {
                    // Format reserves showing full amounts
                    let hb_str = all_tokens.iter()
                        .map(|token| {
                            let amount = hb.get(token).map(|s| s.as_str()).unwrap_or("0");
                            format!("<div title='{}'>{}</div>", token, amount)
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    
                    // Format AO reserves with same order
                    let ao_str = all_tokens.iter()
                        .map(|token| {
                            let amount = ao.get(token).map(|s| s.as_str()).unwrap_or("0");
                            let style = if hb.get(token) != ao.get(token) {
                                "style='color: #666666; text-decoration: underline;'"
                            } else {
                                ""
                            };
                            format!("<div {} title='{}'>{}</div>", style, token, amount)
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    
                    (hb_str, ao_str)
                }
            },
            (Some(hb), None) => {
                let mut tokens: Vec<_> = hb.keys()
                    .filter(|k| k.len() == 43 && !["TokenA", "TokenB", "K"].contains(&k.as_str()))
                    .collect();
                tokens.sort();
                
                let hb_str = if tokens.is_empty() {
                    "No token reserves".to_string()
                } else {
                    tokens.iter()
                        .map(|token| {
                            let amount = hb.get(*token).unwrap();
                            format!("<div title='{}'>{}</div>", token, amount)
                        })
                        .collect::<Vec<_>>()
                        .join("")
                };
                (hb_str, "<span style='color: #999;'>Fetching...</span>".to_string())
            },
            (None, Some(ao)) => {
                let mut tokens: Vec<_> = ao.keys()
                    .filter(|k| k.len() == 43)
                    .collect();
                tokens.sort();
                
                let ao_str = if tokens.is_empty() {
                    "No token reserves".to_string()
                } else {
                    tokens.iter()
                        .map(|token| {
                            let amount = ao.get(*token).unwrap();
                            format!("<div title='{}'>{}</div>", token, amount)
                        })
                        .collect::<Vec<_>>()
                        .join("")
                };
                ("<span style='color: #999;'>Fetching...</span>".to_string(), ao_str)
            },
            _ => ("<span style='color: #999;'>Fetching...</span>".to_string(), "<span style='color: #999;'>Fetching...</span>".to_string())
        };
        
        // Check if reserves match
        let match_status = match (&p.hb_reserves, &p.ao_reserves) {
            (None, _) | (_, None) => "<span style='color: #999;'>[FETCHING]</span>",
            _ => match p.reserves_match() {
                Some(true) => "<span class='synced'>[OK]</span>",
                Some(false) => "<span class='error'>[DIFF]</span>",
                None => "<span style='color: #999;'>[FETCHING]</span>",
            }
        };
        
        format!(
            "<tr><td title='{}'>{}</td><td>{}</td><td>{}</td><td style='font-size: 11px; font-family: monospace;'>{}</td><td style='font-size: 11px; font-family: monospace;'>{}</td><td>{}</td></tr>",
            p.process_id, p.process_id, computed, current_display, hb_reserves_str, ao_reserves_str, match_status
        )
    }).collect::<Vec<_>>().join("\n")
}

fn render_cron_table(cron_items: &[CronItem]) -> String {
    if cron_items.is_empty() {
        return "<tr><td colspan='4'>No active crons</td></tr>".to_string();
    }
    
    cron_items.iter().map(|item| {
        // Extract process ID from path (format: /processId~process@1.0/now)
        let process_id = item.path
            .strip_prefix("/")
            .and_then(|p| p.split("~").next())
            .unwrap_or("unknown");
        
        // Format timestamp
        let created = chrono::DateTime::from_timestamp_millis(item.created_at as i64)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        
        // Shorten task ID for display
        let task_id_display = if item.task_id.len() > 20 {
            format!("{}...{}", &item.task_id[..10], &item.task_id[item.task_id.len()-7..])
        } else {
            item.task_id.clone()
        };
        
        format!(
            "<tr><td title='{}'>{}</td><td>{}</td><td title='{}'>{}</td><td>{}</td></tr>",
            item.task_id, task_id_display, item.cron_type, item.path, process_id, created
        )
    }).collect::<Vec<_>>().join("\n")
}

async fn get_status(State(state): State<Arc<AppState>>) -> Json<ApiResponse<ApiStatus>> {
    let (active_count, queued_count, synced_count) = state.queue.get_status().await;
    let runtime = (Utc::now() - state.start_time).num_seconds() as u64;
    
    let status = ApiStatus {
        active_count,
        queued_count,
        synced_count,
        total_count: active_count + queued_count + synced_count,
        runtime_seconds: runtime,
        active_processes: state.queue.get_active_processes().await,
        queue_preview: state.queue.get_queue_preview(10).await,
        recent_synced: state.queue.get_recent_synced(10).await,
    };
    
    Json(ApiResponse {
        success: true,
        data: Some(status),
        error: None,
    })
}

async fn get_state(State(state): State<Arc<AppState>>) -> Result<Json<models::StateFile>, StatusCode> {
    state::save_state(&state.queue).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    let content = tokio::fs::read_to_string("hydration-state.json")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    let state_file: models::StateFile = serde_json::from_str(&content)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    Ok(Json(state_file))
}

async fn add_to_queue(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AddProcessRequest>,
) -> Json<ApiResponse<String>> {
    let config = ProcessConfig {
        name: request.name,
        process_id: request.process_id.clone(),
        base_url: request.base_url,
    };
    
    match state.queue.add_to_queue(config).await {
        Ok(_) => Json(ApiResponse {
            success: true,
            data: Some(format!("Process {} added to queue", request.process_id)),
            error: None,
        }),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            error: Some(e),
        }),
    }
}

async fn restart_process(
    State(state): State<Arc<AppState>>,
    Path(process_id): Path<String>,
) -> Json<ApiResponse<String>> {
    match state.queue.restart_process(&process_id).await {
        Ok(_) => Json(ApiResponse {
            success: true,
            data: Some(format!("Process {} restarted", process_id)),
            error: None,
        }),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            error: Some(e),
        }),
    }
}