#!/usr/bin/env -S cargo +nightly -Zscript
---
[package]
edition="2024"
[dependencies]
csv="1"
---
fn main()->Result<(),Box<dyn std::error::Error>>{
 let decoded=csv::Reader::from_path("target/all-items/decoded.csv")?.records().collect::<Result<Vec<_>,_>>()?;
 let final_rows=csv::Reader::from_path("translations.csv")?.records().collect::<Result<Vec<_>,_>>()?;
 assert_eq!(decoded.len(),1311);assert_eq!(decoded,final_rows);
 println!("Full decode restores all 1311 records and translations exactly");Ok(())
}
