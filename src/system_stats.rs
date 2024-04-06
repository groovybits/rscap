use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use std::time::{Duration, Instant};
use sysinfo::{NetworkExt, NetworksExt, ProcessorExt, System, SystemExt};

static CACHED_STATS: Lazy<RwLock<(SystemStats, Instant)>> = Lazy::new(|| {
    let stats = get_system_stats_internal();
    RwLock::new((stats, Instant::now()))
});

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SystemStats {
    pub total_memory: u64,
    pub used_memory: u64,
    pub total_swap: u64,
    pub used_swap: u64,
    pub cpu_usage: f32,
    pub cpu_count: usize,
    pub core_count: usize,
    pub boot_time: u64,
    pub load_avg: LoadAverage,
    pub host_name: String,
    pub kernel_version: String,
    pub os_version: String,
    pub network_stats: Vec<NetworkStats>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NetworkStats {
    name: String,
    received: u64,
    transmitted: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LoadAverage {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

fn get_system_stats_internal() -> SystemStats {
    let mut system = System::new_all();
    system.refresh_all();

    let host_name = system.host_name().unwrap_or_else(|| "Unknown".to_string());
    let kernel_version = system
        .kernel_version()
        .unwrap_or_else(|| "Unknown".to_string());
    let os_version = system.os_version().unwrap_or_else(|| "Unknown".to_string());
    let sys_load_avg = system.load_average();
    let load_avg = LoadAverage {
        one: sys_load_avg.one,
        five: sys_load_avg.five,
        fifteen: sys_load_avg.fifteen,
    };

    let cpu_count = system.processors().len();
    let boot_time = system.boot_time();
    let core_count = system.physical_core_count().unwrap_or_else(|| 0);

    let networks = system.networks();
    let network_stats = networks
        .iter()
        .map(|(&ref name, data)| NetworkStats {
            name: name.to_string(),
            received: data.received(),
            transmitted: data.transmitted(),
        })
        .collect();

    let cpu_usage = system.global_processor_info().cpu_usage();

    SystemStats {
        total_memory: system.total_memory(),
        used_memory: system.used_memory(),
        total_swap: system.total_swap(),
        used_swap: system.used_swap(),
        cpu_usage,
        cpu_count,
        core_count,
        boot_time,
        load_avg,
        host_name,
        kernel_version,
        os_version,
        network_stats,
    }
}

pub fn get_system_stats() -> SystemStats {
    let cached_stats = CACHED_STATS.read().unwrap();
    let (stats, last_updated) = &*cached_stats;

    if last_updated.elapsed() > Duration::from_secs(60) {
        drop(cached_stats);
        let mut cached_stats_write = CACHED_STATS.write().unwrap();
        *cached_stats_write = (get_system_stats_internal(), Instant::now());
        cached_stats_write.0.clone()
    } else {
        stats.clone()
    }
}
