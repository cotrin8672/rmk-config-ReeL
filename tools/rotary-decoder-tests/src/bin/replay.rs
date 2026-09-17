fn main() {
    let path = std::env::args().nth(1).expect("usage: replay <rotary.log>");
    let text = std::fs::read_to_string(path).expect("read log");
    match rotary_decoder_tests::replay::validate(&text) {
        Ok(report) => println!(
            "REPLAY_OK samples={} dropped={} source={} output={} clears(confirm,cancel,idle)={:?}",
            report.count,
            report.dropped,
            report.final_source,
            report.final_output,
            report.clear_counts
        ),
        Err(error) => {
            eprintln!("INVALID_LOG: {error}");
            std::process::exit(1);
        }
    }
}
