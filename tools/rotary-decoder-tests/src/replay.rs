//! Host-only validator/replayer for the framed USB log.
use crate::{
    rotary_decoder::{ClockedDetentDecoder, DecoderState, Detent},
    rotary_log,
};
use std::{
    format,
    string::{String, ToString},
    vec::Vec,
};

fn state(fields: &[&str]) -> Result<DecoderState, String> {
    fn number<T: core::str::FromStr>(s: &str) -> Result<T, String> {
        s.parse().map_err(|_| format!("invalid number {s}"))
    }
    fn boolean(s: &str) -> Result<bool, String> {
        match s {
            "0" => Ok(false),
            "1" => Ok(true),
            _ => Err(format!("invalid bool {s}")),
        }
    }
    let last_direction = match fields[8] {
        "0" => None,
        "1" => Some(Detent::Clockwise),
        "-1" => Some(Detent::CounterClockwise),
        _ => return Err("invalid direction".into()),
    };
    let s = DecoderState {
        stable_a: boolean(fields[0])?,
        candidate_a: boolean(fields[1])?,
        run_a: number(fields[2])?,
        previous_state: number(fields[3])?,
        unchanged_samples: number(fields[4])?,
        tracking_a_edge: boolean(fields[5])?,
        edge_movement: number(fields[6])?,
        interval_movement: number(fields[7])?,
        last_direction,
    };
    if s.previous_state > 3 {
        return Err("invalid A/B state".into());
    }
    Ok(s)
}
#[derive(Debug)]
pub struct Report {
    pub count: usize,
    pub dropped: u64,
    pub final_source: String,
    pub final_output: i8,
    pub clear_counts: [usize; 3],
}
pub fn validate(text: &str) -> Result<Report, String> {
    let lines: Vec<_> = text.lines().collect();
    if lines.len() < 4 {
        return Err("incomplete log".into());
    }
    let begin: Vec<_> = lines[0].split(',').collect();
    if begin.len() != 6 || begin[0] != "BEGIN" || begin[1] != "1" || begin[5] != "32768" {
        return Err("unsupported BEGIN".into());
    }
    let count = begin[3].parse::<usize>().map_err(|_| "invalid count")?;
    let dropped = begin[4].parse::<u64>().map_err(|_| "invalid dropped")?;
    if count == 0 || count > 512 || lines.len() != count + 3 || lines[1] != rotary_log::HEADER {
        return Err("count/header mismatch".into());
    }
    let end: Vec<_> = lines.last().unwrap().split(',').collect();
    if end.len() != 3 || end[0] != "END" || end[1].parse::<usize>().ok() != Some(count) {
        return Err("missing END".into());
    }
    let expected = u32::from_str_radix(end[2], 16).map_err(|_| "invalid checksum")?;
    let mut hash = 2166136261;
    for line in &lines[1..lines.len() - 1] {
        hash = rotary_log::checksum(hash, line.as_bytes());
        hash = rotary_log::checksum(hash, b"\n");
    }
    if hash != expected {
        return Err("checksum mismatch".into());
    }
    let mut decoder = None;
    let mut prior_ticks = 0;
    let mut report = Report {
        count,
        dropped,
        final_source: String::new(),
        final_output: 0,
        clear_counts: [0; 3],
    };
    for (index, line) in lines[2..2 + count].iter().enumerate() {
        let fields: Vec<_> = line.split(',').collect();
        if fields.len() != 28 {
            return Err(format!("row {index}: wrong column count"));
        }
        let number = fields[0].parse::<u64>().map_err(|_| "bad sample number")?;
        let ticks = fields[1].parse::<u64>().map_err(|_| "bad timestamp")?;
        if number != dropped + index as u64 + 1 || ticks < prior_ticks {
            return Err(format!("row {index}: sequence/time gap"));
        }
        prior_ticks = ticks;
        let before = state(&fields[4..13])?;
        let after = state(&fields[13..22])?;
        let decoder = decoder.get_or_insert_with(|| ClockedDetentDecoder::from_state(before));
        if decoder.state() != before {
            return Err(format!("row {index}: before-state mismatch"));
        }
        let a = match fields[2] {
            "0" => false,
            "1" => true,
            _ => return Err("invalid A".into()),
        };
        let b = match fields[3] {
            "0" => false,
            "1" => true,
            _ => return Err("invalid B".into()),
        };
        let output = decoder.update(a, b);
        let evidence = decoder.evidence();
        let decision = decoder.take_decision();
        let evidence_text = format!(
            "{},{},{},{}",
            evidence.delta,
            evidence.edge_before_clear,
            evidence.interval_before_clear,
            evidence.clears
        );
        if decoder.state() != after
            || evidence_text != fields[22..26].join(",")
            || rotary_log::source(decision.map(|d| d.source)) != fields[26]
            || rotary_log::direction(output).to_string() != fields[27]
        {
            return Err(format!("row {index}: replay mismatch"));
        }
        for bit in 0..3 {
            report.clear_counts[bit] += usize::from(evidence.clears & (1 << bit) != 0);
        }
        report.final_source = fields[26].into();
        report.final_output = rotary_log::direction(output);
    }
    if report.final_source == "-" {
        return Err("capture does not end at an A confirmation".into());
    }
    Ok(report)
}
