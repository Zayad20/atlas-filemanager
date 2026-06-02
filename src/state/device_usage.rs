use std::collections::VecDeque;
use std::time::Instant;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System, Disks};

pub struct DeviceUsageState {
    pub system: System,
    pub disks: Disks,
    pub cpu_history: VecDeque<f32>,
    pub ram_history: VecDeque<f32>,
    pub last_refresh: Instant,
}

impl Default for DeviceUsageState {
    fn default() -> Self {
        let mut system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        system.refresh_cpu_usage();
        system.refresh_memory();

        let mut cpu_history = VecDeque::with_capacity(60);
        let mut ram_history = VecDeque::with_capacity(60);
        for _ in 0..60 {
            cpu_history.push_back(0.0);
            ram_history.push_back(0.0);
        }

        let disks = Disks::new_with_refreshed_list();

        Self {
            system,
            disks,
            cpu_history,
            ram_history,
            last_refresh: Instant::now(),
        }
    }
}

impl DeviceUsageState {
    pub fn update(&mut self) {
        if self.last_refresh.elapsed().as_secs_f32() >= 1.0 {
            self.system.refresh_cpu_usage();
            self.system.refresh_memory();
            self.disks.refresh(true);

            let global_cpu = self.system.global_cpu_usage();
            let total_ram = self.system.total_memory() as f32;
            let used_ram = self.system.used_memory() as f32;
            let ram_percent = if total_ram > 0.0 {
                (used_ram / total_ram) * 100.0
            } else {
                0.0
            };

            self.cpu_history.pop_front();
            self.cpu_history.push_back(global_cpu);

            self.ram_history.pop_front();
            self.ram_history.push_back(ram_percent);

            self.last_refresh = Instant::now();
        }
    }
}
