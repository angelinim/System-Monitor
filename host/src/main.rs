use serialport::SerialPort;
use std::{path::Path, time::Duration};
use sysinfo::{Components, Disks, Gpus, System};

fn main() {
    let mut components = sysinfo::Components::new_with_refreshed_list().unwrap();
    let mut disks = sysinfo::Disks::new_with_refreshed_list().unwrap();
    let mut gpus = sysinfo::Gpus::new_with_refreshed_list().unwrap();
    let mut system_info = System::new_all().unwrap();
    system_info.refresh_all();

    loop {
        if let Some(mut port) = connect() {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let line = build_line(&mut system_info, &mut components, &mut disks, &mut gpus);
                if let Err(err) = port.write_all(&line.into_bytes()) {
                    println!("Something went wrong! Err: {err}");
                    break;
                }
            }
        } else {
            println!("sleeping for 5 seconds and retrying connection");
            std::thread::sleep(Duration::from_secs(5));
        }
    }
}

fn find_pico() -> Option<String> {
    serialport::available_ports()
        .ok()?
        .into_iter()
        .find(|port|
            matches!(
                &port.port_type,
                serialport::SerialPortType::UsbPort(info) if info.vid == 0xc0de && info.pid == 0xcafe
            )
        ).map(|p|p.port_name)
}

fn connect() -> Option<Box<dyn SerialPort>> {
    if let Some(port_name) = find_pico() {
        match serialport::new(&port_name, 115_200)
            .timeout(Duration::from_millis(500))
            .open()
        {
            Ok(port) => {
                println!("Connected on {port_name}");
                Some(port)
            }
            Err(err) => {
                println!("Error opening serial port {port_name}: Err {err}");
                None
            }
        }
    } else {
        println!("Could not find pico!");
        None
    }
}

fn build_line(
    system_info: &mut System,
    components: &mut Components,
    disks: &mut Disks,
    gpus: &mut Gpus,
) -> String {
    system_info.refresh_cpu_usage();
    system_info.refresh_memory();
    components.refresh(false);
    disks.refresh(false);
    gpus.refresh(false);

    // only will ever have one gpu
    let gpu = gpus.into_iter().next();
    let gpu_usage: f32 = gpu.and_then(|g| g.usage()).unwrap_or(-1.);
    // amdgpu junction is being used because this is the hottest
    // point on the card and will govern throttling
    let gpu_temp: f32 = components
        .iter()
        .find(|c| c.label() == "amdgpu junction")
        .and_then(|t| t.temperature())
        .unwrap_or(-1.);

    let cpu_temp: f32 = components
        .iter()
        .find(|c| c.label() == "k10temp Tctl")
        .and_then(|t| t.temperature())
        .unwrap_or(-1.);

    let cpu_usage = system_info.global_cpu_usage();
    let mem_usage = system_info.used_memory() as f32;

    let disk_usage: f64;
    if let Some(disk) = disks.iter().find(|d| d.mount_point() == Path::new("/")) {
        disk_usage = (disk.total_space() as f64 - disk.available_space() as f64)
            / disk.total_space() as f64
            * 100.;
    } else {
        disk_usage = -1.;
    }

    format!(
        "cpu:{:.1}%,mem:{:.1}GB,cpu_temp:{:.1},disk:{:.1}%,gpu_usage:{:.1},gpu_temp:{:.1}\n",
        cpu_usage,
        mem_usage / 1024. / 1024. / 1024.,
        cpu_temp,
        disk_usage,
        gpu_usage,
        gpu_temp
    )
}
