//! USART1 reads GPS data from GP-735T and sends it over USART2.
//! USART2 reads input and toggles GPS ON/OFF if b'0'/b'1'.
//! TODO: DMA

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use heapless::spsc::Queue;
use panic_semihosting as _; // logs messages to the host stderr; requires a debugger
use stm32l4::stm32l4x2::{self, interrupt};

static mut USART1_PERIPHERAL: Option<stm32l4x2::USART1> = None;
static mut USART2_PERIPHERAL: Option<stm32l4x2::USART2> = None;
static mut GPIOA_PERIPHERAL: Option<stm32l4x2::GPIOA> = None;
static mut BUFFER: Option<Queue<u16, 64>> = None;

/// Queue received bytes and enable USART2 TXE interrupt. Ignore null bytes.
#[interrupt]
fn USART1() {
    let usart1 = unsafe { USART1_PERIPHERAL.as_mut() }.unwrap();
    let usart2 = unsafe { USART2_PERIPHERAL.as_mut() }.unwrap();
    let buffer = unsafe { BUFFER.as_mut() }.unwrap();

    if usart1.isr.read().rxne().bit_is_set() {
        // Read off USART1, this clears RXNE flag
        let received_byte = usart1.rdr.read().rdr().bits();
        if received_byte != 0 {
            // Queue byte, do nothing if queue is full
            if buffer.enqueue(received_byte).is_ok() {
                // Enable USART2 TXE interrupt as buffer is now non-empty
                usart2.cr1.modify(|_, w| w.txeie().enabled());
            }
        }
    }
    // See reference manual p.1206 or ch. 38.7.
    // RXNE interrupt can also be triggered by overrun error. Flag must be cleared.
    if usart1.isr.read().ore().bit_is_set() {
        usart1.icr.write(|w| w.orecf().set_bit());
    }
}

/// Turn on/off A12 based on received byte
#[interrupt]
fn USART2() {
    let usart2 = unsafe { USART2_PERIPHERAL.as_mut() }.unwrap();
    let gpioa = unsafe { GPIOA_PERIPHERAL.as_mut() }.unwrap();
    let buffer = unsafe { BUFFER.as_mut() }.unwrap();

    if usart2.isr.read().txe().bit_is_set() {
        match buffer.dequeue() {
            // Write dequeued byte
            Some(byte) => {
                usart2.tdr.write(|w| w.tdr().bits(byte));
                if buffer.is_empty() {
                    usart2.cr1.modify(|_, w| w.txeie().disabled());
                }
            }
            // Buffer is empty, disable USART2 TXE interrupt
            None => usart2.cr1.modify(|_, w| w.txeie().disabled()),
        }
    }

    // Received command from UART adaptor - toggle GPS ON/OFF
    if usart2.isr.read().rxne().bit_is_set() {
        // Read off USART2, this clears RXNE flag
        let received_byte = usart2.rdr.read().rdr().bits();

        // Turn off if '0', turn on if '1'
        if received_byte == b'0'.into() {
            gpioa.bsrr.write(|w| w.br12().set_bit());
        } else if received_byte == b'1'.into() {
            gpioa.bsrr.write(|w| w.bs12().set_bit());
        }
    }
    if usart2.isr.read().ore().bit_is_set() {
        usart2.icr.write(|w| w.orecf().set_bit());
    }
}

#[entry]
fn main() -> ! {
    // Device defaults to 4MHz clock

    let dp = stm32l4x2::Peripherals::take().unwrap();

    // Enable peripheral clocks - GPIOA, USART1, USART2
    dp.RCC.ahb2enr.write(|w| w.gpioaen().set_bit());
    dp.RCC.apb2enr.write(|w| w.usart1en().set_bit());
    dp.RCC.apb1enr1.write(|w| w.usart2en().set_bit());

    // USART1: Configure A9 (TX), A10 (RX) as alternate function 7
    // USART2: Configure A2 (TX), A3 (RX) as alternate function 7
    // GPIOA: A12 as push-pull output
    dp.GPIOA.moder.write(|w| {
        w.moder2()
            .alternate()
            .moder3()
            .alternate()
            .moder9()
            .alternate()
            .moder10()
            .alternate()
            .moder12()
            .output() // push-pull by default
    });
    dp.GPIOA.ospeedr.write(|w| {
        w.ospeedr2()
            .very_high_speed()
            .ospeedr3()
            .very_high_speed()
            .ospeedr9()
            .very_high_speed()
            .ospeedr10()
            .very_high_speed()
    });
    dp.GPIOA.afrl.write(|w| w.afrl2().af7().afrl3().af7());
    dp.GPIOA.afrh.write(|w| w.afrh9().af7().afrh10().af7());

    // Configure baud rate 9600
    dp.USART1.brr.write(|w| w.brr().bits(417)); // 4Mhz / 9600 approx. 417
    dp.USART2.brr.write(|w| w.brr().bits(417)); // 4Mhz / 9600 approx. 417

    // USART1 interfaces with GPS - enable receiver and RXNE interrupt
    dp.USART1
        .cr1
        .write(|w| w.re().enabled().ue().enabled().rxneie().enabled());
    // USART2 interfaces with UART adaptor - enable receiver, transmitter and RXNE interrupt
    // TXE interrupt is enabled by USART1 on demand
    dp.USART2.cr1.write(|w| {
        w.re()
            .enabled()
            .te()
            .enabled()
            .ue()
            .enabled()
            .rxneie()
            .enabled()
    });

    unsafe {
        BUFFER = Some(Queue::default());
        // Unmask NVIC USART1, USART2 global interrupts
        cortex_m::peripheral::NVIC::unmask(stm32l4x2::Interrupt::USART1);
        cortex_m::peripheral::NVIC::unmask(stm32l4x2::Interrupt::USART2);
        USART1_PERIPHERAL = Some(dp.USART1);
        USART2_PERIPHERAL = Some(dp.USART2);
        GPIOA_PERIPHERAL = Some(dp.GPIOA);
    }

    #[allow(clippy::empty_loop)]
    loop {}
}
