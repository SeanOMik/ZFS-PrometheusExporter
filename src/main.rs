use actix_web::{get, App, HttpServer, Responder};
use actix_web::middleware::Logger;

use libzetta::zpool::{ZpoolOpen3, ZpoolEngine, Vdev, Health, vdev::ErrorStatistics, Reason};

use prometheus::{Encoder, IntCounter, Registry};

use clap::Parser;

use std::{collections::HashMap, string::FromUtf8Error, process::Command};

use log::{error, debug};

fn encode_metrics(reg: &Registry) -> Result<String, FromUtf8Error> {
    let mut buffer: Vec<u8> = Vec::new();
    let encoder = prometheus::TextEncoder::new();
    encoder.encode(&reg.gather(), &mut buffer)
        .unwrap(); // TODO

    String::from_utf8(buffer.clone())
}

fn register_intcounter(reg: &Registry, name: &str, help: &str, val: u64) -> prometheus::Result<()> {
    let counter = IntCounter::new(name, help)?;
    counter.inc_by(val);
    reg.register(Box::new(counter))?;

    Ok(())
}

fn register_health(labels: HashMap<String, String>, health: Health) -> prometheus::Result<Vec<Registry>> {
    let mut labels = labels;
    labels.insert(String::from("field_type"), String::from("enum"));

    labels.insert(String::from("state"), String::from("online"));
    let online_reg = Registry::new_custom(Some("zfs".to_string()), Some(labels.clone()))?;
    let online_val = match health {
        Health::Online => 1,
        _ => 0,
    };
    register_intcounter(&online_reg, "health", "The health of the device. This is an enum.", online_val)?;

    labels.insert(String::from("state"), String::from("degraded"));
    let degraded_reg = Registry::new_custom(Some("zfs".to_string()), Some(labels.clone()))?;
    let degraded_val = match health {
        Health::Degraded => 1,
        _ => 0,
    };
    register_intcounter(&degraded_reg, "health", "The health of the device. This is an enum.", degraded_val)?;

    labels.insert(String::from("state"), String::from("faulted"));
    let faulted_reg = Registry::new_custom(Some("zfs".to_string()), Some(labels.clone()))?;
    let faulted_val = match health {
        Health::Faulted => 1,
        _ => 0,
    };
    register_intcounter(&faulted_reg, "health", "The health of the device. This is an enum.", faulted_val)?;

    labels.insert(String::from("state"), String::from("offline"));
    let offline_reg = Registry::new_custom(Some("zfs".to_string()), Some(labels.clone()))?;
    let offline_val = match health {
        Health::Offline => 1,
        _ => 0,
    };
    register_intcounter(&offline_reg, "health", "The health of the device. This is an enum.", offline_val)?;

    labels.insert(String::from("state"), String::from("available"));
    let available_reg = Registry::new_custom(Some("zfs".to_string()), Some(labels.clone()))?;
    let available_val = match health {
        Health::Available => 1,
        _ => 0,
    };
    register_intcounter(&available_reg, "health", "The health of the device. This is an enum.", available_val)?;

    labels.insert(String::from("state"), String::from("unavailable"));
    let unavailable_reg = Registry::new_custom(Some("zfs".to_string()), Some(labels.clone()))?;
    let unavailable_val = match health {
        Health::Unavailable => 1,
        _ => 0,
    };
    register_intcounter(&unavailable_reg, "health", "The health of the device. This is an enum.", unavailable_val)?;

    labels.insert(String::from("state"), String::from("removed"));
    let removed_reg = Registry::new_custom(Some("zfs".to_string()), Some(labels.clone()))?;
    let removed_val = match health {
        Health::Removed => 1,
        _ => 0,
    };
    register_intcounter(&removed_reg, "health", "The health of the device. This is an enum.", removed_val)?;

    Ok(vec![online_reg, degraded_reg, faulted_reg, offline_reg, available_reg, unavailable_reg, removed_reg])
}

fn register_error_stats(reg: &Registry, error_stats: ErrorStatistics) -> prometheus::Result<()> {
    register_intcounter(reg, "read_errors", "The amount of I/O errors that occurred during reading", error_stats.read)?;
    register_intcounter(reg, "write_errors", "The amount of I/O errors that occurred during writing", error_stats.write)?;
    register_intcounter(reg, "checksum_errors", "The amount of checksum errors, meaning the device returned corrupted data from a read request", error_stats.checksum)?;

    Ok(())
}

fn register_vdev_stats(vdev: &Vdev, vdev_device: &Device, vdev_name: String, start_labels: HashMap<String, String>) -> prometheus::Result<Registry> {
    let mut labels = start_labels.clone();
    labels.insert(String::from("device_type"), String::from("vdev"));
    labels.remove("vdev"); // Remove vdev since its not needed because of "source_name"
    labels.insert(String::from("device_name"), vdev_name.clone());

    let vdev_reg = Registry::new_custom(Some("zfs".to_string()), Some(labels))?;
    vdev_device.io_stats.collect_metrics(&vdev_reg)?;
    register_error_stats(&vdev_reg, vdev.error_statistics().clone())?;
    
    register_intcounter(&vdev_reg, "disk_count", "Total count of drives in this pool or vdev", vdev.disks().len() as u64)?;

    Ok(vdev_reg)
}

#[get("/metrics")]
async fn metrics_endpoint() -> impl Responder {
    let zpool = ZpoolOpen3::default();
    let all_pools = zpool.all().unwrap(); // TODO: Dont unwrap

    let mut registries = Vec::new();

    for pool in all_pools.iter() {
        // Print some stuff that can be used for later features.
        // My pool is in a healthy state currently, so I can't actually work on these
        // to see what they output.
        {
            let logs = pool.logs();
            if logs.len() != 0 {
                debug!("Found pool logs!: {:?}", logs);
            }

            if let Some(errors) = pool.errors() {
                debug!("Found pool errors!: {}", errors);
            }

            // Currently reason is only a wrapper around String.
            if let Some(Reason::Other(reason)) = pool.reason() {
                debug!("Found pool 'reason': {}", reason);
            }
        }

        let mut labels = HashMap::new();
        labels.insert(String::from("device_type"), String::from("pool"));
        labels.insert(String::from("pool"), pool.name().clone());
        labels.insert(String::from("device_name"), pool.name().clone());

        // Create a registry for general pool metrics
        let pool_reg = Registry::new_custom(Some("zfs".to_string()), Some(labels.clone())).unwrap();

        register_intcounter(&pool_reg, "vdev_count", "Count of vdevs in this pool", pool.vdevs().len() as u64).unwrap();
        register_intcounter(&pool_reg, "spare_count", "The amount of spare drives", pool.spares().len() as u64).unwrap();

        // Calculate the total drive count and register it as a metric.
        let total_disk_count = IntCounter::new("disk_count", "Total count of drives in this pool or vdev").unwrap();
        for vdev in pool.vdevs().iter() {
            total_disk_count.inc_by(vdev.disks().len() as u64);
        }
        pool_reg.register(Box::new(total_disk_count)).unwrap();

        // Register pool health
        registries.extend(register_health(labels.clone(), pool.health().clone()).unwrap());
        register_error_stats(&pool_reg, pool.error_statistics().clone()).unwrap();

        // Run the zpool iostat command to get io stat information of all the pool, its vdevs and disks.
        let mut cmd = Command::new("zpool");
        cmd.args(["iostat", "-Hpvy", pool.name().as_str(), "1", "1"]);
        let output = cmd.output();
        let output = output.expect(&format!("Failure to execute `zpool iostat`"));

        // Check if the `zpool iostat` command executed successfully.
        if !output.status.success() {
            error!("Failed to execute `zpool iostat`!");
            error!("Full command: `{:?} {}`", cmd.get_program(), cmd.get_args()
                .into_iter()
                .map(|arg| arg.to_str().unwrap().to_string())
                .collect::<Vec<String>>()
                .join(" "));

            error!("stdout:\n{:?}", output.stdout);
            error!("stderr:\n{:?}", output.stderr);
            error!("exit code: {}", output.status);
            panic!("Failure to execute zpool iostat!");
        }
        let output = String::from_utf8(output.stdout)
            .expect(&format!("Failure to convert output of `zpool iostat` to utf8."));

        let devices = Device::parse_from_stdout(output);

        // Get the pool from the devices and collect the io stats
        if let Some(pool_dev) = devices.iter().find(|dev| dev.name == pool.name().clone()) {
            pool_dev.io_stats.collect_metrics(&pool_reg).unwrap();

            // Get the raw size of the pool.
            let output = String::from_utf8(
                Command::new("zpool")
                    .args(["list", "-Hp", pool.name().as_str()])
                    .output()
                    .expect(&format!("Failure to execute `zpool iostat {} -v 1 2`", pool.name()))
                .stdout).expect(&format!("Failure to convert output of `zpool iostat {} -v 1 2` to utf8.", pool.name()));

            // Extract the size from the output
            let cols: Vec<&str> = output.split("\t").collect();
            if cols.len() == 11 {
                let size: u64 = cols[1].parse().unwrap();
                register_intcounter(&pool_reg, "raw_size", "The raw size of this device (this is not the usable space)", size).unwrap();
            }
        }

        // Push pool metrics
        registries.push(pool_reg);

        // The output of the zpool commands has vdevs listed before the disks in the vdev.
        let mut last_vdev: Option<&Device> = None;
        let mut last_vdev_data: Option<&Vdev> = None;
        for device in devices.iter() {
            // Skip any pools or vdevs
            if device.name == pool.name().clone() {
                // Skip pool
            } else if device.is_pool_or_vdev() {
                // Register the metrics of the last vdev before overwriting it.
                if let Some(vdev) = last_vdev_data {
                    let reg = register_vdev_stats(vdev, device, device.name.clone(), labels.clone()).unwrap();

                    registries.push(reg);
                }

                last_vdev = Some(device); // Store this device as the last vdev
                last_vdev_data = None;
            } else { // Register metrics for this disk
                let mut labels = labels.clone();
                labels.insert(String::from("device_name"), device.name.clone());
                labels.insert(String::from("device_type"), String::from("disk"));

                // If vdev is set, add the vdev label for this disk.
                if let Some(vdev) = last_vdev {
                    labels.insert(String::from("vdev"), vdev.name.clone());
                }

                // Create the device metric registry and collect io stats metrics
                let device_reg = Registry::new_custom(Some("zfs".to_string()), Some(labels.clone())).unwrap();
                device.io_stats.collect_metrics(&device_reg).unwrap();

                // Find the disk, and its vdev in the pool. After its found, register the disk's health and error stats.
                for pool_vdev in pool.vdevs().iter() {
                    if let Some(pool_disk) = pool_vdev.disks().iter().find(|disk| String::from(disk.path().as_os_str().to_str().unwrap_or("")).contains(&device.name)) {
                        registries.extend(register_health(labels, pool_disk.health().clone()).unwrap());
                        register_error_stats(&device_reg, pool_disk.error_statistics().clone()).unwrap();

                        last_vdev_data = Some(pool_vdev);
                        break;
                    }
                }

                registries.push(device_reg);
            }
        }

        // Push the last vdev to the registry list
        if let (Some(device), Some(vdev)) = (last_vdev, last_vdev_data) {
            registries.push(register_vdev_stats(vdev, device, device.name.clone(), labels.clone()).unwrap());
        }
    }

    // Construct the response string from all registeries.
    let mut resp = String::new();
    for reg in registries.iter() {
        resp.push_str(&encode_metrics(&reg).unwrap());
        resp.push_str("\n");
    }

    return resp;
}

#[derive(Debug, PartialEq, Eq)]
struct IoStats {
    capacity: Option<u64>,
    available: Option<u64>,

    read_op: u64,
    write_op: u64,

    read_band: u64,
    write_band: u64,
}

impl IoStats {
    fn new(capacity: Option<u64>, available: Option<u64>, read_op: u64, write_op: u64, read_band: u64, write_band: u64) -> Self {
        Self {
            capacity,
            available,
            read_op,
            write_op,
            read_band,
            write_band,
        }
    }

    fn collect_metrics(&self, reg: &Registry) -> prometheus::Result<()> {
        if let (Some(capacity), Some(available)) = (self.capacity, self.available) {
            register_intcounter(&reg, "capacity", "The capacity of the device in bytes", capacity)?;
            register_intcounter(&reg, "available", "The available bytes in the device", available)?;
        }

        register_intcounter(&reg, "read_operations", "The read operations for this device per second", self.read_op)?;
        register_intcounter(&reg, "write_operations", "The write operations for this device per second", self.write_op)?;
        register_intcounter(&reg, "read_bandwidth", "The read bandwidth for this device in bytes per second", self.read_band)?;
        register_intcounter(&reg, "write_bandwidth", "The write bandwidth for this device in bytes per second", self.write_band)?;

        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Device {
    name: String,
    io_stats: IoStats,
}

impl Device {
    fn new(name: String, io_stats: IoStats) -> Self {
        Self {
            name,
            io_stats
        }
    }

    fn is_pool_or_vdev(&self) -> bool {
        self.io_stats.available.is_some() && self.io_stats.capacity.is_some()
    }

    fn parse_from_stdout(stdout: String) -> Vec<Device> {
        let mut input = stdout.as_str();
    
        // Remove tailing \n
        if input.ends_with("\n") {
            input = &input[..input.len()];
        }

        let mut stats: Vec<Vec<&str>> = input.split("\n").collect::<Vec<&str>>().iter().map(|s| s.split("\t").collect::<Vec<&str>>()).collect();

        // remove all rows that are not of length 7 or have empty columns.
        stats.retain(|l| l.len() == 7 && l.iter().all(|&s| !s.is_empty()));

        let mut parsed = Vec::new();
        for row in stats.iter() {
            let name = row[0];
            let alloc = row[1].parse().unwrap();
            let free = row[2].parse().unwrap();
            let read_op = row[3].parse().unwrap();
            let write_op = row[4].parse().unwrap();
            let read_band = row[5].parse().unwrap();
            let write_band = row[6].parse().unwrap();

            // This is done since these fields can be Some(0), but alloc would never be unless
            // its the device. free can be 0 if the pool is filled.
            let (alloc, free) = if alloc == 0 {
                (None, None)
            } else {
                (Some(alloc), Some(free))
            };

            parsed.push(Device::new(String::from(name), 
                IoStats::new(alloc, free, read_op, write_op, read_band, write_band)));
        }

        return parsed;
    }
}

/// ZFS metrics exporter for Prometheus!
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
   /// The address to bind and listen from.
   #[arg(short, long, default_value_t = String::from("0.0.0.0"))]
   bind_address: String,

   /// The port to listen on.
   #[arg(short, long, default_value_t = 8080)]
   port: u16,

   /// The lowest log level (off, error, warn, info, debug, or trace).
   #[arg(long, default_value_t = String::from("info"))]
   log_level: String,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();

    // Convert log level string to an enum
    let log_level = args.log_level.to_lowercase();
    let log_level = match log_level.as_str() {
        "off" => log::LevelFilter::Off,
        "error" => log::LevelFilter::Error,
        "warn" => log::LevelFilter::Warn,
        "info" => log::LevelFilter::Info,
        "debug" => log::LevelFilter::Debug,
        "trace" => log::LevelFilter::Trace,
        _ => panic!("Unknown log level! {}, expected off, error, warn, info, debug, or trace!", log_level),
    };

    // Create logger
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message
            ))
        })
        .level(log_level)
        .chain(std::io::stdout())
        //.chain(fern::log_file("output.log")?)
        .apply().expect("Failure to initialize fern logger!");

    HttpServer::new(|| {
        App::new()
            .wrap(Logger::default())
            .service(metrics_endpoint)
    })
    .bind((args.bind_address, args.port))?
    .run()
    .await
}