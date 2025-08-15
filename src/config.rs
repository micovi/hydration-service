use serde::{Deserialize, Serialize};
use std::fs;
use anyhow::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub server: ServerConfig,
    pub hyperbeam: HyperbeamConfig,
    pub ao: AoConfig,
    pub monitoring: MonitoringConfig,
    pub limits: LimitsConfig,
    pub ui: UiConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub port: u16,
    pub host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyperbeamConfig {
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AoConfig {
    pub cu_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    pub cron_list_interval: u64,
    pub queue_slots_interval: u64,
    pub synced_pools_interval: u64,
    pub monitor_loop_interval: u64,
    pub queue_slots_delay: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsConfig {
    pub max_active_processes: usize,
    pub queue_preview_limit: usize,
    pub queue_check_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    pub refresh_interval: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                port: 8080,
                host: "0.0.0.0".to_string(),
            },
            hyperbeam: HyperbeamConfig {
                base_url: "http://65.108.7.125:8734".to_string(),
            },
            ao: AoConfig {
                cu_url: "https://cu.ao-testnet.xyz".to_string(),
            },
            monitoring: MonitoringConfig {
                cron_list_interval: 15,
                queue_slots_interval: 30,
                synced_pools_interval: 60,
                monitor_loop_interval: 15,
                queue_slots_delay: 10,
            },
            limits: LimitsConfig {
                max_active_processes: 5,
                queue_preview_limit: 10,
                queue_check_limit: 20,
            },
            ui: UiConfig {
                refresh_interval: 5,
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                format: "full".to_string(),
            },
        }
    }
}

impl ServiceConfig {
    pub fn load() -> Result<Self> {
        // Try to load from config.toml, fall back to defaults if not found
        if let Ok(contents) = fs::read_to_string("config.toml") {
            let config: ServiceConfig = toml::from_str(&contents)?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }
    
    pub fn save(&self) -> Result<()> {
        let toml_string = toml::to_string_pretty(&self)?;
        fs::write("config.toml", toml_string)?;
        Ok(())
    }
}