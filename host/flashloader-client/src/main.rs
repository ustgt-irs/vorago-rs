use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context as _, bail};
use chip::{Chip, Request};
use clap::Parser as _;
use cobs::CobsDecoderOwned;
use flashloader_types::{AppSel, Response};
use spacepackets::CcsdsPacketReader;
use tmtc_utils::transport::serial::PacketTransportSerialCobs;

pub mod chip;
pub mod elf;
pub mod flash;
pub mod tc;

const BAUD_RATE: u32 = 115200;
const ACK_TIMEOUT: Duration = Duration::from_secs(2);
const CONFIG_FILE: &str = "loader.toml";

#[derive(clap::Parser)]
#[command(name = "vorago-image-loader", about = "Vorago flashloader client")]
pub struct Cli {
    /// Chip of the flashloader (overrides loader.toml)
    #[arg(short, long)]
    pub chip: Option<Chip>,

    /// Serial port to use (overrides loader.toml)
    #[arg(short, long)]
    pub port: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub enum Command {
    /// Send a ping command
    Ping,

    /// Set the boot slot (VA108xx only)
    SetBootSlot {
        /// Slot to boot from
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

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
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

impl From<AppTarget> for AppSel {
    fn from(app: AppTarget) -> Self {
        match app {
            AppTarget::A => AppSel::A,
            AppTarget::B => AppSel::B,
        }
    }
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Config {
    pub chip: Option<Chip>,
    pub serial_port: Option<String>,
}

impl Config {
    /// The config file is optional because all settings can also be passed on the command line.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let toml_str = std::fs::read_to_string(path)
            .with_context(|| format!("reading {} failed", path.display()))?;
        toml::from_str(&toml_str).with_context(|| format!("parsing {} failed", path.display()))
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

/// Sends a request and waits until the flashloader acknowledges it.
///
/// The flashloader only replies after it has handled a request, so this also paces the NVM
/// writes. It does not send a reply for requests it can not parse, which shows up as a timeout.
pub fn send_and_await_ok(
    transport: &mut PacketTransportSerialCobs,
    chip: Chip,
    request: Request,
    payload: &[u8],
) -> anyhow::Result<()> {
    let tc = tc::create_tc(chip, request, payload)?;
    log::debug!(
        "TX TC {:#010x}: {request:?} + {} payload bytes",
        tc.ccsds_packet_id_and_psc().raw(),
        payload.len()
    );
    transport.send(&tc.to_vec())?;

    let deadline = Instant::now() + ACK_TIMEOUT;
    let mut got_ok = false;
    while !got_ok {
        if Instant::now() > deadline {
            bail!("timeout waiting for Ok response to {request:?}");
        }
        transport.receive(|packet| {
            let reader = match CcsdsPacketReader::new_with_checksum(packet) {
                Ok(reader) => reader,
                Err(e) => {
                    log::error!("invalid TM packet: {e}");
                    return;
                }
            };
            match postcard::take_from_bytes::<Response>(reader.user_data()) {
                Ok((Response::Ok, _)) => got_ok = true,
                Err(e) => log::error!("failed to deserialize response: {e}"),
            }
        })?;
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    setup_logger().expect("failed to initialize logger");
    let cli = Cli::parse();
    let config = Config::load(Path::new(CONFIG_FILE))?;

    let Some(chip) = cli.chip.or(config.chip) else {
        bail!("no chip specified, use --chip or set chip in {CONFIG_FILE}");
    };
    println!("-- Vorago Flashloader Client ({chip:?}) --");
    let Some(serial_port) = cli.port.or(config.serial_port) else {
        bail!("no serial port specified, use --port or set serial_port in {CONFIG_FILE}");
    };
    let serial = serialport::new(&serial_port, BAUD_RATE)
        // Lets the receive loop block briefly instead of spinning.
        .timeout(Duration::from_millis(100))
        .open()
        .with_context(|| format!("opening serial port {serial_port} failed"))?;
    let mut transport = PacketTransportSerialCobs::new(serial, CobsDecoderOwned::new(4096));

    match cli.command {
        Command::Ping => {
            log::info!("Sending ping request");
            send_and_await_ok(&mut transport, chip, Request::Ping, &[])?;
            log::info!("Ping reply received");
        }
        Command::SetBootSlot { app } => {
            let app_sel = AppSel::from(app);
            log::info!("Sending set boot slot {app_sel:?} request");
            send_and_await_ok(&mut transport, chip, Request::SetBootSlot(app_sel), &[])?;
            log::info!("Boot slot set to {app_sel:?}");
        }
        Command::Corrupt { app } => {
            let app_sel = AppSel::from(app);
            log::info!("Sending corrupt slot {app_sel:?} request");
            send_and_await_ok(&mut transport, chip, Request::Corrupt(app_sel), &[])?;
            log::info!("Slot {app_sel:?} corrupted");
        }
        Command::Flash { target, path } => {
            flash::flash_image(&mut transport, chip, target, &path)?;
        }
    }
    Ok(())
}
