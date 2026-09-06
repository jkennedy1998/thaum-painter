//! Debug helper: reads the OS clipboard and prints what it sees.
fn main() {
    match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
        Ok(text) => println!("read {} bytes: {:.80}", text.len(), text),
        Err(err) => println!("read failed: {err}"),
    }
}
