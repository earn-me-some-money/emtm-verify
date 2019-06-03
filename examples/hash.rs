use chrono;
use emtm_verify;
use emtm_verify::Verifier;
use rand::random;
use std::collections::BTreeMap;

fn main() {
    let v = Verifier::new();
    let params = {
        let mut map = BTreeMap::new();
        map.insert("app_id", 1000001.to_string());
        map.insert("time_stamp", chrono::Utc::now().timestamp().to_string());
        map.insert(
            "nonce_str",
            (0..30)
                .map(|_| (0x20u8 + (random::<f32>() * 96.0) as u8) as char)
                .collect(),
        );
        map.insert(
            "image",
            (0..951434)
                .map(|_| (0x20u8 + (random::<f32>() * 96.0) as u8) as char)
                .collect(),
        );
        map
    };
    println!("{}", v.get_sign_hash(&params));
}
