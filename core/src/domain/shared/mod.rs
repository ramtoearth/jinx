pub mod hex_color;

pub use hex_color::HexColor;

pub fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 29 } else { 28 },
        _ => 30,
    }
}
