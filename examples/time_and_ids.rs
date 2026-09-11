use n2f_rs::shared::{
    clock::FakeClock,
    id::{EntropyError, V7},
};
use std::time::{Duration, UNIX_EPOCH};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let c = FakeClock::new(UNIX_EPOCH + Duration::from_millis(0x0123456789ab));
    // Deterministic entropy belongs only in tests/examples.
    let mut g = V7::with_entropy(
        || c.now(),
        |b: &mut [u8]| -> Result<(), EntropyError> {
            b.fill(0);
            Ok(())
        },
    );
    let a = g.new_id()?;
    c.advance(Duration::from_millis(25))?;
    c.set(UNIX_EPOCH + Duration::from_millis(0x0123456789aa));
    let b = g.new_id()?;
    println!("{a}");
    println!("{b}");
    println!("{}ms", c.elapsed().as_millis());
    Ok(())
}
