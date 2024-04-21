use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use std::time::{Duration, Instant};
use sysinfo::PidExt;
use sysinfo::{DiskExt, NetworkExt, NetworksExt, ProcessExt, ProcessorExt, System, SystemExt};

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
    pub disk_stats: Vec<DiskStats>,
    pub process_count: usize,
    pub uptime: u64,
    pub processes: Vec<ProcessStats>,
    pub system_name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NetworkStats {
    pub name: String,
    pub received: u64,
    pub transmitted: u64,
    pub packets_received: u64,
    pub packets_transmitted: u64,
    pub errors_on_received: u64,
    pub errors_on_transmitted: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DiskStats {
    pub name: String,
    pub total_space: u64,
    pub available_space: u64,
    pub is_removable: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LoadAverage {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProcessStats {
    pub name: String,
    pub pid: i32,
    pub cpu_usage: f32,
    pub memory: u64,
    pub virtual_memory: u64,
    pub start_time: u64,
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
    let core_count = system.physical_core_count().unwrap_or_default();
    let networks = system.networks();
    let network_stats = networks
        .iter()
        .map(|(name, data)| NetworkStats {
            name: name.to_string(),
            received: data.received(),
            transmitted: data.transmitted(),
            packets_received: data.packets_received(),
            packets_transmitted: data.packets_transmitted(),
            errors_on_received: data.errors_on_received(),
            errors_on_transmitted: data.errors_on_transmitted(),
        })
        .collect();
    let cpu_usage = system.global_processor_info().cpu_usage();
    let disks = system.disks();
    let disk_stats = disks
        .iter()
        .map(|disk| DiskStats {
            name: disk.name().to_string_lossy().to_string(),
            total_space: disk.total_space(),
            available_space: disk.available_space(),
            is_removable: disk.is_removable(),
        })
        .collect();
    let process_count = system.processes().len();
    let uptime = system.uptime();
    let processes = system
        .processes()
        .iter()
        .map(|(pid, process)| ProcessStats {
            name: process.name().to_string(),
            pid: pid.as_u32() as i32,
            cpu_usage: process.cpu_usage(),
            memory: process.memory(),
            virtual_memory: process.virtual_memory(),
            start_time: process.start_time(),
        })
        .collect();
    let system_name = system.name().unwrap_or_else(|| "Unknown".to_string());

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
        disk_stats,
        process_count,
        uptime,
        processes,
        system_name,
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
