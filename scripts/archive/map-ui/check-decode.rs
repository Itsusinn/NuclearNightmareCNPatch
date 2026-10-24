#!/usr/bin/env -S cargo +nightly -Zscript
---
[package]
edition="2024"
[dependencies]
csv="1"
---
use std::{collections::BTreeMap,error::Error};
fn load(path:&str)->Result<BTreeMap<(String,String),csv::StringRecord>,Box<dyn Error>>{Ok(csv::Reader::from_path(path)?.records().map(|r|{let r=r?;Ok(((r[0].to_owned(),r[1].to_owned()),r))}).collect::<Result<_,csv::Error>>()?)}
fn main()->Result<(),Box<dyn Error>>{let decoded=load("target/map-ui/decoded.csv")?;let source=load("translations.csv")?;assert_eq!(decoded.len(),1314);assert_eq!(decoded,source);println!("All 1314 records and exact translations survive full decode.");Ok(())}