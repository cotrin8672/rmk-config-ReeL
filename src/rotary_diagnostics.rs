//! Left LCD diagnostic view. No display I/O or formatting in the capture loop.
use crate::{
    rotary_decoder::{DecisionSource, Detent},
    rotary_trace::{CaptureState, Record, Sample, Trace, Trigger},
};
use core::{
    cell::RefCell,
    fmt::{self, Write},
};
use embassy_sync::blocking_mutex::{Mutex, raw::ThreadModeRawMutex};
use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_5X7},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};
use rmk::display::{DisplayRenderer, RenderContext};

// Kept in static RAM; a debugger can inspect samples/records and their counters.
static TRACE: Mutex<ThreadModeRawMutex, RefCell<Trace>> = Mutex::new(RefCell::new(Trace::new()));
pub fn record(sample: Sample) {
    TRACE.lock(|trace| trace.borrow_mut().push(sample));
}
pub fn arm(trigger: Trigger) {
    TRACE.lock(|trace| trace.borrow_mut().arm(trigger));
}
pub fn status() -> (CaptureState, u32, usize, u64) {
    TRACE.lock(|trace| {
        let trace = trace.borrow();
        (trace.state, trace.generation, trace.len(), trace.dropped())
    })
}
pub fn sample(index: usize) -> Option<Sample> {
    TRACE.lock(|trace| trace.borrow().sample(index))
}

fn direction(value: Option<Detent>) -> &'static str {
    match value {
        Some(Detent::Clockwise) => "CW",
        Some(Detent::CounterClockwise) => "CCW",
        None => "-",
    }
}
fn source(value: DecisionSource) -> &'static str {
    match value {
        DecisionSource::Edge => "E",
        DecisionSource::Interval => "I",
        DecisionSource::History => "H",
        DecisionSource::Unknown => "U",
    }
}
struct Line {
    bytes: [u8; 64],
    len: usize,
}
impl Write for Line {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.len + text.len() > self.bytes.len() {
            return Err(fmt::Error);
        }
        self.bytes[self.len..self.len + text.len()].copy_from_slice(text.as_bytes());
        self.len += text.len();
        Ok(())
    }
}
fn line<D: DrawTarget<Color = BinaryColor>>(display: &mut D, y: i32, args: fmt::Arguments<'_>) {
    let mut text = Line {
        bytes: [0; 64],
        len: 0,
    };
    let _ = text.write_fmt(args);
    let style = MonoTextStyle::new(&FONT_5X7, BinaryColor::On);
    let _ = Text::with_baseline(
        core::str::from_utf8(&text.bytes[..text.len]).unwrap_or("?"),
        Point::new(0, y),
        style,
        Baseline::Top,
    )
    .draw(display);
}
fn draw_record<D: DrawTarget<Color = BinaryColor>>(display: &mut D, y: i32, record: Record) {
    let d = record.decision;
    // Display counters/times modulo 10^8; full values stay in RAM.
    line(display, y, format_args!("#{}", record.number % 100_000_000));
    line(
        display,
        y + 9,
        format_args!("{}>{}", source(d.source), direction(d.output)),
    );
    line(
        display,
        y + 18,
        format_args!(
            "t{}ms",
            embassy_time::Instant::from_ticks(record.sample.ticks).as_millis() % 100_000_000
        ),
    );
    line(
        display,
        y + 27,
        format_args!(
            "A{}>{} AB{}{}",
            u8::from(d.old_a),
            u8::from(d.new_a),
            u8::from(d.observed_a),
            u8::from(d.observed_b)
        ),
    );
    line(display, y + 36, format_args!("E:{}", d.edge_movement));
    line(display, y + 45, format_args!("I:{}", d.interval_movement));
    line(
        display,
        y + 54,
        format_args!("H:{}", direction(d.previous_direction)),
    );
}

pub struct DiagnosticRenderer {
    last_number: Option<(u32, u64, CaptureState)>,
}
impl DiagnosticRenderer {
    pub const fn new() -> Self {
        Self { last_number: None }
    }
}
impl DisplayRenderer<BinaryColor> for DiagnosticRenderer {
    fn render<D: DrawTarget<Color = BinaryColor>>(
        &mut self,
        _ctx: &RenderContext,
        display: &mut D,
    ) {
        let (key, newest, previous) = TRACE.lock(|trace| {
            let trace = trace.borrow();
            (
                (trace.generation, trace.decision_count, trace.state),
                trace.latest(0),
                trace.latest(1),
            )
        });
        if self.last_number == Some(key) {
            return;
        }
        self.last_number = Some(key);
        let _ = display.clear(BinaryColor::Off);
        line(
            display,
            0,
            format_args!(
                "{}",
                match key.2 {
                    CaptureState::Disarmed => "USB: RUN TOOL",
                    CaptureState::Armed => "ARMED",
                    CaptureState::Frozen => "FROZEN",
                }
            ),
        );
        if let Some(record) = newest {
            draw_record(display, 12, record);
        } else {
            line(display, 12, format_args!("Waiting..."));
        }
        if let Some(record) = previous {
            draw_record(display, 80, record);
        }
        line(display, 150, format_args!("USB LOG"));
    }
}
