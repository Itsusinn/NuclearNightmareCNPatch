#!/usr/bin/env -S cargo +nightly -Zscript
---
[dependencies]
csv="1"
ttf-parser="=0.25.1"
---
use std::{fs,collections::BTreeSet,error::Error};
fn string(d:&[u8],at:usize)->(String,usize){let n=i32::from_le_bytes(d[at..at+4].try_into().unwrap());assert!(n<0);let end=at+4+(-n as usize)*2;let u:Vec<_>=d[at+4..end-2].chunks_exact(2).map(|s|u16::from_le_bytes(s.try_into().unwrap())).collect();assert_eq!(&d[end-2..end],&[0,0]);(String::from_utf16(&u).unwrap(),end)}
fn main()->Result<(),Box<dyn Error>>{
 let root="target/computer-ui/encode/build/NuclearNightmare/Content";
 let map=fs::read(format!("{root}/Maps/TutorialMap.uexp"))?;
 let previous=fs::read("target/all-items/encode/build/NuclearNightmare/Content/Maps/TutorialMap.uexp")?;
 assert_eq!(map.len(),previous.len());assert_eq!(&map[..124658455],&previous[..124658455]);assert_eq!(&map[124658479..],&previous[124658479..]);
 let (label,end)=string(&map,124658455);assert_eq!(label.trim_end(),"直升机位置");assert_eq!(end,124658479);
 let widget=fs::read(format!("{root}/Graphics/Journal/MapWidget.uexp"))?;
 let original=fs::read("target/map-ui/f700302f-MapWidget.uexp")?;
 assert_eq!(widget.len(),original.len());assert_eq!(&widget[..40925],&original[..40925]);assert_eq!(&widget[40939..],&original[40939..]);
 assert_eq!(&widget[40925..40939],&[0x34,0x1c,0x59,0x20,0,0,0,0x34,0xe5,0x65,0x20,0,0,0]);
 let data=fs::read("fonts/NNChinese-GB2312-Regular.ttf")?;let face=ttf_parser::Face::parse(&data,0)?;let mut chars=BTreeSet::new();let mut count=0;
 for row in csv::Reader::from_path("translations.csv")?.records(){let row=row?;count+=1;chars.extend(row[3].chars().chain(row[4].chars()).filter(|c|!c.is_control()));}
 let missing:Vec<_>=chars.iter().filter(|&&c|face.glyph_index(c).is_none()).collect();assert!(missing.is_empty(),"Missing glyphs: {missing:?}");
 println!("Map override verified; all bytes outside label slot match previous subtitle fix; day/night logic and all other widget bytes unchanged. {count} CSV records; {} glyphs covered.",chars.len());Ok(())
}