#!/usr/bin/env -S cargo +nightly -Zscript
---
[package]
edition="2024"
[dependencies]
csv="1"
---
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let decoded=csv::Reader::from_path("target/ui-missing/current.csv")?.records().collect::<Result<Vec<_>,_>>()?;
    let translated=csv::Reader::from_path("translations.csv")?.records().collect::<Result<Vec<_>,_>>()?;
    assert_eq!(decoded.len(),1271); assert_eq!(decoded.len(),translated.len());
    let mut blank=0;
    for (a,b) in decoded.iter().zip(&translated) {
        for at in [0,1,2,3,5] { assert_eq!(&a[at],&b[at],"field {at}, key {}",&a[1]); }
        if a[4].is_empty() {blank+=1;} else {assert_eq!(&a[4],&b[4]);}
    }
    assert_eq!(blank,5);
    println!("All 1271 decoded source records match, including exact whitespace; 1266 translations retained and 5 added");
    Ok(())
}
