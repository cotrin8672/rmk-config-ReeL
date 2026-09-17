use rotary_decoder_tests::{
    replay,
    rotary_decoder::ClockedDetentDecoder,
    rotary_log,
    rotary_trace::{Sample, Trace, Trigger},
};
use std::fmt::Write;
fn log() -> String {
    let mut decoder = ClockedDetentDecoder::new(false, false);
    let mut trace = Trace::new();
    trace.arm(Trigger::Ambiguous);
    for n in 0..700 {
        let (a, b) = if n < 600 {
            (false, false)
        } else if n < 616 {
            (true, false)
        } else if n < 636 {
            (true, true)
        } else {
            (false, false)
        };
        let before = decoder.state();
        decoder.update(a, b);
        trace.push(Sample {
            number: 0,
            ticks: n,
            a,
            b,
            before,
            after: decoder.state(),
            evidence: decoder.evidence(),
            decision: decoder.take_decision(),
        });
    }
    let mut body = format!("{}\n", rotary_log::HEADER);
    for index in 0..trace.len() {
        rotary_log::row(&mut body, trace.sample(index).unwrap()).unwrap();
    }
    let hash = rotary_log::checksum(2166136261, body.as_bytes());
    format!(
        "BEGIN,1,1,{},{},32768\n{}END,{},{:08x}\n",
        trace.len(),
        trace.dropped(),
        body,
        trace.len(),
        hash
    )
}
#[test]
fn exported_frozen_trace_replays_with_wrapped_prefix() {
    let text = log();
    let report = replay::validate(&text).unwrap();
    assert_eq!(report.final_source, "H");
    assert_eq!(report.final_output, 1);
    assert_eq!(report.count, 512);
    assert!(report.dropped > 0);
    if let Ok(path) = std::env::var("ROTARY_TEST_LOG") {
        std::fs::write(path, text).unwrap();
    }
}
#[test]
fn damaged_truncated_and_reordered_logs_are_rejected() {
    let text = log();
    assert!(replay::validate(&text.replace("END,", "BROKEN,")).is_err());
    assert!(replay::validate(&text.replace(",H,1", ",H,-1")).is_err());
    let mut lines: Vec<_> = text.lines().map(str::to_owned).collect();
    lines.swap(2, 3);
    // Recompute transport checksum: replay must still reject reordered rows.
    let body = lines[1..lines.len() - 1].join("\n") + "\n";
    let end = lines.len() - 1;
    lines[end] = format!(
        "END,512,{:08x}",
        rotary_log::checksum(2166136261, body.as_bytes())
    );
    assert!(replay::validate(&(lines.join("\n") + "\n")).is_err());
    let mut forged = String::new();
    write!(&mut forged, "{}", text.trim_end_matches('\n')).unwrap();
    // Missing final newline is harmless; content still complete.
    assert!(replay::validate(&forged).is_ok());
}
