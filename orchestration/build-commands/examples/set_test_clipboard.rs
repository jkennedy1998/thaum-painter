//! Debug helper: plants a small THAUM3D payload on the OS clipboard so the
//! stamp tool's OS-import path can be driven without first copying in-app.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let payload = r##"{"anchor":[0,0,0],"cells":[
        {"offset":[0,0,0],"graphic":{"Glyph":"@"},"color":{"FlatRgb":{"red":255,"green":60,"blue":60}},"weight_index":1},
        {"offset":[1,0,0],"graphic":{"Glyph":"#"},"color":{"FlatRgb":{"red":60,"green":255,"blue":60}},"weight_index":1},
        {"offset":[0,1,0],"graphic":{"Glyph":"%"},"color":{"FlatRgb":{"red":60,"green":120,"blue":255}},"weight_index":1}
    ]}"##;
    let text = format!("THAUM3D:{payload}");
    arboard::Clipboard::new()?.set_text(text)?;
    println!("clipboard set; holding X11 selection ownership...");
    // X11 clipboard ownership dies with this process; hold it so the
    // painter can import during the hold window.
    std::thread::sleep(std::time::Duration::from_secs(60));
    Ok(())
}
