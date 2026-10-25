#!/usr/bin/env -S cargo +nightly -Zscript
---
[dependencies]
csv = "1"
---
use std::{error::Error, fs, io::Write};
fn main() -> Result<(), Box<dyn Error>> {
    let mut reader = csv::Reader::from_path("target/update-20261024/current.csv")?;
    let mut file = fs::File::create("translations.csv")?;
    file.write_all(b"\xEF\xBB\xBF")?;
    let mut writer = csv::WriterBuilder::new().quote_style(csv::QuoteStyle::Always).terminator(csv::Terminator::CRLF).from_writer(file);
    writer.write_record(reader.headers()?)?;
    for row in reader.records() { writer.write_record(&row?)?; }
    writer.flush()?;
    Ok(())
}
