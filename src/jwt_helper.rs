use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rsa::{
    pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey},
    RsaPrivateKey, RsaPublicKey,
};
use serde::{Deserialize, Serialize};

const ALGORITHM: Algorithm = Algorithm::RS256;
const SECRET_ABSENT: &'_ str = "`pvt_key_secret` is required for generating token.";
const PVT_KEY_ABSENT: &'_ str = "`encrypted_pvt_key` is required for generating token.";
const PUB_KEY_ABSENT: &'_ str = "`pub_key` is required for verifying token.";

/// This helper uses `RS256` algorithm.
/// For instantiation example, see [`tests::generate_and_verify_token`].
#[derive(buildstructor::Builder, Debug)]
pub struct JWTHelper {
    // Pass-phrase that private key has been encrypted with.
    pvt_key_secret: Option<String>,
    expiry_secs: usize,
    leeway: u64,
    encrypted_pvt_key: Option<String>,
    pub_key: Option<RsaPublicKey>,
}

impl JWTHelper {
    /// Pass in the [`subject`] to identify who the token is issued to, e.g. user_id in DB.
    /// If successful, returns signed jwt token which expires according to config set while [`JWTHelper`] instantiation.
    pub fn generate_token(&self, subject: String) -> Result<String, anyhow::Error> {
        let header = Header::new(ALGORITHM);

        let claims = Claims {
            exp: (Utc::now().timestamp() + Duration::seconds(self.expiry_secs as i64).num_seconds()) as usize,
            iat: Utc::now().timestamp() as usize,
            sub: subject,
        };

        let decrypted_key = <RsaPrivateKey as DecodePrivateKey>::from_pkcs8_encrypted_pem(
            self.encrypted_pvt_key.as_ref().expect(PVT_KEY_ABSENT).as_ref(),
            <std::string::String as AsRef<[u8]>>::as_ref(self.pvt_key_secret.as_ref().expect(SECRET_ABSENT)),
        )?;
        let key = &EncodingKey::from_rsa_pem(decrypted_key.to_pkcs8_pem(Default::default())?.as_bytes())?;

        Ok(encode(&header, &claims, &key)?)
    }

    pub fn validate_token(&self, token: &str) -> Result<String, anyhow::Error> {
        let mut validation = Validation::new(ALGORITHM);
        validation.validate_exp = true;
        validation.leeway = self.leeway;
        let data = decode::<Claims>(
            &token,
            &DecodingKey::from_rsa_pem(
                self.pub_key
                    .as_ref()
                    .expect(PUB_KEY_ABSENT)
                    .to_public_key_pem(Default::default())?
                    .as_bytes(),
            )?,
            &validation,
        )?;

        Ok(data.claims.sub)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    exp: usize,
    iat: usize,
    sub: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs8::DecodePublicKey;
    use std::process::Command;
    use std::thread;
    use std::time::Duration;
    use std::{fs, path::Path};

    const TEST_KEYS_DIR: &'_ str = "./test_keys";
    const PVT_KEY_PATH: &'_ str = "./test_keys/rsa";
    const PUB_KEY_PATH: &'_ str = "./test_keys/rsa.pub";
    const PVT_KEY_SECRET: &'_ str = "testpassword";

    #[test]
    fn generate_and_verify_token() -> Result<(), anyhow::Error> {
        // Create "test_keys" directory if it doesn't exist.
        fs::create_dir_all(TEST_KEYS_DIR)?;
        let mut cli_arg_pass = "pass:".to_string();
        cli_arg_pass.push_str(PVT_KEY_SECRET);

        // if rsa256 private key doesn't exist, generate it using openssl.
        // Reason for using openssl, is that in deployment setup we probably would be using
        // openssl rather than rust code.
        if !Path::new(PVT_KEY_PATH).exists() {
            let pvt_key_generated = Command::new("openssl")
                .arg("genrsa")
                .arg("-aes256")
                .arg("-passout")
                .arg(&cli_arg_pass)
                .arg("-out")
                .arg(PVT_KEY_PATH)
                .arg("4096")
                .spawn()?
                .wait()?
                .success();
            assert!(pvt_key_generated);
        }

        // if rsa256 public key doesn't exist, generate it using private key file.
        if !Path::new(PUB_KEY_PATH).exists() {
            let pub_key_generated = Command::new("openssl")
                .arg("rsa")
                .arg("-in")
                .arg(PVT_KEY_PATH)
                .arg("-passin")
                .arg(&cli_arg_pass)
                .arg("-pubout")
                .arg("-out")
                .arg(PUB_KEY_PATH)
                .spawn()?
                .wait()?
                .success();
            assert!(pub_key_generated);
        }

        // Read private key and public key generated by openssl from their respective files.
        let encrypted_pvt_key = fs::read_to_string(PVT_KEY_PATH)?;
        let pub_key = fs::read_to_string(PUB_KEY_PATH)?;

        let jwt_helper: JWTHelper = JWTHelper::builder()
            .pvt_key_secret("testpassword")
            .expiry_secs(2)
            .leeway(0)
            .encrypted_pvt_key(encrypted_pvt_key)
            .pub_key(RsaPublicKey::from_public_key_pem(&pub_key)?)
            .build();

        let user_id = "user_123";
        let signed_token = jwt_helper.generate_token(user_id.to_string())?;

        let decoded_user_id = jwt_helper.validate_token(&signed_token)?;

        assert_eq!(user_id, decoded_user_id);

        // Sleep for longer than this token is valid for, and then try validating token, it should fail.
        thread::sleep(Duration::from_secs(3));
        assert!(jwt_helper.validate_token(&signed_token).is_err());

        Ok(())
    }
}
