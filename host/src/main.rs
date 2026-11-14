use std::{path::Path, time::Duration};
use sysinfo::System;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut port = serialport::new("/dev/ttyACM0", 115_200)
        .timeout(Duration::from_millis(500))
        .open()?;

    let mut components = sysinfo::Components::new_with_refreshed_list().unwrap();
    let mut disks = sysinfo::Disks::new_with_refreshed_list().unwrap();

    let mut system_info = System::new_all().unwrap();
    system_info.refresh_all();
    loop {
        // sleep because cpu usage is delta
        std::thread::sleep(Duration::from_secs(1));
        system_info.refresh_cpu_usage();
        system_info.refresh_memory();
        components.refresh(false);
        disks.refresh(false);

        let cpu_temp: f32 = components
            .iter()
            .find(|c| c.label() == "k10temp Tctl")
            .and_then(|t| t.temperature())
            .unwrap_or(-1.);

        let cpu_usage = system_info.global_cpu_usage();
        let mem_usage = system_info.used_memory();

        let disk_usage: f64;
        if let Some(disk) = disks.iter().find(|d| d.mount_point() == Path::new("/")) {
            disk_usage = (disk.total_space() as f64 - disk.available_space() as f64)
                / disk.total_space() as f64
                * 100.;
        } else {
            disk_usage = -1.;
        }

        let line = format!(
            "cpu:{:.1}%,mem:{},temp:{:.1},disk:{:.1}%\n",
            cpu_usage,
            mem_usage / 1024 / 1024 / 1024,
            cpu_temp,
            disk_usage
        );
        port.write_all(&line.into_bytes())?;
    }
}
