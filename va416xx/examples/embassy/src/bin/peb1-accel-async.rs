//! Async I2C example which reads the LIS2DH12 accelerometer on the PEB1 board.
//!
//! It can also run a set of I2C transaction sequences against the sensor before the
//! accelerometer loop starts. Enable them in the config block below.
#![no_std]
#![no_main]
use defmt_rtt as _;
use panic_probe as _;

use core::sync::atomic::{AtomicU32, Ordering};
use embassy_example::EXTCLK_FREQ;
use embassy_executor::Spawner;
use embassy_time::{Duration, Ticker};
use embedded_hal_async::i2c::Operation;
use lis2dh12::{asynch::Lis2dh12, FullScale, Mode, Odr};
use once_cell::sync::OnceCell;

use va416xx_hal::{
    clock::ClockConfigurator,
    i2c::{self, asynch::I2c},
    pac::{self, interrupt},
    time::Hertz,
};

//==================================================================================================
// Config
//==================================================================================================

/// `[W, R, W]`: a middle group, which ends in `waiting`.
const TEST_WRITE_READ_WRITE: bool = true;
/// `[R, W]`: a write group following a read group.
const TEST_READ_WRITE: bool = true;
/// `[W, W]`: the TX pre-fill across operations of one group.
const TEST_SPLIT_WRITE: bool = true;
/// `[W, R, R]`: several operations in the last group.
const TEST_SPLIT_READ: bool = true;
/// `[W, R(18)]`: a read longer than the FIFO, drained with `rx_ready`.
const TEST_LONG_READ: bool = true;
/// Write to an address nothing answers at, then check the bus still works.
const TEST_NACK_ADDR: bool = true;
/// Empty writes, which only address the target. Once to the sensor, once to an unused address.
const TEST_PROBE: bool = true;

/// Assumed to be unused on the PEB1 I2C bus.
const UNUSED_ADDR: u8 = 0x10;

// LIS2DH12 registers. The lis2dh12 crate keeps its register map private.
const LIS2DH12_ADDR: u8 = 0x18;
const WHO_AM_I: u8 = 0x0F;
const DEVICE_ID: u8 = 0x33;
const OUT_X_L: u8 = 0x28;
const FIFO_CTRL_REG: u8 = 0x2E;
/// Interrupt 1 threshold. Safe to write, because the interrupt is not used.
const INT1_THS: u8 = 0x32;
/// FIFO_SRC_REG, INT1_SRC, INT2_SRC and CLICK_SRC.
const SOURCE_REGS: [u8; 4] = [0x2F, 0x31, 0x35, 0x39];
/// FIFO_CTRL_REG (0x2E) up to ACT_DUR (0x3F).
const LONG_READ_LEN: usize = 18;
/// Sub-address bit which enables register address auto-increment.
const AUTO_INCREMENT: u8 = 0x80;

/// Token identifying the I2C0 peripheral, set once at construction and read by the interrupt
/// handler, which does not have access to the [I2c] driver itself.
static I2C_TOKEN: OnceCell<i2c::Bank> = OnceCell::new();

/// Interrupt count, so the sequence tests can show how many interrupts a transfer took.
static IRQ_COUNT: AtomicU32 = AtomicU32::new(0);

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    defmt::println!("-- VA416xx Async I2C Accelerometer Example --");

    let dp = pac::Peripherals::take().unwrap();

    // Use the external clock connected to XTAL_N.
    let clocks = ClockConfigurator::new(dp.clkgen)
        .xtal_n_clk_with_src_freq(Hertz::from_raw(EXTCLK_FREQ))
        .freeze()
        .unwrap();
    // Safety: Only called once here.
    va416xx_hal::embassy_time::init(dp.tim15, dp.tim14, &clocks);

    let i2c_master = i2c::I2cMaster::new(
        dp.i2c0,
        &clocks,
        i2c::MasterConfig::default(),
        i2c::I2cSpeed::Regular100khz,
    )
    .expect("creating I2C master failed");

    let bank = i2c_master.id();
    let mut i2c = i2c_master.into_async();

    I2C_TOKEN.set(bank).unwrap();

    // Detect the accelerometer's address by scanning all possible values.
    let slave_addr = lis2dh12::asynch::detect_i2c_addr(&mut i2c)
        .await
        .expect("detecting I2C address failed");
    match &slave_addr {
        lis2dh12::SlaveAddr::Default => defmt::info!("Accelerometer slave address: Default"),
        lis2dh12::SlaveAddr::Alternative(a0) => {
            defmt::info!("Accelerometer slave address: Alternative({})", a0)
        }
    }
    let addr = match &slave_addr {
        lis2dh12::SlaveAddr::Default => LIS2DH12_ADDR,
        lis2dh12::SlaveAddr::Alternative(a0) => LIS2DH12_ADDR | *a0 as u8,
    };
    run_sequence_tests(&mut i2c, addr).await;

    let mut accelerometer = Lis2dh12::new(i2c, slave_addr)
        .await
        .expect("creating accelerometer driver failed");
    let device_id = accelerometer
        .get_device_id()
        .await
        .expect("reading device ID failed");
    defmt::info!("Device ID: 0x{:02X}", device_id);
    accelerometer
        .set_mode(Mode::Normal)
        .await
        .expect("setting mode failed");
    accelerometer
        .set_odr(Odr::Hz100)
        .await
        .expect("setting ODR failed");
    accelerometer
        .set_fs(FullScale::G4)
        .await
        .expect("setting full scale failed");
    // This function also enables BDU.
    accelerometer
        .enable_temp(true)
        .await
        .expect("enabling temperature sensor failed");

    let mut ticker = Ticker::every(Duration::from_millis(500));
    loop {
        let temperature = accelerometer
            .get_temp_outf()
            .await
            .expect("reading temperature failed");
        let value = accelerometer
            .accel_norm()
            .await
            .expect("reading accelerometer data failed");
        defmt::info!(
            "Accel Norm F32x3 {{ x: {}, y: {}, z: {} }} | Temp {} °C",
            value.x,
            value.y,
            value.z,
            temperature
        );
        ticker.next().await;
    }
}

async fn read_reg(i2c: &mut I2c, addr: u8, reg: u8) -> u8 {
    let mut buf = [0];
    i2c.write_read(addr, &[reg], &mut buf)
        .await
        .expect("register read failed");
    buf[0]
}

async fn run_sequence_tests(i2c: &mut I2c, addr: u8) {
    if TEST_WRITE_READ_WRITE {
        let mut id = [0];
        take_irqs();
        i2c.transaction(
            addr,
            &mut [
                Operation::Write(&[WHO_AM_I]),
                Operation::Read(&mut id),
                Operation::Write(&[INT1_THS, 0x11]),
            ],
        )
        .expect("starting [W, R, W] failed")
        .await
        .expect("[W, R, W] failed");
        let irqs = take_irqs();
        assert_eq!(id[0], DEVICE_ID);
        assert_eq!(read_reg(i2c, addr, INT1_THS).await, 0x11);
        defmt::info!("[W, R, W] ok ({} irqs)", irqs);
    }
    if TEST_READ_WRITE {
        // Reads whatever register the address pointer is at. Only the write is checked.
        let mut any = [0];
        take_irqs();
        i2c.transaction(
            addr,
            &mut [
                Operation::Read(&mut any),
                Operation::Write(&[INT1_THS, 0x22]),
            ],
        )
        .expect("starting [R, W] failed")
        .await
        .expect("[R, W] failed");
        let irqs = take_irqs();
        assert_eq!(read_reg(i2c, addr, INT1_THS).await, 0x22);
        defmt::info!("[R, W] ok ({} irqs)", irqs);
    }
    if TEST_SPLIT_WRITE {
        take_irqs();
        i2c.transaction(
            addr,
            &mut [Operation::Write(&[INT1_THS]), Operation::Write(&[0x33])],
        )
        .expect("starting [W, W] failed")
        .await
        .expect("[W, W] failed");
        let irqs = take_irqs();
        assert_eq!(read_reg(i2c, addr, INT1_THS).await, 0x33);
        defmt::info!("[W, W] ok ({} irqs)", irqs);
    }
    if TEST_SPLIT_READ {
        // The output registers change all the time, so only the transfer itself is checked.
        let mut first = [0; 3];
        let mut second = [0; 3];
        take_irqs();
        i2c.transaction(
            addr,
            &mut [
                Operation::Write(&[OUT_X_L | AUTO_INCREMENT]),
                Operation::Read(&mut first),
                Operation::Read(&mut second),
            ],
        )
        .expect("starting [W, R, R] failed")
        .await
        .expect("[W, R, R] failed");
        defmt::info!("[W, R, R] ok ({} irqs)", take_irqs());
    }
    if TEST_LONG_READ {
        // FIFO_CTRL_REG (0x2E) up to ACT_DUR (0x3F). The range starts after the output
        // registers, because auto-increment wraps from OUT_Z_H back to OUT_X_L.
        let mut regs = [0; LONG_READ_LEN];
        take_irqs();
        i2c.write_read(addr, &[FIFO_CTRL_REG | AUTO_INCREMENT], &mut regs)
            .await
            .expect("long read failed");
        let irqs = take_irqs();
        let mut single = [0; LONG_READ_LEN];
        for (i, value) in single.iter_mut().enumerate() {
            *value = read_reg(i2c, addr, FIFO_CTRL_REG + i as u8).await;
        }
        // Source registers report events and can change or clear on read.
        let mismatch = (0..LONG_READ_LEN)
            .find(|&i| !SOURCE_REGS.contains(&(FIFO_CTRL_REG + i as u8)) && regs[i] != single[i]);
        if let Some(i) = mismatch {
            defmt::error!("long read:   {=[u8]:#04x}", regs);
            defmt::error!("single read: {=[u8]:#04x}", single);
            defmt::panic!("register {:#04x} differs", FIFO_CTRL_REG + i as u8);
        }
        defmt::info!("[W, R({})] ok ({} irqs)", LONG_READ_LEN, irqs);
    }
    if TEST_NACK_ADDR {
        take_irqs();
        assert_eq!(
            i2c.write(UNUSED_ADDR, &[0]).await,
            Err(i2c::Error::NackAddr)
        );
        let irqs = take_irqs();
        assert_eq!(read_reg(i2c, addr, WHO_AM_I).await, DEVICE_ID);
        defmt::info!("NACK on address ok ({} irqs)", irqs);
    }
    if TEST_PROBE {
        take_irqs();
        i2c.write(addr, &[]).await.expect("probing sensor failed");
        assert_eq!(i2c.write(UNUSED_ADDR, &[]).await, Err(i2c::Error::NackAddr));
        let irqs = take_irqs();
        assert_eq!(read_reg(i2c, addr, WHO_AM_I).await, DEVICE_ID);
        defmt::info!("probe ok ({} irqs)", irqs);
    }
}

/// Interrupts since the last call.
fn take_irqs() -> u32 {
    IRQ_COUNT.swap(0, Ordering::Relaxed)
}

#[interrupt]
#[allow(non_snake_case)]
fn I2C0_MS() {
    IRQ_COUNT.fetch_add(1, Ordering::Relaxed);
    // Safety: Only called once from the I2C0 master interrupt handler.
    unsafe { I2c::on_interrupt(*I2C_TOKEN.get().unwrap()) };
}
