use emtm_verify;
use emtm_verify::Verifier;
use std::io::Read;

fn main() {
    let mut image_data = vec![];
    std::fs::File::open("../card/card.jpg")
        .unwrap()
        .read_to_end(&mut image_data)
        .unwrap();
    let v = Verifier::new();

    println!(
        "{:?}",
        v.verify(&image_data, "中山大学", Some("16340025"))
    );
}
