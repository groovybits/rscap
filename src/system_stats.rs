use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use sysinfo::{NetworkExt, NetworksExt};
use sysinfo::{ProcessorExt, System, SystemExt};

static SYSTEM: Lazy<Mutex<System>> = Lazy::new(|| {
    let mut system = System::new_all();
    system.refresh_all(); // Initial refresh
    Mutex::new(system)
});

#[derive(Serialize, Deserialize, Debug)]
pub struct SystemStats {
    total_memory: u64,
    used_memory: u64,
    total_swap: u64,
    used_swap: u64,
    cpu_usage: f32,
    cpu_count: usize,
    core_count: usize,
    boot_time: u64,
    load_avg: LoadAverage,
    host_name: String,
    kernel_version: String,
    os_version: String,
    network_stats: Vec<NetworkStats>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct NetworkStats {
    name: String,
    received: u64,
    transmitted: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LoadAverage {
    one: f64,
    five: f64,
    fifteen: f64,
}

pub fn get_system_stats() -> SystemStats {
    // Access the lazily-initialized static system instance
    let mut system = SYSTEM.lock().unwrap();

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

    SystemStats {
        total_memory: system.total_memory(),
        used_memory: system.used_memory(),
        total_swap: system.total_swap(),
        used_swap: system.used_swap(),
        cpu_usage: system.global_processor_info().cpu_usage(),
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
