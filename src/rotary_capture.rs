//! Continuous signed A-edge acquisition, independent of executor/BLE latency.
//!
//! TIMER1/2: positive/negative A edges; TIMER3: 1 MHz, 1 ms A quiet time;
//! TIMER4: stable-frame count. GPIOTE0/1: P1.14/P1.15 toggles.
//! PPI0..5 and groups 0/1 implement EQ/NE; PPI6..8 handle framing.
//! SDC reserves channels 17..31; MPSL reserves TIMER0 and 19/30/31.
//! Their nRF52 integration resource lists reserve no PPI groups (no FEM here).
//! The owned tokens prevent reuse by other application drivers.
//!
//! This is an experimental circuit: simultaneous/closely spaced A/B events
//! and A edges coincident with the quiet deadline require hardware testing.

use crate::rotary_decoder::Frame;
use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use embassy_nrf::interrupt::InterruptExt;
use embassy_nrf::{Peri, interrupt, pac, peripherals::*};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, signal::Signal,
};

macro_rules! resources {
    ($($name:ident),+ $(,)?) => {
        #[allow(non_snake_case, dead_code)]
        pub struct Resources { $(pub $name: Peri<'static, $name>,)+ }
    };
}
resources!(
    P1_14, P1_15, GPIOTE_CH0, GPIOTE_CH1, TIMER1, TIMER2, TIMER3, TIMER4, PPI_CH0, PPI_CH1,
    PPI_CH2, PPI_CH3, PPI_CH4, PPI_CH5, PPI_CH6, PPI_CH7, PPI_CH8, PPI_GROUP0, PPI_GROUP1
);

const ALL_CHANNELS: u32 = 0x1ff;
const EQ_CHANNELS: u32 = 0x07;
const NE_CHANNELS: u32 = 0x38;
const FRAME_CHANNELS: u32 = 0x1c0;
static FRAMES: Channel<CriticalSectionRawMutex, Frame, 64> = Channel::new();
static FAULT_WAKE: Signal<CriticalSectionRawMutex, ()> = Signal::new();
// 1: startup activity, 2: capture overwrite, 3: frame queue full,
// 4: decoder sequence error, 5: output queue full. Latched until reset.
static FAULT: AtomicU8 = AtomicU8::new(0);
static LAST_SEQUENCE: AtomicU32 = AtomicU32::new(0);

pub fn fault_code() -> u8 {
    FAULT.load(Ordering::Acquire)
}
pub fn fail(code: u8) {
    let _ = FAULT.compare_exchange(0, code, Ordering::AcqRel, Ordering::Acquire);
    pac::PPI.chenclr().write(|w| w.0 = ALL_CHANNELS);
    pac::TIMER3.intenclr().write(|w| w.set_compare(0, true));
    pac::TIMER3.tasks_stop().write_value(1);
    FAULT_WAKE.signal(());
}

pub struct Capture {
    _resources: Resources,
}

fn connect(channel: usize, event: *mut u32, task: *mut u32, fork: Option<*mut u32>) {
    pac::PPI.ch(channel).eep().write_value(event as u32);
    pac::PPI.ch(channel).tep().write_value(task as u32);
    pac::PPI
        .fork(channel)
        .tep()
        .write_value(fork.map_or(0, |p| p as u32));
}

impl Capture {
    pub fn new(
        resources: Resources,
        _irq: impl interrupt::typelevel::Binding<interrupt::typelevel::TIMER3, InterruptHandler>,
    ) -> Self {
        let p = pac::PPI;
        let g = pac::GPIOTE;
        let positive = pac::TIMER1;
        let negative = pac::TIMER2;
        let quiet = pac::TIMER3;
        let frames = pac::TIMER4;
        p.chenclr().write(|w| w.0 = ALL_CHANNELS);
        for timer in [positive, negative, quiet, frames] {
            timer.tasks_stop().write_value(1);
            timer.tasks_clear().write_value(1);
            timer.intenclr().write(|w| w.0 = u32::MAX);
            timer.shorts().write(|w| w.0 = 0);
            timer
                .bitmode()
                .write(|w| w.set_bitmode(pac::timer::vals::Bitmode::_32bit));
            timer
                .mode()
                .write(|w| w.set_mode(pac::timer::vals::Mode::Counter));
            for i in 0..4 {
                timer.events_compare(i).write_value(0);
            }
        }
        quiet
            .mode()
            .write(|w| w.set_mode(pac::timer::vals::Mode::Timer));
        quiet.prescaler().write(|w| w.set_prescaler(4));
        quiet.cc(0).write_value(1000);
        // One frame per burst; only an A event restarts this timer.
        quiet.shorts().write(|w| w.set_compare_stop(0, true));
        p.chg(0).write(|w| w.0 = EQ_CHANNELS);
        p.chg(1).write(|w| w.0 = NE_CHANNELS);
        let a = g.events_in(0).as_ptr();
        let b = g.events_in(1).as_ptr();
        connect(
            0,
            a,
            positive.tasks_count().as_ptr(),
            Some(p.tasks_chg(0).dis().as_ptr()),
        );
        connect(1, a, p.tasks_chg(1).en().as_ptr(), None);
        connect(
            2,
            b,
            p.tasks_chg(0).dis().as_ptr(),
            Some(p.tasks_chg(1).en().as_ptr()),
        );
        connect(
            3,
            a,
            negative.tasks_count().as_ptr(),
            Some(p.tasks_chg(1).dis().as_ptr()),
        );
        connect(4, a, p.tasks_chg(0).en().as_ptr(), None);
        connect(
            5,
            b,
            p.tasks_chg(1).dis().as_ptr(),
            Some(p.tasks_chg(0).en().as_ptr()),
        );
        connect(
            6,
            a,
            quiet.tasks_clear().as_ptr(),
            Some(quiet.tasks_start().as_ptr()),
        );
        connect(
            7,
            quiet.events_compare(0).as_ptr(),
            positive.tasks_capture(0).as_ptr(),
            Some(negative.tasks_capture(0).as_ptr()),
        );
        connect(
            8,
            quiet.events_compare(0).as_ptr(),
            frames.tasks_count().as_ptr(),
            None,
        );
        for timer in [positive, negative, frames] {
            timer.tasks_start().write_value(1);
        }

        // Configure inputs once; never wait/rearm GPIOTE from software.
        // Reject activity during initialization rather than seed a guessed EQ.
        for (channel, pin) in [(0, 14), (1, 15)] {
            pac::P1.pin_cnf(pin).write(|w| {
                w.set_pull(pac::gpio::vals::Pull::Pullup);
            });
            g.intenclr(0).write(|w| w.0 = 1 << channel);
            g.config(channel).write(|w| {
                w.set_mode(pac::gpiote::vals::Mode::Event);
                w.set_psel(pin as u8);
                w.set_port(true);
                w.set_polarity(pac::gpiote::vals::Polarity::Toggle);
            });
            g.events_in(channel).write_value(0);
        }
        let levels = pac::P1.in_().read();
        p.chenset().write(|w| {
            w.0 = FRAME_CHANNELS
                | if levels.pin(14) == levels.pin(15) {
                    EQ_CHANNELS
                } else {
                    NE_CHANNELS
                }
        });
        cortex_m::asm::dsb();
        if g.events_in(0).read() != 0 || g.events_in(1).read() != 0 {
            fail(1);
        } else {
            quiet.intenset().write(|w| w.set_compare(0, true));
            interrupt::TIMER3.set_priority(interrupt::Priority::P3);
            interrupt::TIMER3.unpend();
            // SAFETY: binding proves the handler; resources remain owned forever.
            unsafe {
                interrupt::TIMER3.enable();
            }
        }
        Self {
            _resources: resources,
        }
    }

    pub async fn receive(&mut self) -> Result<Frame, u8> {
        use rmk::embassy_futures::select::{Either, select};
        if fault_code() != 0 {
            return Err(fault_code());
        }
        match select(FRAMES.receive(), FAULT_WAKE.wait()).await {
            Either::First(frame) if fault_code() == 0 => Ok(frame),
            _ => Err(fault_code()),
        }
    }
}

fn sequence() -> u32 {
    pac::TIMER4.tasks_capture(0).write_value(1);
    cortex_m::asm::dsb();
    pac::TIMER4.cc(0).read()
}

pub struct InterruptHandler;
impl interrupt::typelevel::Handler<interrupt::typelevel::TIMER3> for InterruptHandler {
    unsafe fn on_interrupt() {
        if pac::TIMER3.events_compare(0).read() == 0 || fault_code() != 0 {
            return;
        }
        pac::TIMER3.events_compare(0).write_value(0);
        let first = sequence();
        let frame = Frame {
            positive: pac::TIMER1.cc(0).read(),
            negative: pac::TIMER2.cc(0).read(),
            sequence: first,
        };
        // Detect both already overwritten frames and overwrite while copying.
        let second = sequence();
        if first != second
            || first.wrapping_sub(LAST_SEQUENCE.load(Ordering::Relaxed)) != 1
            || pac::TIMER3.events_compare(0).read() != 0
        {
            fail(2);
            return;
        }
        LAST_SEQUENCE.store(first, Ordering::Relaxed);
        if FRAMES.try_send(frame).is_err() {
            fail(3);
        }
    }
}
