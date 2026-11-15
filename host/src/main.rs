use std::{path::Path, time::Duration};
use sysinfo::{Components, Disks, System};

fn main() {
    let mut components = sysinfo::Components::new_with_refreshed_list().unwrap();
    let mut disks = sysinfo::Disks::new_with_refreshed_list().unwrap();
    let mut system_info = System::new_all().unwrap();
    system_info.refresh_all();

    loop {
        match serialport::new("/dev/ttyACM0", 115_200)
            .timeout(Duration::from_millis(500))
            .open()
        {
            Ok(mut port) => {
                println!("Connected on /dev/ttyACM0");
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    let line = build_line(&mut system_info, &mut components, &mut disks);
                    if let Err(err) = port.write_all(&line.into_bytes()) {
                        println!("Something went wrong! Err: {err}");
                        break;
                    }
                }
            }
            Err(err) => {
                println!("Could not find pico on /dev/ttyACM0... Err: {err}");
                println!("sleeping for 5 seconds and retrying connection");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

fn build_line(system_info: &mut System, components: &mut Components, disks: &mut Disks) -> String {
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

    format!(
        "cpu:{:.1}%,mem:{},temp:{:.1},disk:{:.1}%\n",
        cpu_usage,
        mem_usage / 1024 / 1024 / 1024,
        cpu_temp,
        disk_usage
    )
}
