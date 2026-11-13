use std::{thread, time::Duration};
use sysinfo::System;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut port = serialport::new("/dev/ttyACM0", 115_200)
        .timeout(Duration::from_millis(500))
        .open()?;
    let mut system_info = System::new_all().unwrap();
    system_info.refresh_all();
    loop {
        // sleep because cpu usage is delta
        thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        system_info.refresh_cpu_usage();
        system_info.refresh_memory();
        let cpu_usage = system_info.global_cpu_usage();
        let mem_usage = system_info.used_memory();
        let line = format!(
            "cpu:{:.2}%,mem:{}\n",
            cpu_usage,
            mem_usage / 1024 / 1024 / 1024
        );
        port.write_all(&line.into_bytes())?;
        std::thread::sleep(Duration::from_secs(1));
    }
}
