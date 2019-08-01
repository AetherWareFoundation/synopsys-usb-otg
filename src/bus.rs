use core::marker::PhantomData;
use usb_device::{Result, UsbDirection, UsbError};
use usb_device::bus::{UsbBusAllocator, PollResult};
use usb_device::endpoint::{EndpointType, EndpointAddress};
use cortex_m::interrupt::{self, Mutex, CriticalSection};
use stm32ral::{read_reg, write_reg, modify_reg, otg_fs_global, otg_fs_device, otg_fs_pwrclk};

use crate::target::{OTG_FS_GLOBAL, OTG_FS_DEVICE, OTG_FS_PWRCLK, apb_usb_enable, UsbRegisters, UsbPins};
use crate::endpoint::{EndpointIn, EndpointOut, Endpoint};
use crate::endpoint_memory::EndpointMemoryAllocator;
use core::ops::Deref;

const RX_FIFO_SIZE: u32 = 32;


/// USB peripheral driver for STM32 microcontrollers.
pub struct UsbBus<PINS> {
    regs: Mutex<UsbRegisters>,
    endpoints_in: [EndpointIn; 4],
    endpoints_out: [EndpointOut; 4],
    endpoint_allocator: EndpointMemoryAllocator,
    pins: PhantomData<PINS>,
}

impl<PINS: Send+Sync> UsbBus<PINS> {
    /// Constructs a new USB peripheral driver.
    pub fn new(regs: (OTG_FS_GLOBAL, OTG_FS_DEVICE, OTG_FS_PWRCLK), _pins: PINS, ep_memory: &'static mut [u32]) -> UsbBusAllocator<Self>
        where PINS: UsbPins
    {
        let endpoints_in = [
            EndpointIn::new(EndpointAddress::from_parts(0, UsbDirection::In)),
            EndpointIn::new(EndpointAddress::from_parts(1, UsbDirection::In)),
            EndpointIn::new(EndpointAddress::from_parts(2, UsbDirection::In)),
            EndpointIn::new(EndpointAddress::from_parts(3, UsbDirection::In)),
        ];
        let endpoints_out = [
            EndpointOut::new(EndpointAddress::from_parts(0, UsbDirection::Out)),
            EndpointOut::new(EndpointAddress::from_parts(1, UsbDirection::Out)),
            EndpointOut::new(EndpointAddress::from_parts(2, UsbDirection::Out)),
            EndpointOut::new(EndpointAddress::from_parts(3, UsbDirection::Out)),
        ];
        let bus = UsbBus {
            regs: Mutex::new(UsbRegisters::new(regs.0, regs.1, regs.2)),
            endpoint_allocator: EndpointMemoryAllocator::new(ep_memory),
            endpoints_in,
            endpoints_out,
            pins: PhantomData,
        };

        UsbBusAllocator::new(bus)
    }

    pub fn configure_all(&self, cs: &CriticalSection) {
        for ep in &self.endpoints_in {
            if ep.is_initialized() {
                ep.configure(cs);
            }
        }

        for ep in &self.endpoints_out {
            if ep.is_initialized() {
                ep.configure(cs);
            }
        }
    }

    pub fn deconfigure_all(&self, cs: &CriticalSection) {
        for ep in &self.endpoints_in {
            ep.deconfigure(cs);
        }

        for ep in &self.endpoints_out {
            ep.deconfigure(cs);
        }
    }
}

fn find_free_endpoint<EP: Deref<Target=Endpoint>>(
    endpoints: &mut [EP],
    ep_addr: Option<EndpointAddress>
) -> Result<&mut EP>
{
    if let Some(address) = ep_addr {
        for ep in endpoints {
            if ep.address == address {
                if !ep.is_initialized() {
                    return Ok(ep);
                } else {
                    return Err(UsbError::InvalidEndpoint);
                }
            }
        }
        Err(UsbError::InvalidEndpoint)
    } else {
        for ep in &mut endpoints[1..] {
            if !ep.is_initialized() {
                return Ok(ep)
            }
        }
        Err(UsbError::EndpointOverflow)
    }
}

impl<PINS: Send+Sync> usb_device::bus::UsbBus for UsbBus<PINS> {
    fn alloc_ep(
        &mut self,
        ep_dir: UsbDirection,
        ep_addr: Option<EndpointAddress>,
        ep_type: EndpointType,
        max_packet_size: u16,
        _interval: u8) -> Result<EndpointAddress>
    {
        if ep_dir == UsbDirection::In {
            let ep = find_free_endpoint(&mut self.endpoints_in, ep_addr)?;
            ep.initialize(ep_type, max_packet_size);

            Ok(ep.address)
        } else {
            let ep = find_free_endpoint(&mut self.endpoints_out, ep_addr)?;

            let buffer = self.endpoint_allocator.allocate_rx_buffer(max_packet_size as usize)?;
            ep.initialize(ep_type, max_packet_size, buffer);

            Ok(ep.address)
        }
    }

    fn enable(&mut self) {
        // Enable USB_OTG in RCC
        apb_usb_enable();

        interrupt::free(|cs| {
            let regs = self.regs.borrow(cs);

            // Wait for AHB ready
            while read_reg!(otg_fs_global, regs.global, FS_GRSTCTL, AHBIDL) == 0 {}

            // Configure OTG as device
            modify_reg!(otg_fs_global, regs.global, FS_GUSBCFG,
                SRPCAP: 0, // SRP capability is not enabled
                TRDT: 0x6, // ??? USB turnaround time
                FDMOD: 1 // Force device mode
            );

            // Configuring Vbus sense and SOF output
            //write_reg!(otg_fs_global, regs.global, FS_GCCFG, VBUSBSEN: 1);
            write_reg!(otg_fs_global, regs.global, FS_GCCFG, 1 << 21); // set NOVBUSSENS

            // Enable PHY clock
            write_reg!(otg_fs_pwrclk, regs.pwrclk, FS_PCGCCTL, 0);

            // Soft disconnect device
            modify_reg!(otg_fs_device, regs.device, FS_DCTL, SDIS: 1);

            // Setup USB FS speed [and frame interval]
            modify_reg!(otg_fs_device, regs.device, FS_DCFG,
                DSPD: 0b11 // Device speed: Full speed
            );

            // Setting max RX FIFO size
            write_reg!(otg_fs_global, regs.global, FS_GRXFSIZ, RX_FIFO_SIZE);

            // setting up EP0 TX FIFO SZ as 64 byte
            write_reg!(otg_fs_global, regs.global, FS_GNPTXFSIZ,
                TX0FD: 16,
                TX0FSA: RX_FIFO_SIZE
            );

            // unmask EP interrupts
            write_reg!(otg_fs_device, regs.device, DIEPMSK, XFRCM: 1);

            // unmask core interrupts
            write_reg!(otg_fs_global, regs.global, FS_GINTMSK,
                USBRST: 1, ENUMDNEM: 1,
                USBSUSPM: 1, WUIM: 1,
                IEPINT: 1, RXFLVLM: 1
            );

            // clear pending interrupts
            write_reg!(otg_fs_global, regs.global, FS_GINTSTS, 0xffffffff);

            // unmask global interrupt
            modify_reg!(otg_fs_global, regs.global, FS_GAHBCFG, GINT: 1);

            // connect(true)
            modify_reg!(otg_fs_global, regs.global, FS_GCCFG, PWRDWN: 1);
            modify_reg!(otg_fs_device, regs.device, FS_DCTL, SDIS: 0);
        });
    }

    fn reset(&self) {
        interrupt::free(|cs| {
            let regs = self.regs.borrow(cs);

            self.configure_all(cs);

            modify_reg!(otg_fs_device, regs.device, FS_DCFG, DAD: 0);
        });
    }

    fn set_device_address(&self, addr: u8) {
        interrupt::free(|cs| {
            let regs = self.regs.borrow(cs);

            modify_reg!(otg_fs_device, regs.device, FS_DCFG, DAD: addr as u32);
        });
    }

    fn write(&self, ep_addr: EndpointAddress, buf: &[u8]) -> Result<usize> {
        if !ep_addr.is_in() || ep_addr.index() >= 4 {
            return Err(UsbError::InvalidEndpoint);
        }

        self.endpoints_in[ep_addr.index()].write(buf).map(|_| buf.len())
    }

    fn read(&self, ep_addr: EndpointAddress, buf: &mut [u8]) -> Result<usize> {
        if !ep_addr.is_out() || ep_addr.index() >= 4 {
            return Err(UsbError::InvalidEndpoint);
        }

        self.endpoints_out[ep_addr.index()].read(buf)
    }

    fn set_stalled(&self, ep_addr: EndpointAddress, stalled: bool) {
        if ep_addr.index() >= 4 {
            return;
        }

        if ep_addr.is_in() {
            self.endpoints_in[ep_addr.index()].set_stalled(stalled)
        } else {
            self.endpoints_out[ep_addr.index()].set_stalled(stalled)
        }
    }

    fn is_stalled(&self, ep_addr: EndpointAddress) -> bool {
        if ep_addr.index() >= 4 {
            return true;
        }

        if ep_addr.is_in() {
            self.endpoints_in[ep_addr.index()].is_stalled()
        } else {
            self.endpoints_out[ep_addr.index()].is_stalled()
        }
    }

    fn suspend(&self) {
        // Nothing to do here?
    }

    fn resume(&self) {
        // Nothing to do here?
    }

    fn poll(&self) -> PollResult {
        interrupt::free(|cs| {
            let regs = self.regs.borrow(cs);

            let (wakeup, suspend, enum_done, reset, iep, rxflvl) = read_reg!(otg_fs_global, regs.global, FS_GINTSTS,
                WKUPINT, USBSUSP, ENUMDNE, USBRST, IEPINT, RXFLVL
            );

            if reset != 0 {
                write_reg!(otg_fs_global, regs.global, FS_GINTSTS, USBRST: 1);

                self.deconfigure_all(cs);

                // Flush RX
                modify_reg!(otg_fs_global, regs.global, FS_GRSTCTL, RXFFLSH: 1);
                while read_reg!(otg_fs_global, regs.global, FS_GRSTCTL, RXFFLSH) == 1 {}
            }

            if enum_done != 0 {
                write_reg!(otg_fs_global, regs.global, FS_GINTSTS, ENUMDNE: 1);

                PollResult::Reset
            } else if wakeup != 0 {
                // Clear the interrupt
                write_reg!(otg_fs_global, regs.global, FS_GINTSTS, WKUPINT: 1);

                PollResult::Resume
            } else if suspend != 0 {
                write_reg!(otg_fs_global, regs.global, FS_GINTSTS, USBSUSP: 1);

                PollResult::Suspend
            } else if (iep | rxflvl) != 0 {
                // Flags are read-only, there is no need to clear them

                let mut ep_out = 0;
                let mut ep_in_complete = 0;
                let mut ep_setup = 0;

                use crate::ral::{endpoint_in, endpoint_out};

                if rxflvl != 0 {
                    let (epnum, status) = read_reg!(otg_fs_global, regs.global, FS_GRXSTSR, EPNUM, PKTSTS);
                    match status {
                        0x02 => { // OUT received
                            ep_out |= 1 << epnum;
                        }
                        0x06 => { // SETUP received
                            // flushing TX if something stuck in control endpoint
                            let ep = endpoint_in::instance(epnum as usize);
                            if read_reg!(endpoint_in, ep, DIEPTSIZ, PKTCNT) != 0 {
                                modify_reg!(otg_fs_global, regs.global, FS_GRSTCTL, TXFNUM: epnum, TXFFLSH: 1);
                                while read_reg!(otg_fs_global, regs.global, FS_GRSTCTL, TXFFLSH) == 1 {}
                            }
                            ep_setup |= 1 << epnum;
                        }
                        0x03 | 0x04 => { // OUT completed | SETUP completed
                            let ep = endpoint_out::instance(epnum as usize);
                            modify_reg!(endpoint_out, ep, DOEPCTL, CNAK: 1, EPENA: 1);
                            read_reg!(otg_fs_global, regs.global, GRXSTSP); // pop GRXSTSP
                        }
                        _ => {
                            read_reg!(otg_fs_global, regs.global, GRXSTSP); // pop GRXSTSP
                        }
                    }
                }

                if iep != 0 {
                    for ep in &self.endpoints_in {
                        if ep.is_initialized() {
                            let ep_regs = endpoint_in::instance(ep.address.index());
                            if read_reg!(endpoint_in, ep_regs, DIEPINT, XFRC) != 0 {
                                write_reg!(endpoint_in, ep_regs, DIEPINT, XFRC: 1);
                                ep_in_complete |= 1 << ep.address.index();
                            }
                        }
                    }
                }

                if (ep_in_complete | ep_out | ep_setup) != 0 {
                    PollResult::Data { ep_out, ep_in_complete, ep_setup }
                } else {
                    PollResult::None
                }
            } else {
                PollResult::None
            }
        })
    }

    const QUIRK_SET_ADDRESS_BEFORE_STATUS: bool = true;
}
