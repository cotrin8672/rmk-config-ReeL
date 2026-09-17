use rotary_decoder_tests::{
    replay,
    rotary_decoder::ClockedDetentDecoder,
    rotary_log,
    rotary_trace::{Sample, Trace, Trigger},
};
use std::fmt::Write;

#[test]
fn measured_downward_click_replays_the_reported_wrong_history_output() {
    // Device capture 2026-09-17, firmware 493c176. User: wheel down, PC up.
    // Preserve this failure as evidence; do not fabricate a middle state to
    // turn replay green. The acquisition change needs a NEW device capture.
    let text = include_str!("fixtures/measured-down-output-up.log");
    let report = replay::validate(text).unwrap();
    assert_eq!(report.count, 16);
    assert_eq!(report.dropped, 0);
    assert_eq!(report.final_source, "H");
    assert_eq!(report.final_output, -1);
    assert_eq!(report.clear_counts, [1, 0, 0]);
    let rows: Vec<Vec<&str>> = text
        .lines()
        .skip(2)
        .take(16)
        .map(|row| row.split(',').collect())
        .collect();
    assert_eq!(rows[0][7], "0"); // Before first update: AB=00.
    for row in &rows {
        assert_eq!((row[2], row[3]), ("1", "1"));
        assert_eq!((row[22], row[23], row[24]), ("0", "0", "0"));
    }
    for pair in rows.windows(2) {
        let elapsed = pair[1][1].parse::<u64>().unwrap() - pair[0][1].parse::<u64>().unwrap();
        assert!((9..=10).contains(&elapsed)); // 275-305 us, not 2 ticks.
    }
}

#[test]
fn measured_interrupt_capture_still_has_no_gray_direction_evidence() {
    // User again reported down -> PC up. Moving the reader to an interrupt
    // executor alone did NOT resolve the observed loss of phase information.
    let text = include_str!("fixtures/measured-irq-down-output-up.log");
    let report = replay::validate(text).unwrap();
    assert_eq!((report.count, report.dropped), (37, 0));
    assert_eq!(report.final_source, "H");
    assert_eq!(report.final_output, 1);
    assert_eq!(report.clear_counts, [1, 0, 7]);
    let mut states: Vec<(&str, &str)> = text
        .lines()
        .skip(2)
        .take(37)
        .map(|row| {
            let fields: Vec<_> = row.split(',').collect();
            assert_eq!((fields[22], fields[23], fields[24]), ("0", "0", "0"));
            (fields[2], fields[3])
        })
        .collect();
    states.dedup();
    assert_eq!(states, [("0", "0"), ("1", "1"), ("0", "0"), ("1", "1")]);
}

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
