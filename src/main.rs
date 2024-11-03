//! Sends "Hello, world!" through the ITM port 0
//!
//! ITM is much faster than semihosting. Like 4 orders of magnitude or so.
//!
//! **NOTE** Cortex-M0 chips don't support ITM.
//!
//! You'll have to connect the microcontroller's SWO pin to the SWD interface. Note that some
//! development boards don't provide this option.
//!
//! You'll need [`itmdump`] to receive the message on the host plus you'll need to uncomment two
//! `monitor` commands in the `.gdbinit` file.
//!
//! [`itmdump`]: https://docs.rs/itm/0.2.1/itm/
//!
//! ---

#![no_main]
#![no_std]

use cortex_m::iprintln;
use cortex_m_rt::entry;
use nb::block;
use panic_halt as _;
use stm32f1xx_hal::{
    pac::{self},
    prelude::*,
    timer::Timer,
};

const SWO_BAUDRATE: u32 = 1 * 1000 * 1000;
const ARM_LAR_ACCESS_ENABLE: u32 = 0xc5acce55;
const SCS_DEMCR: *mut u32 = 0xE000EDFC as *mut u32;
const SCS_DEMCR_TRCENA: u32 = 1 << 24;
const TPIU_LAR: *mut u32 = 0xE0040FB0 as *mut u32;
const TPIU_CSPSR: *mut u32 = 0xE0040004 as *mut u32;
const TPIU_ACPR: *mut u32 = 0xE0040010 as *mut u32;
const TPIU_SPPR: *mut u32 = 0xE00400F0 as *mut u32;
const TPIU_FFCR: *mut u32 = 0xE0040304 as *mut u32;
const DWT_LAR: *mut u32 = 0xE0001FB0 as *mut u32;
const DWT_CTRL: *mut u32 = 0xE0001000 as *mut u32;
const ITM_LAR: *mut u32 = 0xE0000FB0 as *mut u32;
const ITM_TPR: *mut u32 = 0xE0000E40 as *mut u32;
const ITM_TCR: *mut u32 = 0xE0000E80 as *mut u32;
const ITM_TCR_SWOENA: u32 = 1 << 4;
const ITM_TCR_TXENA: u32 = 1 << 3;
const ITM_TCR_SYNCENA: u32 = 1 << 2;
// const ITM_TCR_TSENA: u32 = 1 << 1;
const ITM_TCR_ITMENA: u32 = 1 << 0;
const ITM_TER: *mut u32 = 0xE0000E00 as *mut u32;
const DBGMCU_CR: *mut u32 = 0xe0042004 as *mut u32;
const DBGMCU_CR_TRACE_IOEN: u32 = 0x00000020;
const DBGMCU_CR_TRACE_MODE_MASK: u32 = 0x000000C0;
const DBGMCU_CR_TRACE_MODE_ASYNC: u32 = 0x00000000;
const TPIU_FFCR_ENFCONT: u32 = 1 << 1;
const TPIU_SPPR_ASYNC_MANCHESTER: u32 = 1;
// const TPIU_SPPR_ASYNC_NRZ: u32 = 2;

fn swo_setup(clocks: &stm32f1xx_hal::rcc::Clocks) {
    unsafe {
        /* Enable tracing in DEMCR */
        SCS_DEMCR.write_volatile(SCS_DEMCR.read_volatile() | SCS_DEMCR_TRCENA);

        /* Get the active clock frequency of the core and use that to calculate a divisor */
        let clock_frequency = clocks.hclk().to_Hz();
        let divisor = (clock_frequency / SWO_BAUDRATE) - 1;
        /* And configure the TPIU for 1-bit async trace (SWO) in Manchester coding */
        TPIU_LAR.write_volatile(ARM_LAR_ACCESS_ENABLE);
        TPIU_CSPSR.write_volatile(1 /* 1-bit mode */);
        TPIU_ACPR.write_volatile(divisor);
        TPIU_SPPR.write_volatile(TPIU_SPPR_ASYNC_MANCHESTER);
        /* Ensure that TPIU framing is off */
        TPIU_FFCR.write_volatile(TPIU_FFCR.read_volatile() & !TPIU_FFCR_ENFCONT);

        /* Configure the DWT to provide the sync source for the ITM */
        DWT_LAR.write_volatile(ARM_LAR_ACCESS_ENABLE);
        DWT_CTRL.write_volatile(DWT_CTRL.read_volatile() | 0x000003fe);
        /* Enable access to the ITM registers and configure tracing output from the first stimulus port */
        ITM_LAR.write_volatile(ARM_LAR_ACCESS_ENABLE);
        /* User-level access to the first 8 ports */
        ITM_TPR.write_volatile(0x0000000f);
        ITM_TCR.write_volatile(
            ITM_TCR_ITMENA | ITM_TCR_SYNCENA | ITM_TCR_TXENA | ITM_TCR_SWOENA | (1 << 16),
        );
        ITM_TER.write_volatile(1);

        /* Now tell the DBGMCU that we want trace enabled and mapped as SWO */
        DBGMCU_CR.write_volatile(DBGMCU_CR.read_volatile() & !DBGMCU_CR_TRACE_MODE_MASK);
        DBGMCU_CR.write_volatile(
            DBGMCU_CR.read_volatile() | DBGMCU_CR_TRACE_IOEN | DBGMCU_CR_TRACE_MODE_ASYNC,
        );
    }
}

#[entry]
fn main() -> ! {
    // let mut p = Peripherals::take().unwrap();
    // let stim = &mut p.ITM.stim[0];

    // loop {
    //     iprintln!(stim, "Hello, world!");
    // }
    // Get access to the core peripherals from the cortex-m crate
    let mut cp = cortex_m::Peripherals::take().unwrap();
    // Get access to the device specific peripherals from the peripheral access crate
    let mut dp = pac::Peripherals::take().unwrap();


    // Take ownership over the raw flash and rcc devices and convert them into the corresponding
    // HAL structs
    let mut flash = dp.FLASH.constrain();
    let rcc = dp.RCC.constrain();

    // Freeze the configuration of all the clocks in the system and store the frozen frequencies in
    // `clocks`
    let clocks = rcc.cfgr.freeze(&mut flash.acr);

    swo_setup(&clocks);

    // Acquire the GPIOC peripheral
    let mut gpioa = dp.GPIOA.split();
    let stim = &mut cp.ITM.stim[0];

    // Configure gpio A pin 3 as a push-pull output. The `crl` register is passed to the function
    // in order to configure the port. For pins 8-15, crh should be passed instead.
    let mut led = gpioa.pa3.into_push_pull_output(&mut gpioa.crl);
    // Configure the syst timer to trigger an update every second
    let mut timer = Timer::syst(cp.SYST, &clocks).counter_hz();
    timer.start(1.Hz()).unwrap();

    // Wait for the timer to trigger an update and change the state of the LED
    let mut b = 0x80u8;
    loop {
        // led.set_high();
        cortex_m::itm::write_all(stim, &[b, b, b, b]);
        cortex_m::itm::write_all(stim, &[b, b, b, b]);
        b = b.wrapping_add(1);
        // block!(timer.wait()).unwrap();

        // // led.set_low();
        // cortex_m::itm::write_all(stim, &[0x55]);
        // // block!(timer.wait()).unwrap();

        // led.set_low();
        // cortex_m::itm::write_all(stim, &[0xff]);
        // // block!(timer.wait()).unwrap();

        // led.set_low();
        // cortex_m::itm::write_all(stim, &[0xaa]);
        // // block!(timer.wait()).unwrap();

        // led.set_low();
        // cortex_m::itm::write_all(stim, &[1, 2, 3, 254]);
        // // block!(timer.wait()).unwrap();
    }
}
