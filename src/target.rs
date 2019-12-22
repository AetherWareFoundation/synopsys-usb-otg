#![allow(dead_code)]
#![allow(unused_imports)]
//! Target-specific definitions

use vcell::VolatileCell;

#[cfg(feature = "cortex-m")]
pub use cortex_m::interrupt;

// Export HAL
pub use stm32f4xx_hal as hal;


// USB PAC reexports
#[cfg(feature = "fs")]
pub use hal::stm32::OTG_FS_GLOBAL as OTG_GLOBAL;
#[cfg(feature = "fs")]
pub use hal::stm32::OTG_FS_DEVICE as OTG_DEVICE;
#[cfg(feature = "fs")]
pub use hal::stm32::OTG_FS_PWRCLK as OTG_PWRCLK;

#[cfg(feature = "hs")]
pub use hal::stm32::OTG_HS_GLOBAL as OTG_GLOBAL;
#[cfg(feature = "hs")]
pub use hal::stm32::OTG_HS_DEVICE as OTG_DEVICE;
#[cfg(feature = "hs")]
pub use hal::stm32::OTG_HS_PWRCLK as OTG_PWRCLK;

use crate::ral::{otg_global, otg_device, otg_pwrclk, otg_fifo};

pub fn fifo_write(channel: impl Into<usize>, mut buf: &[u8]) {
    let fifo = otg_fifo::instance(channel.into());

    while buf.len() >= 4 {
        let mut u32_bytes = [0u8; 4];
        u32_bytes.copy_from_slice(&buf[..4]);
        buf = &buf[4..];
        fifo.write(u32::from_ne_bytes(u32_bytes));
    }
    if buf.len() > 0 {
        let mut u32_bytes = [0u8; 4];
        u32_bytes[..buf.len()].copy_from_slice(buf);
        fifo.write(u32::from_ne_bytes(u32_bytes));
    }
}

pub fn fifo_read(mut buf: &mut [u8]) {
    let fifo = otg_fifo::instance(0);

    while buf.len() >= 4 {
        let word = fifo.read();
        let bytes = word.to_ne_bytes();
        buf[..4].copy_from_slice(&bytes);
        buf = &mut buf[4..];
    }
    if buf.len() > 0 {
        let word = fifo.read();
        let bytes = word.to_ne_bytes();
        buf.copy_from_slice(&bytes[..buf.len()]);
    }
}

pub fn fifo_read_into(buf: &[VolatileCell<u32>]) {
    let fifo = otg_fifo::instance(0);

    for p in buf {
        let word = fifo.read();
        p.set(word);
    }
}

/// Enables USB peripheral
pub fn apb_usb_enable() {
    cortex_m::interrupt::free(|_| {
        let rcc = unsafe { (&*hal::stm32::RCC::ptr()) };
        #[cfg(feature = "fs")]
        rcc.ahb2enr.modify(|_, w| w.otgfsen().set_bit());
        #[cfg(feature = "hs")]
        rcc.ahb1enr.modify(|_, w| w.otghsen().set_bit());
    });
}


/// Wrapper around device-specific peripheral that provides unified register interface
pub struct UsbRegisters {
    pub global: otg_global::Instance,
    pub device: otg_device::Instance,
    pub pwrclk: otg_pwrclk::Instance,
}

unsafe impl Send for UsbRegisters {}

impl UsbRegisters {
    pub fn new(_global: OTG_GLOBAL, _device: OTG_DEVICE, _pwrclk: OTG_PWRCLK) -> Self {
        Self {
            global: unsafe { otg_global::OTG_GLOBAL::steal() },
            device: unsafe { otg_device::OTG_DEVICE::steal() },
            pwrclk: unsafe { otg_pwrclk::OTG_PWRCLK::steal() },
        }
    }
}



pub trait UsbPins: Send { }


pub mod usb_pins {
    #[cfg(feature = "fs")]
    use super::hal::gpio::{AF10, Alternate};
    #[cfg(feature = "hs")]
    use super::hal::gpio::{AF12, Alternate};

    #[cfg(feature = "fs")]
    use super::hal::gpio::gpioa::{PA11, PA12};
    #[cfg(feature = "hs")]
    use super::hal::gpio::gpiob::{PB13, PB14, PB15};

    #[cfg(feature = "fs")]
    pub type UsbPinsType = (
        PA11<Alternate<AF10>>,
        PA12<Alternate<AF10>>
    );
    #[cfg(feature = "hs")]
    pub type UsbPinsType = (
        PB13<Alternate<AF12>>, // VBUS pin
        PB14<Alternate<AF12>>, // DM pin
        PB15<Alternate<AF12>>  // DP pin
    );

    impl super::UsbPins for UsbPinsType {}
}
