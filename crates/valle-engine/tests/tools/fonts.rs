#[path = "../cases/fonts.rs"]
mod fonts;

fn main() -> Result<(), String> {
    fonts::prepare()
}
