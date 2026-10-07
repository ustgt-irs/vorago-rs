use std::{
    fs::File,
    io::Read as _,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::bail;
use clap::Parser as _;
use cobs::CobsDecoderOwned;
use models::Response;
use spacepackets::CcsdsPacketReader;
use tmtc_utils::transport::serial::PacketTransportSerialCobs;

pub mod elf;
pub mod flash;
pub mod tc;

#[derive(clap::Parser)]
#[command(name = "image-loader", about = "VA416XX Image Loader Application")]
pub struct Cli {
    /// Serial port to use (overrides config.toml)
    #[arg(short, long)]
    pub port: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub enum Command {
    /// Send a ping command
    Ping,

    /// Set the active boot slot
    SetBootSlot {
        /// Boot slot to activate
        app: AppTarget,
    },

    /// Corrupt an app slot (for testing)
    Corrupt {
        /// Target slot to corrupt
        app: AppTarget,
    },

    /// Flash an ELF image to a target slot
    Flash {
        /// Target to flash
        target: Target,

        /// Path to the ELF image
        path: PathBuf,
    },
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum Target {
    /// Bootloader slot
    Bl,
    /// Application slot A
    A,
    /// Application slot B
    B,
}

/// Only app slots can be corrupted (not the bootloader)
#[derive(clap::ValueEnum, Clone, Debug)]
pub enum AppTarget {
    A,
    B,
}

#[derive(Debug, serde::Deserialize)]
pub struct Config {
    pub interface: Interface,
}

#[derive(Debug, serde::Deserialize)]
pub struct Interface {
    pub serial_port: Option<String>,
    pub udp_addr: Option<SocketAddr>,
}

impl Config {
    pub fn new_from_file() -> Self {
        let mut config_file =
            File::open(Path::new("config.toml")).expect("opening config.toml file failed");
        let mut toml_str = String::new();
        config_file
            .read_to_string(&mut toml_str)
            .expect("reading config.toml file failed");
        let config: Config = toml::from_str(&toml_str).expect("parsing config.toml file failed");
        config
    }
}

pub fn setup_logger() -> Result<(), fern::InitError> {
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{} {} {}] {}",
                humantime::format_rfc3339_seconds(SystemTime::now()),
                record.level(),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::Info)
        .chain(std::io::stdout())
        .chain(fern::log_file("output.log")?)
        .apply()?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    setup_logger().expect("failed to initialize logger");
    println!("-- VA108xx Flashloader Client --");
    let cli = Cli::parse();
    let serial_port = match cli.port {
        Some(port) => port,
        None => match Config::new_from_file().interface.serial_port {
            Some(port) => port,
            None => bail!("Serial port not specified in configuration file."),
        },
    };
    let serial = serialport::new(&serial_port, 115200)
        .open()
        .expect("opening serial port failed");
    let mut transport = PacketTransportSerialCobs::new(serial, CobsDecoderOwned::new(4096));

    match cli.command {
        Command::Ping => {
            let tc = tc::create_tc(&models::Request::Ping);
            log::info!(
                "Sending ping request with TC ID: {:#010x}",
                tc.ccsds_packet_id_and_psc().raw()
            );
            let tc_raw = tc.to_vec();
            transport.send(&tc_raw).unwrap();
        }
        Command::SetBootSlot { app } => {
            let app_sel = match app {
                AppTarget::A => models::AppSel::A,
                AppTarget::B => models::AppSel::B,
            };
            let tc = tc::create_tc(&models::Request::SetBootSlot(app_sel));
            log::info!(
                "Sending app select {:?} command with TC ID: {:#010x}",
                app_sel,
                tc.ccsds_packet_id_and_psc().raw()
            );
            transport.send(&tc.to_vec()).unwrap();
        }
        Command::Corrupt { app } => {
            let app_sel = match app {
                AppTarget::A => models::AppSel::A,
                AppTarget::B => models::AppSel::B,
            };
            let tc = tc::create_tc(&models::Request::Corrupt(app_sel));
            log::info!(
                "Sending corrupt slot {:?} command with TC ID: {:#010x}",
                app_sel,
                tc.ccsds_packet_id_and_psc().raw()
            );
            transport.send(&tc.to_vec()).unwrap();
        }
        Command::Flash { target, path } => {
            flash::flash_image(&mut transport, &target, &path)?;
        }
    }

    log::info!("Waiting for response...");
    loop {
        transport
            .receive(|packet: &[u8]| {
                let reader = CcsdsPacketReader::new_with_checksum(packet);
                log::debug!("Received packet: {:?}", reader);
                if let Ok(reader) = reader {
                    let packet_data = reader.packet_data();
                    let response_result = postcard::take_from_bytes::<Response>(packet_data);
                    if let Ok((response, _remainder)) = response_result {
                        log::info!("Received TM with response: {:?}", response);
                    } else {
                        log::error!(
                            "Failed to parse response from packet data: {:?}",
                            response_result
                        );
                    }
                }
            })
            .unwrap();
    }
}
