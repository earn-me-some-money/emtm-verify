use dotenv::dotenv;
use hex;
use image::{GenericImageView, ImageError};
use md5::{Digest, Md5};
use rand::random;
use std::collections::BTreeMap;
use std::env;

use actix_web::client::{Client, SendRequestError};
use futures::Future;

use log::*;
use serde::*;

pub struct Verifier {
    app_id: u64,
    app_key: String,
}

#[derive(Debug)]
pub enum VerifierError {
    /// Verification info doesn't match
    StudentIdNotMatch,
    InstituteNotMatch,
    /// Failed to process image data
    ImageDataError(ImageError),
    /// Failed to encode image data
    JpegEncodeError(std::io::Error),
    /// Failed to connect to api server
    ApiServerConnectionError(SendRequestError),
    /// Server returns error message
    ServerResponseError(String),
    /// Api server internal error
    ApiServerError(String),
}

#[derive(Serialize, Deserialize, Debug)]
struct RequestForm {
    pub app_id: String,
    pub time_stamp: String,
    pub nonce_str: String,
    pub image: String,
    pub sign: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct OcrItem {
    pub item: String,
    pub itemstring: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct ResponseData {
    pub angle: String,
    pub item_list: Vec<OcrItem>,
}

#[derive(Serialize, Deserialize, Debug)]
struct ResponseParams {
    pub ret: u64,
    pub msg: String,
    pub data: ResponseData,
}

static OCR_URL: &str = "https://api.ai.qq.com/fcgi-bin/ocr/ocr_bcocr";

impl Verifier {
    pub fn new() -> Self {
        dotenv().ok();
        openssl_probe::init_ssl_cert_env_vars();
        let app_id_str = env::var("TENCENT_APP_ID").expect("TENCENT_APP_ID must be set.");
        let app_id = app_id_str
            .parse::<u64>()
            .expect("TENCENT_APP_ID must be an integer");
        let app_key = env::var("TENCENT_APP_KEY").expect("TENCENT_APP_KEY must be set.");
        Self { app_id, app_key }
    }

    pub fn get_sign_hash(&self, params: &BTreeMap<&str, String>) -> String {
        let mut encoded = vec![];
        for (key, value) in params {
            encoded.push([*key, value].join("="));
        }
        encoded.push(["app_key", &self.app_key].join("="));
        let to_hash = encoded.join("&");
        debug!("to_hash: {}", to_hash);
        let mut hasher = Md5::new();
        hasher.input(to_hash);

        hex::encode(&hasher.result()[..]).to_ascii_uppercase()
    }

    pub fn verify(
        &self,
        image_data: &[u8],
        institute: &str,
        student_id: Option<&str>,
    ) -> Box<Future<Item = (), Error = VerifierError>> {
        let mut img = match image::load_from_memory(image_data) {
            Ok(img) => img,
            Err(e) => {
                return Box::new(futures::future::err(VerifierError::ImageDataError(e)));
            }
        };

        // Api only allows image smaller than 1mb
        if image_data.len() > 1048576 {
            info!("Rescale for verification of {}:{:?}", institute, student_id);
            let scalar = (image_data.len() as f64 / 1000000.0).sqrt();
            img = img.resize(
                (img.width() as f64 / scalar) as u32,
                (img.height() as f64 / scalar) as u32,
                image::FilterType::CatmullRom,
            )
        }

        let mut jpeg_data = vec![];
        let mut jpeg_encoder = image::jpeg::JPEGEncoder::new(&mut jpeg_data);
        if let Err(e) =
            jpeg_encoder.encode(&img.raw_pixels(), img.width(), img.height(), img.color())
        {
            return Box::new(futures::future::err(VerifierError::JpegEncodeError(e)));
        }
        let base64_image = base64::encode(&jpeg_data);

        let mut params = {
            let mut map = BTreeMap::new();
            map.insert("app_id", self.app_id.to_string());
            map.insert("time_stamp", chrono::Utc::now().timestamp().to_string());
            map.insert(
                "nonce_str",
                (0..30)
                    .map(|_| ('a' as u8 + (random::<f32>() * 26.0) as u8) as char)
                    .collect(),
            );
            map.insert(
                "image",
                //To URL encoding
                base64_image
                    .replace("=", "%3D")
                    .replace("+", "%2B")
                    .replace("/", "%2F"),
            );
            map
        };

        let md5_hash = self.get_sign_hash(&params);
        debug!("hashed: {}", md5_hash);
        let form = RequestForm {
            app_id: params.remove("app_id").unwrap(),
            time_stamp: params.remove("time_stamp").unwrap(),
            nonce_str: params.remove("nonce_str").unwrap(),
            image: base64_image,
            sign: md5_hash,
        };

        let sid = match student_id {
            Some(id) => Some(id.to_owned()),
            None => None,
        };
        let institute = institute.to_owned();

        let ret = Self::api_request(&form)
            .map_err(|err| err)
            .and_then(move |api_response| {
                debug!("response: {}", api_response);

                let ocr_result: ResponseParams = match serde_json::from_str(&api_response) {
                    Ok(r) => r,
                    Err(e) => {
                        debug!("failed to parse json: {}", e);
                        return Err(VerifierError::ApiServerError(
                            "Failed to parse API server response.".to_string(),
                        ));
                    }
                };

                if ocr_result.ret != 0 {
                    return Err(VerifierError::ServerResponseError(ocr_result.msg));
                }

                let mut institute_match = false;
                let mut id_match = sid.is_none();
                for item in ocr_result.data.item_list {
                    if item.itemstring == institute {
                        institute_match = true;
                    }
                    if sid.as_ref().is_some() && &item.itemstring == sid.as_ref().unwrap() {
                        id_match = true;
                    }
                }

                if !institute_match {
                    Err(VerifierError::InstituteNotMatch)
                } else if !id_match {
                    Err(VerifierError::StudentIdNotMatch)
                } else {
                    Ok(())
                }
            });
        Box::new(ret)
    }

    fn api_request(form: &RequestForm) -> Box<Future<Item = String, Error = VerifierError>> {
        let mut client_builder = Client::build();
        //        client_builder = client_builder.timeout(Duration::from_secs(20));
        client_builder = client_builder.disable_timeout();
        let client = client_builder.finish();

        let ret = client
            .post(OCR_URL)
            .set_header("Content-Type", "application/x-www-form-urlencoded")
            .send_form(form)
            .map_err(|error| {
                warn!("Error {:?} when requesting api", error);
                VerifierError::ApiServerConnectionError(error)
            })
            .and_then(|mut response| {
                debug!("Response header: {:?}", response);
                use actix_web::http::StatusCode;
                match response.status() {
                    StatusCode::OK => match response.body().wait() {
                        Ok(item) => Ok(String::from_utf8_lossy(&item[..]).into_owned()),
                        Err(e) => Err(VerifierError::ServerResponseError(e.to_string())),
                    },
                    _ => Err(VerifierError::ApiServerError(format!(
                        "Server response code {}",
                        response.status()
                    ))),
                }
            });
        Box::new(ret)
    }
}
