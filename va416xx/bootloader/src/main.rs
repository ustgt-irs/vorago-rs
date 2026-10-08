//! Vorago bootloader which can boot from two images.
//!
//! As opposed to the Vorago example code, this bootloader assumes a 40 MHz external clock
//! but does not scale that clock up.
#![no_main]
#![no_std]

use cortex_m_rt::entry;
use crc::{Crc, CRC_32_ISO_HDLC};
use defmt_rtt as _;
use panic_probe as _;
use va416xx_hal::{
    clock::{pll_setup_delay, ClockConfigurator, ClockDivisorSelect, ClockSelect},
    edac,
    nvm::Nvm,
    pac::{self, interrupt},
    time::Hertz,
    wdt::Wdt,
};

const EXTCLK_FREQ: u32 = 40_000_000;
const WITH_WDT: bool = false;
const WDT_FREQ_MS: u32 = 50;
const DEBUG_PRINTOUTS: bool = true;

// Dangerous option! An image with this option set to true will flash itself from RAM directly
// into the NVM. This can be used as a recovery option from a direct RAM flash to fix the NVM
// boot process. Please note that this will flash an image which will also always perform the
// self-flash itself. It is recommended that you use a tool like probe-rs, Keil IDE, or a flash
// loader to boot a bootloader without this feature.
const FLASH_SELF: bool = false;
// Useful for debugging and see what the bootloader is doing. Enabled currently, because
// the binary stays small enough.
const DEFMT_PRINTOUTS: bool = true;

// Important bootloader addresses and offsets, vector table information.

const NVM_SIZE: u32 = 0x40000;

const BOOTLOADER_START_ADDR: u32 = 0x0;
const BOOTLOADER_CRC_ADDR: u32 = BOOTLOADER_END_ADDR - 4;
const BOOTLOADER_END_ADDR: u32 = 0x4000;

// 0x4000
const APP_A_START_ADDR: u32 = BOOTLOADER_END_ADDR;
// The actual size of the image which is relevant for CRC calculation will be store at this
// address.
// 0x21FF8
const APP_A_SIZE_ADDR: u32 = APP_A_END_ADDR - 8;
// 0x21FFC
const APP_A_CRC_ADDR: u32 = APP_A_END_ADDR - 4;
pub const APP_A_END_ADDR: u32 = BOOTLOADER_END_ADDR + APP_IMG_SZ;

//  0x22000
const APP_B_START_ADDR: u32 = APP_A_END_ADDR;
// The actual size of the image which is relevant for CRC calculation will be stored at this
// address.
// 0x3FFF8
const APP_B_SIZE_ADDR: u32 = APP_B_END_ADDR - 8;
// 0x3FFFC
const APP_B_CRC_ADDR: u32 = APP_B_END_ADDR - 4;
// 0x40000
pub const APP_B_END_ADDR: u32 = NVM_SIZE;

pub const APP_IMG_SZ: u32 = (APP_B_END_ADDR - APP_A_START_ADDR) / 2;

static_assertions::const_assert!((APP_B_END_ADDR - BOOTLOADER_END_ADDR).is_multiple_of(2));
static_assertions::const_assert_eq!(APP_A_END_ADDR, 0x22000);
static_assertions::const_assert_eq!(APP_A_CRC_ADDR, 0x21FFC);

pub const VECTOR_TABLE_OFFSET: u32 = 0x0;
pub const VECTOR_TABLE_LEN: u32 = 0x350;
pub const RESET_VECTOR_OFFSET: u32 = 0x4;

const CRC_ALGO: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);

#[derive(Debug, Copy, Clone, PartialEq, Eq, defmt::Format)]
enum AppSel {
    A,
    B,
}

pub trait WdtInterface {
    fn feed(&self);
}

pub struct OptWdt(Option<Wdt>);

impl WdtInterface for OptWdt {
    fn feed(&self) {
        if let Some(wdt) = &self.0 {
            wdt.feed();
        }
    }
}

#[entry]
fn main() -> ! {
    if DEFMT_PRINTOUTS {
        defmt::println!("-- VA416xx bootloader --");
    }
    let mut dp = pac::Peripherals::take().unwrap();
    let cp = cortex_m::Peripherals::take().unwrap();
    // Disable ROM protection.
    dp.sysconfig.rom_prot().write(|w| unsafe { w.bits(1) });
    setup_edac(&mut dp.sysconfig);
    // Use the external clock connected to XTAL_N.
    let clocks = ClockConfigurator::new(dp.clkgen)
        .xtal_n_clk_with_src_freq(Hertz::from_raw(EXTCLK_FREQ))
        .freeze()
        .unwrap();
    let mut opt_wdt = OptWdt(None);
    if WITH_WDT {
        opt_wdt.0 = Some(Wdt::start(dp.watch_dog, &clocks, WDT_FREQ_MS));
    }

    let nvm = Nvm::new(dp.spi3, &clocks);

    if FLASH_SELF {
        #[allow(clippy::zero_ptr)]
        let first_four_bytes = unsafe { core::ptr::read_volatile(0x0 as *const u32) }.to_ne_bytes();
        let bootloader_data = {
            unsafe {
                &*core::ptr::slice_from_raw_parts(
                    (BOOTLOADER_START_ADDR + 4) as *const u8,
                    (BOOTLOADER_END_ADDR - BOOTLOADER_START_ADDR - 8) as usize,
                )
            }
        };
        let mut digest = CRC_ALGO.digest();
        digest.update(&first_four_bytes);
        digest.update(bootloader_data);
        let bootloader_crc = digest.finalize();

        nvm.write_data(0x0, &first_four_bytes);
        nvm.write_data(0x4, bootloader_data);
        if let Err(e) = nvm.verify_data(0x0, &first_four_bytes) {
            if DEFMT_PRINTOUTS {
                defmt::error!("verification of self-flash to NVM failed: {:?}", e);
            }
        }
        if let Err(e) = nvm.verify_data(0x4, bootloader_data) {
            if DEFMT_PRINTOUTS {
                defmt::error!("verification of self-flash to NVM failed: {:?}", e);
            }
        }

        nvm.write_data(BOOTLOADER_CRC_ADDR, &bootloader_crc.to_be_bytes());
        if let Err(e) = nvm.verify_data(BOOTLOADER_CRC_ADDR, &bootloader_crc.to_be_bytes()) {
            if DEFMT_PRINTOUTS {
                defmt::error!(
                    "error: CRC verification for bootloader self-flash failed: {:?}",
                    e
                );
            }
        }
    }

    // Check bootloader's CRC (and write it if blank)
    check_own_crc(&opt_wdt, &nvm, &cp);

    if check_app_crc(AppSel::A, &opt_wdt) {
        boot_app(AppSel::A, &cp)
    } else if check_app_crc(AppSel::B, &opt_wdt) {
        boot_app(AppSel::B, &cp)
    } else {
        if DEBUG_PRINTOUTS && DEFMT_PRINTOUTS {
            defmt::println!("both images corrupt! booting image A");
        }
        // TODO: Shift a CCSDS packet out to inform host/OBC about image corruption.
        // Both images seem to be corrupt. Boot default image A.
        boot_app(AppSel::A, &cp)
    }
}

fn check_own_crc(wdt: &OptWdt, nvm: &Nvm, cp: &cortex_m::Peripherals) {
    let crc_exp = unsafe { (BOOTLOADER_CRC_ADDR as *const u32).read_unaligned().to_be() };
    wdt.feed();
    // I'd prefer to use [core::slice::from_raw_parts], but that is problematic
    // because the address of the bootloader is 0x0, so the NULL check fails and the functions
    // panics.
    #[allow(clippy::zero_ptr)]
    let first_four_bytes = unsafe { core::ptr::read_volatile(0x0 as *const u32) }.to_ne_bytes();
    let mut digest = CRC_ALGO.digest();
    digest.update(&first_four_bytes);
    digest.update(unsafe {
        &*core::ptr::slice_from_raw_parts(
            (BOOTLOADER_START_ADDR + 4) as *const u8,
            (BOOTLOADER_END_ADDR - BOOTLOADER_START_ADDR - 8) as usize,
        )
    });
    let crc_calc = digest.finalize();
    wdt.feed();
    if crc_exp == 0x0000 || crc_exp == 0xffff {
        if DEBUG_PRINTOUTS && DEFMT_PRINTOUTS {
            defmt::info!("BL CRC blank - prog new CRC");
        }
        // Blank CRC, write it to NVM.
        nvm.write_data(BOOTLOADER_CRC_ADDR, &crc_calc.to_be_bytes());
        // The Vorago bootloader resets here. I am not sure why this is done but I think it is
        // necessary because somehow the boot will not work if we just continue as usual.
        // cortex_m::peripheral::SCB::sys_reset();
    } else if crc_exp != crc_calc {
        // Bootloader is corrupted. Try to run App A.
        if DEBUG_PRINTOUTS && DEFMT_PRINTOUTS {
            defmt::info!(
                "bootloader CRC corrupt, read {} and expected {}. booting image A immediately",
                crc_calc,
                crc_exp
            );
        }
        // TODO: Shift out minimal CCSDS frame to notify about bootloader corruption.
        boot_app(AppSel::A, cp);
    }
}

fn check_app_crc(app_sel: AppSel, wdt: &OptWdt) -> bool {
    if DEBUG_PRINTOUTS && DEFMT_PRINTOUTS {
        defmt::info!("Checking image {:?}", app_sel);
    }
    if app_sel == AppSel::A {
        check_app_given_addr(APP_A_CRC_ADDR, APP_A_START_ADDR, APP_A_SIZE_ADDR, wdt)
    } else {
        check_app_given_addr(APP_B_CRC_ADDR, APP_B_START_ADDR, APP_B_SIZE_ADDR, wdt)
    }
}

fn check_app_given_addr(
    crc_addr: u32,
    start_addr: u32,
    image_size_addr: u32,
    wdt: &OptWdt,
) -> bool {
    let crc_exp = unsafe { (crc_addr as *const u32).read_unaligned().to_be() };
    let image_size = unsafe { (image_size_addr as *const u32).read_unaligned().to_be() };
    // Sanity check.
    if image_size > APP_A_END_ADDR - APP_A_START_ADDR - 8 {
        if DEFMT_PRINTOUTS {
            defmt::info!("detected invalid app size {}", image_size);
        }
        return false;
    }
    wdt.feed();
    let crc_calc = CRC_ALGO.checksum(unsafe {
        core::slice::from_raw_parts(start_addr as *const u8, image_size as usize)
    });
    wdt.feed();
    if crc_calc == crc_exp {
        return true;
    }
    false
}

fn boot_app(app_sel: AppSel, cp: &cortex_m::Peripherals) -> ! {
    if DEBUG_PRINTOUTS && DEFMT_PRINTOUTS {
        defmt::info!("booting app {:?}", app_sel);
    }
    let clkgen = unsafe { pac::Clkgen::steal() };
    clkgen
        .ctrl0()
        .modify(|_, w| unsafe { w.clksel_sys().bits(ClockSelect::Hbo as u8) });
    pll_setup_delay();
    clkgen
        .ctrl0()
        .modify(|_, w| unsafe { w.clk_div_sel().bits(ClockDivisorSelect::Div1 as u8) });
    // Clear all interrupts set.
    unsafe {
        cp.NVIC.icer[0].write(0xFFFFFFFF);
        cp.NVIC.icpr[0].write(0xFFFFFFFF);
    }
    let app_start = match app_sel {
        AppSel::A => APP_A_START_ADDR,
        AppSel::B => APP_B_START_ADDR,
    };
    // `bootload` only sets the stack pointer and jumps. Without this, the app could use
    // the interrupt handlers of the bootloader.
    //
    // SAFETY: We care about the side-effect of this write.
    unsafe {
        cp.SCB.vtor.write(app_start);
    }
    cortex_m::asm::dsb();
    cortex_m::asm::isb();
    // SAFETY: The image passed the CRC check, or it is the fallback image A.
    unsafe {
        cortex_m::asm::bootload(app_start as *const u32);
    }
}

fn setup_edac(syscfg: &mut pac::Sysconfig) {
    // The scrub values are based on the Vorago provided bootloader.
    edac::enable_rom_scrub(syscfg, 125);
    edac::enable_ram0_scrub(syscfg, 1000);
    edac::enable_ram1_scrub(syscfg, 1000);
    edac::enable_sbe_irq();
    edac::enable_mbe_irq();
}

#[interrupt]
#[allow(non_snake_case)]
fn WATCHDOG() {
    let wdt = unsafe { pac::WatchDog::steal() };
    // Clear interrupt.
    wdt.wdogintclr().write(|w| unsafe { w.bits(1) });
}

#[interrupt]
#[allow(non_snake_case)]
fn EDAC_SBE() {
    // TODO: Send some command via UART for notification purposes. Also identify the problematic
    // memory.
    edac::clear_sbe_irq();
}

#[interrupt]
#[allow(non_snake_case)]
fn EDAC_MBE() {
    // TODO: Send some command via UART for notification purposes.
    edac::clear_mbe_irq();
    // TODO: Reset like the vorago example?
}
