use core::mem;
use cortex_m::interrupt::{self, Mutex, CriticalSection};
use usb_device::{Result, UsbError, UsbDirection};
use usb_device::endpoint::{EndpointType, EndpointAddress};
use crate::endpoint_memory::EndpointBuffer;
use usb_device::bus::PollResult;
use crate::ral::{endpoint_in, endpoint_out};
use stm32ral::{read_reg, modify_reg};

struct UnusedEndpoint {
    address: EndpointAddress,
}

struct EndpointIn {
    address: EndpointAddress,
    ep_type: EndpointType,
}

struct EndpointOut {
    address: EndpointAddress,
    ep_type: EndpointType,
    buffer: Mutex<EndpointBuffer>,
}

/// Arbitrates access to the endpoint-specific registers and packet buffer memory.
pub struct Endpoint {
    buffer: Option<Mutex<EndpointBuffer>>,
    ep_type: Option<EndpointType>,
    address: EndpointAddress,
}

impl Endpoint {
    pub fn new(address: EndpointAddress) -> Endpoint {
        Endpoint {
            buffer: None,
            ep_type: None,
            address,
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.ep_type.is_some()
    }

    pub fn set_stalled(&self, stalled: bool) {
        interrupt::free(|cs| {
            if self.is_stalled() == stalled {
                return
            }

            if self.address.is_in() {
                let ep = endpoint_in::instance(self.address.index());
                modify_reg!(endpoint_in, ep, DIEPCTL, Stall: stalled as u32);
            } else {
                let ep = endpoint_out::instance(self.address.index());
                modify_reg!(endpoint_out, ep, DOEPCTL, Stall: stalled as u32);
            }
        })
    }

    pub fn is_stalled(&self) -> bool {
        let stall = if self.address.is_in() {
            let ep = endpoint_in::instance(self.address.index());
            read_reg!(endpoint_in, ep, DIEPCTL, Stall)
        } else {
            let ep = endpoint_out::instance(self.address.index());
            read_reg!(endpoint_out, ep, DOEPCTL, Stall)
        };
        stall != 0
    }

    pub fn ep_type(&self) -> Option<EndpointType> {
        self.ep_type
    }

    pub fn set_ep_type(&mut self, ep_type: EndpointType) {
        self.ep_type = Some(ep_type);
    }

    pub fn configure(&self, cs: &CriticalSection) {
        /*let ep_type = match self.ep_type {
            Some(t) => t,
            None => { return },
        };

        self.reg().modify(|_, w| {
            Self::set_invariant_values(w)
                .ctr_rx().clear_bit()
                // dtog_rx
                // stat_rx
                .ep_type().bits(ep_type.bits())
                .ep_kind().clear_bit()
                .ctr_tx().clear_bit()
                // dtog_rx
                // stat_tx
        });

        self.set_stat_rx(cs,
            if self.out_buf.is_some() { EndpointStatus::Valid }
            else { EndpointStatus::Disabled} );

        self.set_stat_tx(cs,
            if self.in_buf.is_some() { EndpointStatus::Nak }
            else { EndpointStatus::Disabled} );*/
    }

    pub fn write(&self, buf: &[u8]) -> Result<()> {
        unimplemented!()
//        interrupt::free(|cs| {
//            let in_buf = self.in_buf.as_ref().unwrap().borrow(cs);
//
//            if buf.len() > in_buf.capacity() {
//                return Err(UsbError::BufferOverflow);
//            }
//
//            let reg = self.reg();
//
//            match reg.read().stat_tx().bits().into() {
//                EndpointStatus::Valid | EndpointStatus::Disabled => return Err(UsbError::WouldBlock),
//                _ => {},
//            };
//
//            in_buf.write(buf);
//            self.descr().count_tx.set(buf.len() as u16 as UsbAccessType);
//
//            self.set_stat_tx(cs, EndpointStatus::Valid);
//
//            Ok(())
//        })
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        unimplemented!()
//        interrupt::free(|cs| {
//            let out_buf = self.out_buf.as_ref().unwrap().borrow(cs);
//
//            let reg_v = self.reg().read();
//
//            let status: EndpointStatus = reg_v.stat_rx().bits().into();
//
//            if status == EndpointStatus::Disabled || !reg_v.ctr_rx().bit_is_set() {
//                return Err(UsbError::WouldBlock);
//            }
//
//            self.clear_ctr_rx(cs);
//
//            let count = (self.descr().count_rx.get() & 0x3ff) as usize;
//            if count > buf.len() {
//                return Err(UsbError::BufferOverflow);
//            }
//
//            out_buf.read(&mut buf[0..count]);
//
//            self.set_stat_rx(cs, EndpointStatus::Valid);
//
//            Ok(count)
//        })
    }
}


pub struct DeviceEndpoints {
    out_ep: [Endpoint; 4],
    in_ep: [Endpoint; 4],
}

impl DeviceEndpoints {
    pub fn new() -> Self {
        let out_ep = [
            Endpoint::new(EndpointAddress::from_parts(0, UsbDirection::Out)),
            Endpoint::new(EndpointAddress::from_parts(1, UsbDirection::Out)),
            Endpoint::new(EndpointAddress::from_parts(2, UsbDirection::Out)),
            Endpoint::new(EndpointAddress::from_parts(3, UsbDirection::Out)),
        ];
        let in_ep = [
            Endpoint::new(EndpointAddress::from_parts(0, UsbDirection::In)),
            Endpoint::new(EndpointAddress::from_parts(1, UsbDirection::In)),
            Endpoint::new(EndpointAddress::from_parts(2, UsbDirection::In)),
            Endpoint::new(EndpointAddress::from_parts(3, UsbDirection::In)),
        ];
        Self {
            out_ep,
            in_ep,
        }
    }

    fn find_free(
        endpoints: &mut [Endpoint],
        ep_addr: Option<EndpointAddress>
    ) -> Result<&mut Endpoint>
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

    pub fn alloc_ep(
        &mut self,
        ep_dir: UsbDirection,
        ep_addr: Option<EndpointAddress>,
        ep_type: EndpointType,
        max_packet_size: u16,
        _interval: u8) -> Result<EndpointAddress>
    {
        if ep_dir == UsbDirection::Out {
            let ep = Self::find_free(&mut self.out_ep, ep_addr)?;
            ep.ep_type = Some(ep_type);

            // TODO: allocate buffers

            Ok(ep.address)
        } else {
            let ep = Self::find_free(&mut self.out_ep, ep_addr)?;
            ep.ep_type = Some(ep_type);

            // TODO: allocate buffers

            Ok(ep.address)
        }
    }

    pub fn write_packet(&self, ep_addr: EndpointAddress, buf: &[u8]) -> Result<()> {
        if !ep_addr.is_in() || ep_addr.index() >= 4 {
            return Err(UsbError::InvalidEndpoint);
        }

        self.out_ep[ep_addr.index()].write(buf)
    }

    pub fn read_packet(&self, ep_addr: EndpointAddress, buf: &mut [u8]) -> Result<usize> {
        if !ep_addr.is_out() || ep_addr.index() >= 4 {
            return Err(UsbError::InvalidEndpoint);
        }

        self.in_ep[ep_addr.index()].read(buf)
    }

    pub fn poll(&self) -> PollResult {
        let mut ep_out = 0;
        let mut ep_in_complete = 0;
        let mut ep_setup = 0;

        for ep in &self.in_ep {
            if ep.ep_type.is_some() {
                ep_in_complete |= (1 << ep.address.index());
                // TODO
            }
        }

        for ep in &self.out_ep {
            if ep.ep_type.is_some() {
                ep_out |= (1 << ep.address.index());
                // TODO
            }
        }

        if (ep_in_complete | ep_out | ep_setup) != 0 {
            PollResult::Data { ep_out, ep_in_complete, ep_setup }
        } else {
            PollResult::None
        }
    }

    pub fn set_stalled(&self, ep_addr: EndpointAddress, stalled: bool) {
        self.out_ep[ep_addr.index()].set_stalled(stalled)
    }

    pub fn is_stalled(&self, ep_addr: EndpointAddress) -> bool {
        self.out_ep[ep_addr.index()].is_stalled()
    }
}
