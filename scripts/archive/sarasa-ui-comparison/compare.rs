#!/usr/bin/env -S cargo +nightly -Zscript
---
[dependencies]
anyhow = "1"
csv = "1"
fontdue = "=0.9.5"
png = "=0.17.16"
serde_json = "1"
ttf-parser = "=0.25.1"
---
use anyhow::{Result, Context};
use fontdue::{Font, FontSettings};
use serde_json::json;
use std::{fs, collections::BTreeSet, path::Path};
use ttf_parser::Face;

fn name(face: &Face, id: u16) -> String {
    face.names().into_iter().find(|n| n.name_id == id && n.language_id == 0x409)
        .and_then(|n| n.to_string()).unwrap_or_default()
}
fn width(face: &Face, s: &str) -> f64 {
    s.chars().filter_map(|c| face.glyph_index(c)).filter_map(|g| face.glyph_hor_advance(g))
        .map(f64::from).sum::<f64>() * 32.0 / f64::from(face.units_per_em())
}
fn draw(pixels: &mut [u8], font: &Font, text: &str, size: f32, mut x: f32, baseline: i32, color: [u8;3]) {
    for c in text.chars() {
        let (m, bmp) = font.rasterize(c, size);
        for row in 0..m.height { for col in 0..m.width {
            let px=x.round() as i32+m.xmin+col as i32;
            let py=baseline-m.ymin-m.height as i32+row as i32;
            if (0..1640).contains(&px) && (0..800).contains(&py) {
                let i=(py as usize*1640+px as usize)*3;
                let a=bmp[row*m.width+col] as u32;
                for ch in 0..3 { pixels[i+ch]=((pixels[i+ch] as u32*(255-a)+color[ch] as u32*a)/255) as u8; }
            }
        }}
        x += m.advance_width;
    }
}
fn main() -> Result<()> {
    let root=Path::new(".");
    let a=fs::read(root.join("fonts/SarasaGothicSC-Regular.ttf"))?;
    let b=fs::read(root.join("fonts/SarasaUiSC-Regular.ttf"))?;
    let faces=[Face::parse(&a,0)?,Face::parse(&b,0)?];
    let fonts=[Font::from_bytes(a.as_slice(),FontSettings::default()).map_err(anyhow::Error::msg)?,
        Font::from_bytes(b.as_slice(),FontSettings::default()).map_err(anyhow::Error::msg)?];
    let mut cps=BTreeSet::new();
    for table in faces[0].tables().cmap.context("cmap")?.subtables.into_iter().filter(|t|t.is_unicode()) {
        table.codepoints(|cp| {cps.insert(cp);});
    }
    let mut differences=Vec::new();
    for cp in cps { if let Some(c)=char::from_u32(cp) {
        let wa=faces[0].glyph_index(c).and_then(|g| faces[0].glyph_hor_advance(g));
        let wb=faces[1].glyph_index(c).and_then(|g| faces[1].glyph_hor_advance(g));
        if wa!=wb { differences.push(json!({"unicode":format!("U+{cp:04X}"),"char":c,"gothic":wa,"ui":wb})); }
    }}
    let samples=["现场终端 / 系统设置", "画面   控制   音频   显示   教程", "行走（WASD）  使用（鼠标左键）",
        "请选择“English”，然后重启游戏。", "按“E”互动，按‘V’进行语音聊天。", "\"English\"  'WASD'  Very High  0123456789"];
    let mut pixels=vec![0;1640*800*3];
    for pixel in pixels.chunks_mut(3) {pixel.copy_from_slice(&[20,27,35]);}
    for (index,(face,font)) in faces.iter().zip(&fonts).enumerate() {
        let x=28.0+index as f32*820.0;
        draw(&mut pixels,font,if index==0 {"Sarasa Gothic SC"} else {"Sarasa UI SC"},30.0,x,50,[124,211,255]);
        draw(&mut pixels,font,"32 px - same scale / unhinted",18.0,x,84,[170,180,190]);
        for (row,text) in samples.iter().enumerate() {draw(&mut pixels,font,text,32.0,x,145+row as i32*65,[238,241,245]);}
        draw(&mut pixels,font,"18 px",18.0,x,570,[124,211,255]);
        draw(&mut pixels,font,samples[3],18.0,x,604,[238,241,245]);
        draw(&mut pixels,font,samples[4],18.0,x,638,[238,241,245]);
        draw(&mut pixels,font,"48 px - quotes",18.0,x,692,[124,211,255]);
        draw(&mut pixels,font,"“中文”  ‘English’",48.0,x,756,[238,241,245]);
        let _=face;
    }
    let output=root.join("target/sarasa-ui-comparison");
    let mut encoder=png::Encoder::new(fs::File::create(output.join("gothic-vs-ui.png"))?,1640,800);
    encoder.set_color(png::ColorType::Rgb); encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&pixels)?;
    let mut csv=csv::Reader::from_path(root.join("translations.csv"))?;
    let headers=csv.headers()?.clone();
    let trans=headers.iter().position(|x|x=="trans_text").context("column")?;
    let mut changed_rows=0;
    for row in csv.records() {let row=row?;if width(&faces[0],&row[trans])!=width(&faces[1],&row[trans]) {changed_rows+=1;}}
    let report=json!({"fonts":faces.iter().map(|f|json!({"family":name(f,1),"version":name(f,5),"units_per_em":f.units_per_em(),"ascender":f.ascender(),"descender":f.descender(),"line_gap":f.line_gap(),"typo_ascender":f.typographic_ascender(),"typo_descender":f.typographic_descender()})).collect::<Vec<_>>(),"width_differences":differences,"translation_rows_with_changed_advance":changed_rows,"samples_32px":samples.iter().map(|s|json!({"text":s,"gothic_width":width(&faces[0],s),"ui_width":width(&faces[1],s)})).collect::<Vec<_>>()});
    fs::write(output.join("gothic-vs-ui.json"),serde_json::to_string_pretty(&report)?)?;
    println!("{}",serde_json::to_string_pretty(&report)?);
    Ok(())
}
